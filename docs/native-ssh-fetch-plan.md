# Plan: Native (pure-Rust) SSH fetching for cljrs-vcs

## Context

`cljrs-vcs` is now pure-Rust over gitoxide (`gix`) for https/local/file remotes,
but `ssh://` and scp-like `git@host:path` remotes are rejected
(`classify_remote` → `Unsupported`, `VcsError::UnsupportedRemote`). gitoxide has
no pure-Rust SSH client — its built-in ssh transport spawns the system `ssh`
binary. This phase adds a native SSH transport using `russh` so ssh remotes can
be fetched without any external process, completing the "no external tooling"
goal.

### Decisions (confirmed with user)
- **Host-key verification:** verify the server key against `~/.ssh/known_hosts`
  (reject unknown/mismatched), via `russh_keys::check_known_hosts`.
- **Authentication:** ssh-agent only (via `$SSH_AUTH_SOCK`); no private-key-file
  loading, so there is no passphrase handling.
- **Opt-in:** gated behind an off-by-default `ssh` cargo feature so the russh +
  tokio async stack is not forced on users who only need https/local.

## Why this is a bridge, not a rewrite

gitoxide exposes the seam we need:

- **`gix::Remote::to_connection_with_transport<T: Transport>(t)`**
  (`gix-0.84/src/remote/connect.rs:61`) hands gix a pre-built transport and then
  runs its **normal** `ref_map` + `prepare_fetch` + `receive` fetch logic — no
  reimplementation of negotiation/packfile handling.
- The transport itself can be gitoxide's generic
  **`gix_transport::client::git::Connection<R, W>`** (`ConnectMode::Process`)
  over any `R: Read + W: Write`. This is exactly how the spawn-`ssh` path works:
  it runs `git-upload-pack '<path>'` on the remote and speaks pack-protocol over
  the process's stdio.

So we only have to produce a blocking `Read`/`Write` pair backed by an SSH exec
channel; gitoxide does the rest.

### Data flow
```
ssh:// URL
  ──► russh: TCP connect + SSH handshake
  ──► Handler::check_server_key → russh_keys::check_known_hosts (reject if absent/mismatch)
  ──► authenticate via ssh-agent (russh_keys::agent::AgentClient::connect_env + request_identities)
  ──► channel_open_session().exec(true, "git-upload-pack '<path>'")
  ──► channel.into_stream()                  // ChannelStream: AsyncRead + AsyncWrite (verified)
  ──► tokio_util::io::SyncIoBridge           // async → blocking Read/Write
  ──► gix_transport::client::git::Connection::new(r, w, ConnectMode::Process, path, version, None, trace)
  ──► repo.remote_at(url).to_connection_with_transport(conn) → ref_map() + prepare_fetch() + receive()
```

### Verified API facts
- russh 0.61: `client::connect`, `Handler::check_server_key(&mut self, &ssh_key::PublicKey)`,
  `Handle::authenticate_publickey_with(user, signer)`, `channel_open_session`,
  `Channel::exec(want_reply, cmd)`, `Channel::into_stream() -> ChannelStream`
  (AsyncRead+AsyncWrite). russh reuses the **same `ssh-key` crate** cljrs-vcs
  already depends on for signatures.
- russh-keys 0.50: `check_known_hosts` / `check_known_hosts_path`;
  `agent::AgentClient::connect_env()` + `request_identities()`, and the agent
  client is usable as the `Signer` for `authenticate_publickey_with`.

## Design

### 1. Dependencies / feature gate (`Cargo.toml` + `cljrs-vcs/Cargo.toml`)
Add to `[workspace.dependencies]`, pinned: `russh = "0.61"`,
`russh-keys = "0.50"` (beta), `tokio-util = { version = "0.7", features = ["io"] }`,
and reuse the workspace `tokio`. In `cljrs-vcs`:
```toml
[features]
ssh = ["dep:russh", "dep:russh-keys", "dep:tokio-util", "dep:tokio"]
```
The `ssh` feature is **not** in the default set. `cljrs` (the CLI) enables it.

### 2. New module `crates/cljrs-vcs/src/ssh.rs` (cfg-gated)
- `struct KnownHostsVerifier` implementing `russh::client::Handler` whose
  `check_server_key` calls `russh_keys::check_known_hosts(host, port, key)` and
  returns `Ok(false)` (→ rejected) on unknown/mismatched keys.
