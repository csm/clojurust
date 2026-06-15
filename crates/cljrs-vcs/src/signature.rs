//! Native commit-signature verification.
//!
//! Git commits can be signed in two formats, both stored in the commit
//! object's `gpgsig` header:
//!
//! * **PGP** — `-----BEGIN PGP SIGNATURE-----`, verified here with rPGP.
//! * **SSH** — `-----BEGIN SSH SIGNATURE-----` (the `SSHSIG` format produced by
//!   `ssh-keygen -Y sign -n git`), verified with `ssh-key`.
//!
//! Trust is *cljrs-managed*: a signature is accepted only when it is
//! cryptographically valid **and** made by a key present in the caller-supplied
//! [`TrustedKeys`]. There is no implicit fallback to the user's GPG keyring or
//! SSH `allowed_signers` file.

use pgp::composed::{Deserializable, DetachedSignature, SignedPublicKey};
use thiserror::Error;

/// The SSHSIG namespace git uses for commit/tag signatures.
const SSH_NAMESPACE: &str = "git";

/// Errors raised while loading a trusted public key.
#[derive(Debug, Error)]
pub enum TrustedKeyError {
    #[error("invalid PGP public key: {0}")]
    Pgp(String),
    #[error("invalid SSH public key: {0}")]
    Ssh(String),
    #[error("unrecognized key format (expected an armored PGP key or an OpenSSH public key)")]
    Unrecognized,
}

/// A cljrs-managed set of public keys trusted to sign commits.
///
/// Populate it from `cljrs.edn`'s `:trusted-signers` entries (see
/// `cljrs-deps`), then pass it to [`crate::verify_commit_signature`].
#[derive(Default)]
pub struct TrustedKeys {
    pgp: Vec<SignedPublicKey>,
    ssh: Vec<ssh_key::PublicKey>,
}

impl TrustedKeys {
    /// An empty trust set. Any signature check against it fails.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` when no keys are configured.
    pub fn is_empty(&self) -> bool {
        self.pgp.is_empty() && self.ssh.is_empty()
    }

    /// Add a trusted key from text, auto-detecting PGP vs OpenSSH format.
    ///
    /// * Armored PGP public-key blocks (`-----BEGIN PGP PUBLIC KEY BLOCK-----`).
    /// * OpenSSH public keys (`ssh-ed25519 AAAA… comment`, `ecdsa-…`, `rsa-…`).
    pub fn add_key_text(&mut self, text: &str) -> Result<(), TrustedKeyError> {
        let trimmed = text.trim_start();
        if trimmed.starts_with("-----BEGIN PGP") {
            self.add_pgp_armored(text)
        } else if is_openssh_public_key(trimmed) {
            self.add_ssh_openssh(text)
        } else {
            Err(TrustedKeyError::Unrecognized)
        }
    }

    /// Add a trusted PGP public key from an armored block.
    pub fn add_pgp_armored(&mut self, armored: &str) -> Result<(), TrustedKeyError> {
        let (key, _headers) = SignedPublicKey::from_armor_single(armored.as_bytes())
            .map_err(|e| TrustedKeyError::Pgp(e.to_string()))?;
        self.pgp.push(key);
        Ok(())
    }

    /// Add a trusted SSH public key in OpenSSH `authorized_keys` format.
    pub fn add_ssh_openssh(&mut self, openssh: &str) -> Result<(), TrustedKeyError> {
        let key = ssh_key::PublicKey::from_openssh(openssh.trim())
            .map_err(|e| TrustedKeyError::Ssh(e.to_string()))?;
        self.ssh.push(key);
        Ok(())
    }
}

/// Returns `true` when `line` looks like an OpenSSH public key.
fn is_openssh_public_key(line: &str) -> bool {
    matches!(
        line.split_whitespace().next(),
        Some(
            "ssh-ed25519"
                | "ssh-rsa"
                | "ssh-dss"
                | "ecdsa-sha2-nistp256"
                | "ecdsa-sha2-nistp384"
                | "ecdsa-sha2-nistp521"
                | "sk-ssh-ed25519@openssh.com"
                | "sk-ecdsa-sha2-nistp256@openssh.com"
        )
    )
}

/// Verify the signature embedded in a raw commit object (`object.data`, i.e. the
/// decoded commit text starting with `tree …`, without the `commit <size>\0`
/// git object header) against `trusted`.
///
/// Returns `Ok(())` on a valid, trusted signature; `Err(reason)` otherwise.
pub(crate) fn verify_commit_object(raw: &[u8], trusted: &TrustedKeys) -> Result<(), String> {
    let (payload, sig) =
        split_commit_signature(raw).ok_or_else(|| "commit is not signed".to_string())?;
    let sig_str =
        std::str::from_utf8(&sig).map_err(|_| "signature is not valid UTF-8".to_string())?;
    let banner = sig_str.trim_start();

    if banner.starts_with("-----BEGIN PGP SIGNATURE-----") {
        verify_pgp(&payload, sig_str, trusted)
    } else if banner.starts_with("-----BEGIN SSH SIGNATURE-----") {
        verify_ssh(&payload, sig_str, trusted)
    } else {
        Err("unrecognized signature format".to_string())
    }
}

