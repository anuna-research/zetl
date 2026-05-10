//! SPEC-040 mobile process-wide state (REQ-4003).
//!
//! Houses types that are shared between the Tauri Mobile shell crate
//! and the embedded `zetl serve` running in the same process. Lives
//! in the parent crate (rather than the mobile crate) so the
//! `/_mobile/*` route handlers in `web::mobile` can import them
//! without a circular dependency back into `zetl-mobile`.
//!
//! v0.1-strawman scope: an in-memory `KeyStore` for the SSH key
//! derived from the user's BIP39 seed. Persistent storage in the
//! platform secure element ([[iOS Keychain]] / [[Android Keystore]])
//! is the next slice; until that ships, the user re-enters the seed
//! each launch.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result};
use ed25519_dalek::SigningKey;

/// Process-lifetime SSH key store. Cheap to clone; clones share the
/// same inner `Mutex<Option<StoredKey>>` so updates from one entry
/// point are visible to all later callers across both Tauri commands
/// and embedded-serve handlers.
#[derive(Clone, Default)]
pub struct KeyStore(Arc<Mutex<Option<StoredKey>>>);

#[derive(Clone)]
struct StoredKey {
    /// `ssh-ed25519 AAAA<base64-blob> zetl-mobile` — the line the user
    /// pastes into their git host's "add SSH key" page.
    pub_openssh: String,
    /// OpenSSH PEM private key, used by the `git2` credential
    /// callback for clone / pull / push.
    priv_pem: String,
}

impl KeyStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Derive the ssh keypair from a 12-word BIP39 mnemonic, retain
    /// it in memory, and return the formatted public-key line for
    /// display to the user.
    pub fn import_mnemonic(&self, mnemonic_phrase: &str) -> Result<String> {
        let signing: SigningKey =
            crate::user::recovery::derive_ssh_key_from_mnemonic(mnemonic_phrase)
                .context("BIP39 → ed25519 derivation failed")?;
        let pub_bytes: [u8; 32] = signing.verifying_key().to_bytes();
        let priv_bytes: [u8; 32] = signing.to_bytes();

        let priv_pem = crate::user::recovery::encode_openssh_ed25519(&priv_bytes, &pub_bytes);
        let pub_openssh = format_ssh_ed25519_pub_line(&pub_bytes, "zetl-mobile");

        let mut guard = self.0.lock().expect("KeyStore mutex poisoned");
        *guard = Some(StoredKey {
            pub_openssh: pub_openssh.clone(),
            priv_pem,
        });
        Ok(pub_openssh)
    }

    /// Public-key OpenSSH line for the currently-imported seed, if
    /// any. Used by the onboarding template to redisplay the line if
    /// the user navigates back without re-entering the seed.
    pub fn pub_openssh(&self) -> Option<String> {
        self.0
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|k| k.pub_openssh.clone()))
    }

    /// Private-key PEM for the credential callback. Held only in
    /// memory; never logged. Returns `None` if no seed has been
    /// imported yet — the git module surfaces this as an actionable
    /// user-facing error.
    pub fn priv_pem(&self) -> Option<String> {
        self.0
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|k| k.priv_pem.clone()))
    }

    /// True if a seed has been imported in this process.
    pub fn is_loaded(&self) -> bool {
        self.0.lock().ok().is_some_and(|g| g.is_some())
    }
}

/// Process-wide singleton. The first caller (typically the Tauri
/// shell's `setup()` hook on app launch) materialises an empty store;
/// subsequent callers (Tauri command handlers, `/_mobile/*` route
/// handlers) share the same instance.
pub fn global() -> &'static KeyStore {
    static INSTANCE: OnceLock<KeyStore> = OnceLock::new();
    INSTANCE.get_or_init(KeyStore::new)
}

/// Process-wide vault root, registered by the Tauri shell at launch
/// before the embedded serve task spawns. Onboarding handlers read
/// this to know where to clone into; capture / save handlers will use
/// it as the working-tree root.
///
/// Stored as a `OnceLock` rather than a `Mutex` because a single
/// device app session uses one vault for its lifetime — switching
/// vaults requires restarting the app.
fn vault_root_cell() -> &'static OnceLock<PathBuf> {
    static CELL: OnceLock<PathBuf> = OnceLock::new();
    &CELL
}

/// Register the vault root for this process. Called by the Tauri
/// shell from its `setup()` hook. Subsequent calls are no-ops — the
/// initial value wins.
pub fn set_vault_root(path: PathBuf) {
    let _ = vault_root_cell().set(path);
}

/// Vault root, if registered. Returns `None` outside of a Tauri shell
/// context (e.g. from in-process unit tests that exercise the
/// `/_mobile/*` handlers without a real shell).
pub fn vault_root() -> Option<PathBuf> {
    vault_root_cell().get().cloned()
}

/// Format an ed25519 public key as the standard `ssh-ed25519 AAAA…
/// <comment>` line accepted by `~/.ssh/authorized_keys` and every
/// git host's "Add SSH key" page.
fn format_ssh_ed25519_pub_line(public_key: &[u8; 32], comment: &str) -> String {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;

    // SSH wire format for the public-key blob: length-prefixed
    // "ssh-ed25519" string followed by the length-prefixed 32-byte key.
    let blob: Vec<u8> = [
        &11u32.to_be_bytes()[..],
        b"ssh-ed25519",
        &32u32.to_be_bytes()[..],
        &public_key[..],
    ]
    .concat();
    let b64 = STANDARD.encode(&blob);
    format!("ssh-ed25519 {b64} {comment}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Same fixture mnemonic as zetl's existing recovery tests.
    const FIXTURE_MNEMONIC: &str = "abandon abandon abandon abandon abandon abandon \
                                    abandon abandon abandon abandon abandon about";

    #[test]
    fn import_mnemonic_round_trips() {
        let store = KeyStore::new();
        let pub_line = store.import_mnemonic(FIXTURE_MNEMONIC).unwrap();
        assert!(pub_line.starts_with("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI"));
        assert!(pub_line.ends_with(" zetl-mobile"));
        assert_eq!(store.pub_openssh().as_deref(), Some(pub_line.as_str()));
        assert!(store.priv_pem().is_some());
        assert!(store.is_loaded());
    }

    #[test]
    fn rejects_garbage_mnemonic() {
        let store = KeyStore::new();
        let result = store.import_mnemonic("not a real mnemonic at all sorry");
        assert!(result.is_err());
        assert!(!store.is_loaded(), "failed import must not populate store");
    }

    #[test]
    fn deterministic_across_imports() {
        let a = KeyStore::new();
        let b = KeyStore::new();
        let line_a = a.import_mnemonic(FIXTURE_MNEMONIC).unwrap();
        let line_b = b.import_mnemonic(FIXTURE_MNEMONIC).unwrap();
        assert_eq!(line_a, line_b, "same seed must yield same pubkey");
    }
}