- `fn open_upload_pack(host, port, user, path) -> VcsResult<ChannelStream>`:
  build a tokio runtime handle, `client::connect`, run host-key check,
  authenticate against every agent identity until one succeeds (clear error if
  the agent is absent or no identity is accepted), open a session channel, and
  `exec("git-upload-pack '<path>'")`. Returns the channel stream.
- `fn ssh_transport(url: &gix::Url) -> VcsResult<git::Connection<impl Read, impl Write>>`:
  parse user/host/port/path from the `gix::Url`, open the channel, wrap it with
  `SyncIoBridge` on a dedicated current-thread runtime, and build the
  `git::Connection` in `ConnectMode::Process`.

### 3. Wire into `fetch_remote` (`crates/cljrs-vcs/src/lib.rs`)
- Extend `classify_remote` with an `Ssh` kind. With the `ssh` feature **off**,
  `Ssh` still maps to `UnsupportedRemote` (current behavior, clear error). With
  it **on**, `fetch_remote` routes ssh URLs through the native transport.
- ssh clone path: `prepare_clone_bare` can't be used (it picks its own
  transport), so for a fresh ssh repo do `gix::init_bare(repo_dir)`, create an
  anonymous remote at the URL, then
  `remote.to_connection_with_transport(ssh_transport(url)?)`, `ref_map`, and
  fetch with a default refspec — enough to make `sha` present (which is all
  `fetch_remote` then checks via `rev_parse_single`).
- ssh fetch-existing path: open the cached repo and fetch over the same custom
  transport instead of `remote.connect()`.
- https/local/file paths are unchanged.

### 4. CLI
Enable `cljrs-vcs/ssh` from `crates/cljrs/Cargo.toml` so `cljrs run`/`deps fetch`
get ssh support by default in the binary, while the library stays lean.

## Complexity / risks
- **Async↔sync bridge** is the main subtlety. Keep gix in blocking mode (as
  today) and drive russh on a dedicated tokio runtime, exposing the channel as
  blocking I/O via `SyncIoBridge`. Going async-all-the-way would force gix's
  whole network layer into async mode — far larger blast radius; rejected.
- **`git-upload-pack` path quoting**: the remote path must be shell-quoted in
  the exec command (git does this); cover scp-like and `ssh://…/~user/repo`
  forms.
- **Auth UX**: agent-only means a clear, actionable error when `$SSH_AUTH_SOCK`
  is unset or holds no accepted key. Document that key-file/passphrase auth is
  out of scope for this phase.
- **russh-keys is pre-1.0 (beta)**: pin exactly; isolate its use to `ssh.rs`.
- **Dependency weight / build time**: russh pulls a sizeable async/crypto tree;
  the feature gate keeps it off by default.

## Critical files
- `Cargo.toml` (workspace deps), `crates/cljrs-vcs/Cargo.toml` (feature)
- `crates/cljrs-vcs/src/ssh.rs` (new, cfg-gated)
- `crates/cljrs-vcs/src/lib.rs` (`classify_remote` + ssh routing in `fetch_remote`)
- `crates/cljrs/Cargo.toml` (enable the feature in the binary)
- `crates/cljrs-vcs/README.md`, `VERSIONING.md`, `docs/book/src/cli/deps.md`

## Verification
1. `cargo build -p cljrs-vcs` (no ssh) and `--features ssh` — both clean; clippy
   clean.
2. Unit tests for `classify_remote` (ssh kind) and the path/exec-quoting helper.
3. Integration test (gated, like the dylib e2e): start an in-process SSH server
   with `russh` (server side) backed by a temp git repo, advertise
   `git-upload-pack`, and assert `fetch_remote("ssh://…", sha)` populates the
   cache and `rev_parse_single(sha)` succeeds; assert an unknown host key is
   rejected. Gate behind an env var since it needs an agent/server fixture.
4. Manual: with `$SSH_AUTH_SOCK` set and a real `git@github.com:…` dep, confirm
   fetch works with **no `ssh`/`git` binary** on `PATH`, and that a host missing
   from `known_hosts` is rejected.

## Out of scope (future)
- Private-key-file auth and passphrase prompting.
- SSH config (`~/.ssh/config`) host aliases/IdentityFile resolution.
- Pure-Rust SSH for *pushing* (only fetch/upload-pack is needed here).