/// Verify a PGP-signed commit payload against the trusted PGP keys.
fn verify_pgp(payload: &[u8], armored_sig: &str, trusted: &TrustedKeys) -> Result<(), String> {
    let (sig, _headers) = DetachedSignature::from_armor_single(armored_sig.as_bytes())
        .map_err(|e| format!("malformed PGP signature: {e}"))?;

    if trusted.pgp.is_empty() {
        return Err("no trusted PGP keys configured".to_string());
    }

    for key in &trusted.pgp {
        // Try the primary key, then any signing subkeys.
        if sig.verify(key, payload).is_ok() {
            return Ok(());
        }
        for subkey in &key.public_subkeys {
            if sig.verify(subkey, payload).is_ok() {
                return Ok(());
            }
        }
    }
    Err("signature is not valid for any trusted PGP key".to_string())
}

/// Verify an SSH-signed commit payload against the trusted SSH keys.
fn verify_ssh(payload: &[u8], pem_sig: &str, trusted: &TrustedKeys) -> Result<(), String> {
    let sshsig =
        ssh_key::SshSig::from_pem(pem_sig).map_err(|e| format!("malformed SSH signature: {e}"))?;

    if trusted.ssh.is_empty() {
        return Err("no trusted SSH keys configured".to_string());
    }

    for key in &trusted.ssh {
        // `verify` enforces that the signing key equals this trusted key, the
        // namespace matches, and the signature is cryptographically valid.
        if key.verify(SSH_NAMESPACE, payload, &sshsig).is_ok() {
            return Ok(());
        }
    }
    Err("signature is not valid for any trusted SSH key".to_string())
}

/// Split a raw commit object into `(signed_payload, armored_signature)`.
///
/// The signed payload is the commit object with its `gpgsig` header removed —
/// exactly the bytes git signs. The signature is reconstructed from the
/// `gpgsig` header line and its space-prefixed continuation lines. Returns
/// `None` when there is no `gpgsig` header.
fn split_commit_signature(raw: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    const HEADER: &[u8] = b"gpgsig ";
    let mut payload = Vec::with_capacity(raw.len());
    let mut sig = Vec::new();
    let mut found = false;
    let mut i = 0;

    while i < raw.len() {
        let (line, next) = read_line(raw, i);

        // A blank line terminates the header section; copy the message verbatim.
        if line.is_empty() {
            payload.extend_from_slice(&raw[i..]);
            return if found { Some((payload, sig)) } else { None };
        }

        if !found && line.starts_with(HEADER) {
            found = true;
            sig.extend_from_slice(&line[HEADER.len()..]);
            i = next;
            // Consume space-prefixed continuation lines, stripping the leading space.
            while i < raw.len() {
                let (cont, cnext) = read_line(raw, i);
                if cont.first() == Some(&b' ') {
                    sig.push(b'\n');
                    sig.extend_from_slice(&cont[1..]);
                    i = cnext;
                } else {
                    break;
                }
            }
            continue;
        }

        // Ordinary header line: keep it in the signed payload.
        payload.extend_from_slice(line);
        payload.push(b'\n');
        i = next;
    }

    if found { Some((payload, sig)) } else { None }
}

