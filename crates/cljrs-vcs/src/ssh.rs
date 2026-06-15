//! Native (pure-Rust) SSH transport for `ssh://` and scp-like git remotes.
//!
//! Instead of spawning the system `ssh` binary (gitoxide's default), this module
//! opens an SSH connection with [`russh`], runs `git-upload-pack` on the remote,
//! and feeds the resulting byte stream into gitoxide's generic
//! [`git::Connection`](gix::protocol::transport::client::git::Connection) so the
//! normal fetch logic drives the transfer.
//!
//! * **Host keys** are verified against the user's `~/.ssh/known_hosts`.
//! * **Authentication** is via a running ssh-agent (`$SSH_AUTH_SOCK`).
//!
//! russh is async; gitoxide runs in blocking mode here. The SSH connection runs
//! on a dedicated multi-threaded Tokio runtime, and the exec channel is exposed
//! to gitoxide as blocking `Read`/`Write` via [`tokio_util::io::SyncIoBridge`].

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use russh::client;
use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::AgentClient;
use russh::keys::ssh_key;

use crate::{VcsError, VcsResult};

/// Fetch `url` into the bare cache repository at `repo_dir`, creating it if it
/// does not yet exist. After this returns, the remote's reachable objects are
/// present in `repo_dir`.
pub(crate) fn fetch_into_cache(url: &str, repo_dir: &Path) -> VcsResult<()> {
    let repo = if repo_dir.exists() {
        gix::open(repo_dir).map_err(|e| VcsError::Git(e.to_string()))?
    } else {
        std::fs::create_dir_all(repo_dir).map_err(VcsError::Io)?;
        gix::init_bare(repo_dir).map_err(|e| VcsError::Git(e.to_string()))?
    };
    fetch_over_ssh(&repo, url)
}

/// Drive a single fetch of every branch from `url` into `repo` over a native
/// SSH connection.
fn fetch_over_ssh(repo: &gix::Repository, url: &str) -> VcsResult<()> {
    use gix::protocol::transport::Protocol;
    use gix::protocol::transport::client::git;

    let parsed =
        gix::url::parse(url.into()).map_err(|e| VcsError::Git(format!("invalid ssh url: {e}")))?;
    let host = parsed
        .host()
        .ok_or_else(|| VcsError::Git(format!("ssh url has no host: {url:?}")))?
        .to_string();
    let port = parsed.port.unwrap_or(22);
    let user = parsed.user().unwrap_or("git").to_string();
    let path = parsed.path.to_string();
    let command = format!("git-upload-pack {}", shell_quote(&path));

    // Dedicated runtime that drives the SSH socket while gitoxide blocks on it.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .map_err(VcsError::Io)?;
    let (ssh_handle, stream) = rt.block_on(open_upload_pack(&host, port, &user, command))?;

    let (read_half, write_half) = tokio::io::split(stream);
    let reader = tokio_util::io::SyncIoBridge::new_with_handle(read_half, rt.handle().clone());
    let writer = tokio_util::io::SyncIoBridge::new_with_handle(write_half, rt.handle().clone());

    let transport = git::blocking_io::Connection::new(
        reader,
        writer,
        Protocol::V2,
        parsed.path.clone(),
        // ssh runs git-upload-pack directly: no daemon virtual-host handshake.
        None::<(String, Option<u16>)>,
        git::ConnectMode::Process,
        false,
    );

    let remote = repo
        .remote_at(url)
        .map_err(|e| VcsError::Git(format!("remote_at: {e}")))?
        .with_refspecs(
            Some("+refs/heads/*:refs/heads/*"),
            gix::remote::Direction::Fetch,
        )
        .map_err(|e| VcsError::Git(format!("refspec: {e}")))?;

    let outcome = remote
        .to_connection_with_transport(transport)
        .prepare_fetch(gix::progress::Discard, Default::default())
        .map_err(|e| VcsError::Git(format!("prepare fetch: {e}")))?
        .receive(gix::progress::Discard, &AtomicBool::new(false))
        .map_err(|e| VcsError::Git(format!("fetch: {e}")))?;

    // Keep the SSH session and runtime alive until the fetch has completed.
    let _ = outcome;
    drop(ssh_handle);
    drop(rt);
    Ok(())
}

/// Connect, authenticate via ssh-agent, and start `git-upload-pack` on the
/// remote, returning the live handle (kept alive by the caller) and the exec
/// channel's byte stream.
async fn open_upload_pack(
    host: &str,
    port: u16,
    user: &str,
    command: String,
) -> VcsResult<(
    client::Handle<KnownHostsHandler>,
    russh::ChannelStream<client::Msg>,
)> {
    let config = Arc::new(client::Config::default());
    let handler = KnownHostsHandler {
        host: host.to_string(),
        port,
    };
    let mut handle = client::connect(config, (host, port), handler)
        .await
        .map_err(|e| VcsError::Git(format!("ssh connect to {host}:{port}: {e}")))?;

    authenticate_with_agent(&mut handle, user, host).await?;

    let channel = handle
        .channel_open_session()
        .await
        .map_err(|e| VcsError::Git(format!("ssh open session: {e}")))?;
    channel
        .exec(true, command)
        .await
        .map_err(|e| VcsError::Git(format!("ssh exec git-upload-pack: {e}")))?;
    Ok((handle, channel.into_stream()))
}

/// Authenticate `handle` as `user` by trying each ssh-agent identity in turn.
async fn authenticate_with_agent(
    handle: &mut client::Handle<KnownHostsHandler>,
    user: &str,
    host: &str,
) -> VcsResult<()> {
    let mut agent = AgentClient::connect_env().await.map_err(|e| {
        VcsError::Git(format!(
            "could not connect to ssh-agent ({e}); is SSH_AUTH_SOCK set?"
        ))
    })?;
    let identities = agent
        .request_identities()
        .await
        .map_err(|e| VcsError::Git(format!("ssh-agent identities: {e}")))?;
    if identities.is_empty() {
        return Err(VcsError::Git(
            "ssh-agent has no identities loaded".to_string(),
        ));
    }

    for identity in identities {
        // Only plain public-key identities are used; certificate identities are
        // skipped (out of scope for this phase).
        let AgentIdentity::PublicKey { key, .. } = identity else {
            continue;
        };
        if let Ok(result) = handle
            .authenticate_publickey_with(user, key, None, &mut agent)
            .await
            && result.success()
        {
            return Ok(());
        }
    }
    Err(VcsError::Git(format!(
        "ssh authentication failed for {user}@{host}: no ssh-agent key was accepted"
    )))
}

/// A russh handler that accepts a server host key only when it is present in,
/// and matches, the user's `~/.ssh/known_hosts`.
struct KnownHostsHandler {
    host: String,
    port: u16,
}

impl client::Handler for KnownHostsHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        // Accept only an exact known_hosts match; unknown host or changed key
        // is rejected.
        Ok(matches!(
            russh::keys::check_known_hosts(&self.host, self.port, server_public_key),
            Ok(true)
        ))
    }
}

/// Single-quote `s` for safe use as an argument in the remote shell command,
/// escaping any embedded single quotes (`git`'s own quoting scheme).
fn shell_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::shell_quote;

    #[test]
    fn quotes_plain_and_special_paths() {
        assert_eq!(shell_quote("user/repo.git"), "'user/repo.git'");
        assert_eq!(shell_quote("/srv/git/x"), "'/srv/git/x'");
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }
}