/// Return the line starting at `start` (without its trailing `\n`) and the index
/// just past the line terminator.
fn read_line(raw: &[u8], start: usize) -> (&[u8], usize) {
    match raw[start..].iter().position(|&b| b == b'\n') {
        Some(p) => (&raw[start..start + p], start + p + 1),
        None => (&raw[start..], raw.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A static Ed25519 keypair used to exercise the SSH verification path
    // without generating keys (which would need an RNG feature). Generated once
    // with `ssh-key`; used only in tests.
    const TEST_SSH_PRIVATE: &str = "\
-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACBmFUP3SDH5k28ErT2na8g4asrcsI4STLcmDImAF0WjDwAAAIiFW+7uhVvu
7gAAAAtzc2gtZWQyNTUxOQAAACBmFUP3SDH5k28ErT2na8g4asrcsI4STLcmDImAF0WjDw
AAAEAgsZE1vrnYoatnjRDx6BGE9PeOViG9mgDVkCbPj8unnmYVQ/dIMfmTbwStPadryDhq
ytywjhJMtyYMiYAXRaMPAAAAAAECAwQF
-----END OPENSSH PRIVATE KEY-----
";
    const TEST_SSH_PUBLIC: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGYVQ/dIMfmTbwStPadryDhqytywjhJMtyYMiYAXRaMP cljrs-test";

    /// Build a raw commit object (`object.data` form) carrying `armored_sig` in
    /// its `gpgsig` header. `payload` must be the no-signature commit text.
    fn assemble_signed_commit(payload: &[u8], armored_sig: &str) -> Vec<u8> {
        // Headers end at the first blank line; insert gpgsig as the last header.
        let split = payload
            .windows(2)
            .position(|w| w == b"\n\n")
            .expect("payload has a header/message separator");
        let headers = &payload[..=split]; // includes the trailing header newline
        let message = &payload[split + 1..]; // includes the leading blank line

        let mut out = Vec::new();
        out.extend_from_slice(headers);
        out.extend_from_slice(b"gpgsig ");
        for (i, line) in armored_sig.lines().enumerate() {
            if i > 0 {
                out.push(b'\n');
                out.push(b' ');
            }
            out.extend_from_slice(line.as_bytes());
        }
        out.push(b'\n');
        out.extend_from_slice(message);
        out
    }

    fn sign_payload_ssh(payload: &[u8]) -> String {
        let key = ssh_key::PrivateKey::from_openssh(TEST_SSH_PRIVATE).expect("parse private key");
        let sig = ssh_key::SshSig::sign(&key, SSH_NAMESPACE, ssh_key::HashAlg::Sha512, payload)
            .expect("sign");
        sig.to_pem(ssh_key::LineEnding::LF).expect("pem")
    }

    const PAYLOAD: &[u8] = b"tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\n\
        author Test <t@example.com> 0 +0000\n\
        committer Test <t@example.com> 0 +0000\n\
        \n\
        signed commit\n";

    #[test]
    fn ssh_signed_commit_verifies_with_trusted_key() {
        let pem = sign_payload_ssh(PAYLOAD);
        let raw = assemble_signed_commit(PAYLOAD, &pem);

        let mut trusted = TrustedKeys::new();
        trusted.add_ssh_openssh(TEST_SSH_PUBLIC).unwrap();
        assert!(verify_commit_object(&raw, &trusted).is_ok());
    }

    #[test]
    fn ssh_signed_commit_fails_with_empty_trust() {
        let pem = sign_payload_ssh(PAYLOAD);
        let raw = assemble_signed_commit(PAYLOAD, &pem);
        let err = verify_commit_object(&raw, &TrustedKeys::new()).unwrap_err();
        assert!(err.contains("no trusted SSH keys"), "got: {err}");
    }

    #[test]
    fn ssh_signed_commit_fails_with_untrusted_key() {
        let pem = sign_payload_ssh(PAYLOAD);
        // Tamper with the payload so the signature no longer matches.
        let mut tampered = PAYLOAD.to_vec();
        tampered.extend_from_slice(b"extra\n");
        let raw = assemble_signed_commit(&tampered, &pem);

        let mut trusted = TrustedKeys::new();
        trusted.add_ssh_openssh(TEST_SSH_PUBLIC).unwrap();
        assert!(verify_commit_object(&raw, &trusted).is_err());
    }

    #[test]
    fn add_key_text_autodetects_ssh() {
        let mut trusted = TrustedKeys::new();
        trusted.add_key_text(TEST_SSH_PUBLIC).expect("ssh key");
        assert!(!trusted.is_empty());
    }

    #[test]
    fn unsigned_commit_has_no_signature() {
        let raw = b"tree 0000000000000000000000000000000000000000\n\
                    author A <a@example.com> 0 +0000\n\
                    committer A <a@example.com> 0 +0000\n\
                    \n\
                    hello\n";
        assert!(split_commit_signature(raw).is_none());
    }

    #[test]
    fn splits_payload_and_signature() {
        let raw = b"tree 0000000000000000000000000000000000000000\n\
                    author A <a@example.com> 0 +0000\n\
                    committer A <a@example.com> 0 +0000\n\
                    gpgsig -----BEGIN SSH SIGNATURE-----\n\
                    \x20line1\n\
                    \x20line2\n\
                    \x20-----END SSH SIGNATURE-----\n\
                    \n\
                    subject\n";
        let (payload, sig) = split_commit_signature(raw).expect("signed");
        // Payload must not contain the gpgsig header.
        assert!(!payload.windows(6).any(|w| w == b"gpgsig"));
        // Payload must retain the message and the other headers.
        assert!(payload.ends_with(b"\nsubject\n"));
        assert!(payload.starts_with(b"tree "));
        // Signature is reassembled with continuation lines de-indented.
        let sig = String::from_utf8(sig).unwrap();
        assert_eq!(
            sig,
            "-----BEGIN SSH SIGNATURE-----\nline1\nline2\n-----END SSH SIGNATURE-----"
        );
    }
}
