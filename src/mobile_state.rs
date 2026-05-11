//! SPEC-040 mobile process-wide state (REQ-4003).
//!
//! Houses types that are shared between the Tauri Mobile shell crate
//! and the embedded `zetl serve` running in the same process. Lives
//! in the parent crate (rather than the mobile crate) so the
//! `/_mobile/*` route handlers in `web::mobile` can import them
//! without a circular dependency back into `zetl-mobile`.
//!
//! v0.1-strawman scope:
//!
//! - `KeyStore` — in-memory SSH keypair derived from a BIP39 seed
//!   (REQ-4003).
//! - Filesystem persistence at `{app_data_dir}/ssh_key.json` so the
//!   user does not re-enter the seed on every launch. The seed
//!   phrase itself is never written to disk; only the derived
//!   ed25519 keypair (priv_pem + pub_openssh) is persisted.
//! - Process-wide vault root + app data dir registered by the Tauri
//!   shell's `setup()` hook.
//!
//! v0.1 explicitly does **not** use the platform secure element
//! ([[iOS Keychain]] / [[Android Keystore]]). The persisted key file
//! is written with `0o600` permissions on Unix and lives inside the
//! Tauri-allocated app data directory, which is sandboxed on iOS and
//! Android. Moving to a real keychain is the next-but-one slice and
//! happens behind the same `KeyStore` API — no caller changes.

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
    /// display to the user. This is the **advanced** onboarding path
    /// — used when a user wants the phone to share the same SSH
    /// identity as a desktop already provisioned via `zetl
    /// derive-ssh-key --mnemonic`. The default path is
    /// [`Self::generate_new`] which keeps each device's key isolated.
    pub fn import_mnemonic(&self, mnemonic_phrase: &str) -> Result<String> {
        let signing: SigningKey =
            crate::user::recovery::derive_ssh_key_from_mnemonic(mnemonic_phrase)
                .context("BIP39 → ed25519 derivation failed")?;
        self.install_keypair(signing)
    }

    /// Generate a brand-new ed25519 keypair from OS randomness and
    /// install it in the store. This is the **default** onboarding
    /// path — each mobile device gets its own per-device key,
    /// registered with the git host like any other client. No seed
    /// transfer required between desktop and phone; no shared master
    /// secret.
    pub fn generate_new(&self) -> Result<String> {
        use rand_core::OsRng;
        let signing = SigningKey::generate(&mut OsRng);
        self.install_keypair(signing)
    }

    /// Shared install path used by both `import_mnemonic` and
    /// `generate_new`: extracts the keypair bytes, encodes them into
    /// OpenSSH PEM (private) and the `ssh-ed25519 AAAA…` line
    /// (public), and stores them under the same `StoredKey` slot so
    /// every caller observes the same in-memory key going forward.
    fn install_keypair(&self, signing: SigningKey) -> Result<String> {
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

    /// Write the in-memory keypair to `{app_data_dir}/ssh_key.json`
    /// with `0o600` permissions on Unix. Caller is the onboarding
    /// POST handler, immediately after a successful
    /// [`import_mnemonic`]. Returns `Ok(())` even if the file mode
    /// chmod fails on platforms that don't support it (Windows).
    pub fn persist(&self, app_data_dir: &std::path::Path) -> Result<()> {
        let guard = self.0.lock().expect("KeyStore mutex poisoned");
        let stored = guard
            .as_ref()
            .context("no key to persist; call import_mnemonic first")?;

        std::fs::create_dir_all(app_data_dir)
            .with_context(|| format!("create {}", app_data_dir.display()))?;

        let path = app_data_dir.join("ssh_key.json");
        let body = serde_json::json!({
            "schema": "zetl-mobile/ssh_key.v1",
            "pub_openssh": stored.pub_openssh,
            "priv_pem": stored.priv_pem,
        });
        let json = serde_json::to_string(&body).context("serialise key json")?;

        // Atomic write so a crash mid-persist never leaves the file
        // half-written. Same idiom as mobile_capture::write_atomic.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json.as_bytes())
            .with_context(|| format!("write {}", tmp.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))
                .with_context(|| format!("chmod 600 {}", tmp.display()))?;
        }
        std::fs::rename(&tmp, &path).with_context(|| format!("rename → {}", path.display()))?;
        Ok(())
    }

    /// Try to restore a previously-persisted keypair from
    /// `{app_data_dir}/ssh_key.json`. Returns `Ok(true)` if a key was
    /// loaded, `Ok(false)` if the file did not exist (fresh install).
    /// Errors only on parse failure of an existing file — that's an
    /// actionable corruption case worth surfacing.
    pub fn restore(&self, app_data_dir: &std::path::Path) -> Result<bool> {
        let path = app_data_dir.join("ssh_key.json");
        if !path.exists() {
            return Ok(false);
        }
        let raw =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let v: serde_json::Value =
            serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
        let pub_openssh = v["pub_openssh"]
            .as_str()
            .context("ssh_key.json missing pub_openssh")?
            .to_string();
        let priv_pem = v["priv_pem"]
            .as_str()
            .context("ssh_key.json missing priv_pem")?
            .to_string();

        let mut guard = self.0.lock().expect("KeyStore mutex poisoned");
        *guard = Some(StoredKey {
            pub_openssh,
            priv_pem,
        });
        Ok(true)
    }

    /// Forget any in-memory key. Used by tests to start from a clean
    /// slate; not currently exposed in the UI.
    pub fn clear(&self) {
        if let Ok(mut g) = self.0.lock() {
            *g = None;
        }
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
/// this to know where to clone into; capture / save handlers use it
/// as the working-tree root.
///
/// Stored behind a `Mutex` rather than a `OnceLock` to keep
/// integration tests flexible — production code sets the root once
/// at startup, but tests need to swap it per scenario without
/// restarting the process.
fn vault_root_cell() -> &'static Mutex<Option<PathBuf>> {
    static CELL: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(None))
}

/// Register the vault root for this process. Called by the Tauri
/// shell from its `setup()` hook. Overwrites any previous value.
pub fn set_vault_root(path: PathBuf) {
    if let Ok(mut g) = vault_root_cell().lock() {
        *g = Some(path);
    }
}

/// Vault root, if registered. Returns `None` outside of a Tauri shell
/// context (e.g. from in-process unit tests that exercise the
/// `/_mobile/*` handlers without configuring a vault first).
pub fn vault_root() -> Option<PathBuf> {
    vault_root_cell().lock().ok().and_then(|g| g.clone())
}

/// Process-wide app-data dir — typically `~/Library/Application Support/io.anuna.zetl.mobile`
/// on macOS, the corresponding sandboxed location on iOS / Android.
/// Used by [`KeyStore::persist`] and [`KeyStore::restore`].
fn app_data_dir_cell() -> &'static Mutex<Option<PathBuf>> {
    static CELL: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(None))
}

pub fn set_app_data_dir(path: PathBuf) {
    if let Ok(mut g) = app_data_dir_cell().lock() {
        *g = Some(path);
    }
}

pub fn app_data_dir() -> Option<PathBuf> {
    app_data_dir_cell().lock().ok().and_then(|g| g.clone())
}

// ── Vault metadata (label + remote) ──────────────────────────────────────────

/// Metadata for the currently-active vault — derived label, the
/// remote URL the user cloned from, and a clone timestamp. Stored at
/// `{app_data_dir}/vault_meta.json`. v0.1 keeps a single-vault story:
/// only one of these exists at a time. v0.2 will move this to a
/// per-vault subdir under `vaults/{label}/meta.json`.
#[derive(Clone, Debug)]
pub struct VaultMeta {
    pub label: String,
    pub remote_url: String,
    pub cloned_at: String,
}

/// Derive a short, human-readable label from a git remote URL.
/// Accepts both SSH (`git@host:owner/repo.git`) and HTTPS
/// (`https://host/owner/repo.git`) forms; returns `"owner/repo"` for
/// well-formed URLs, falling back to a sanitised tail otherwise.
pub fn derive_vault_label(remote_url: &str) -> String {
    let stripped = remote_url
        .trim()
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .trim_end_matches('/');

    // Walk from the end, taking segments split on '/' or ':'.
    let parts: Vec<&str> = stripped
        .rsplit(|c: char| c == '/' || c == ':')
        .filter(|s| !s.is_empty())
        .collect();

    match parts.as_slice() {
        [repo, owner, ..] => format!("{owner}/{repo}"),
        [single] => (*single).to_string(),
        _ => "vault".to_string(),
    }
}

/// Persist the vault metadata after a successful clone.
pub fn write_vault_meta(app_data_dir: &std::path::Path, meta: &VaultMeta) -> Result<()> {
    std::fs::create_dir_all(app_data_dir)
        .with_context(|| format!("create {}", app_data_dir.display()))?;
    let body = serde_json::json!({
        "schema": "zetl-mobile/vault_meta.v1",
        "label": meta.label,
        "remote_url": meta.remote_url,
        "cloned_at": meta.cloned_at,
    });
    let path = app_data_dir.join("vault_meta.json");
    std::fs::write(&path, serde_json::to_string(&body).context("serialise vault meta")?)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// Read the vault metadata if present.
pub fn read_vault_meta(app_data_dir: &std::path::Path) -> Option<VaultMeta> {
    let path = app_data_dir.join("vault_meta.json");
    let raw = std::fs::read_to_string(&path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    Some(VaultMeta {
        label: v["label"].as_str()?.to_string(),
        remote_url: v["remote_url"].as_str()?.to_string(),
        cloned_at: v["cloned_at"].as_str().unwrap_or("").to_string(),
    })
}

/// Convenience: read meta from the registered `app_data_dir()`.
pub fn vault_meta() -> Option<VaultMeta> {
    read_vault_meta(&app_data_dir()?)
}

#[cfg(test)]
mod label_tests {
    use super::derive_vault_label;

    #[test]
    fn https_url() {
        assert_eq!(
            derive_vault_label("https://github.com/anuna-cooperative/agent-comms-wiki.git"),
            "anuna-cooperative/agent-comms-wiki"
        );
    }

    #[test]
    fn ssh_url() {
        assert_eq!(
            derive_vault_label("git@codeberg.org:anuna/zetl.git"),
            "anuna/zetl"
        );
    }

    #[test]
    fn ssh_url_no_git_suffix() {
        assert_eq!(
            derive_vault_label("git@gitlab.com:group/project"),
            "group/project"
        );
    }

    #[test]
    fn trailing_slash() {
        assert_eq!(
            derive_vault_label("https://codeberg.org/anuna/zetl/"),
            "anuna/zetl"
        );
    }

    #[test]
    fn whitespace_trimmed() {
        assert_eq!(
            derive_vault_label("  https://github.com/x/y.git  "),
            "x/y"
        );
    }

    #[test]
    fn single_segment_fallback() {
        assert_eq!(derive_vault_label("vault.git"), "vault");
    }
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

    #[test]
    fn persist_then_restore_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let original_pub = {
            let store = KeyStore::new();
            let pub_line = store.import_mnemonic(FIXTURE_MNEMONIC).unwrap();
            store.persist(dir.path()).unwrap();
            pub_line
        };

        // Fresh KeyStore — no key in memory.
        let store = KeyStore::new();
        assert!(!store.is_loaded());

        let loaded = store.restore(dir.path()).unwrap();
        assert!(loaded);
        assert!(store.is_loaded());
        assert_eq!(store.pub_openssh().as_deref(), Some(original_pub.as_str()));
    }

    #[test]
    fn restore_on_fresh_install_is_ok_false() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::new();
        let loaded = store.restore(dir.path()).unwrap();
        assert!(!loaded);
        assert!(!store.is_loaded());
    }

    #[test]
    fn restore_on_corrupted_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ssh_key.json"), b"{not json")
            .unwrap();
        let store = KeyStore::new();
        let res = store.restore(dir.path());
        assert!(res.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn persisted_file_is_mode_600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::new();
        store.import_mnemonic(FIXTURE_MNEMONIC).unwrap();
        store.persist(dir.path()).unwrap();
        let meta = std::fs::metadata(dir.path().join("ssh_key.json")).unwrap();
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "ssh_key.json must be 0600, got {mode:o}");
    }

    #[test]
    fn clear_drops_in_memory_key() {
        let store = KeyStore::new();
        store.import_mnemonic(FIXTURE_MNEMONIC).unwrap();
        assert!(store.is_loaded());
        store.clear();
        assert!(!store.is_loaded());
    }

    #[test]
    fn generate_new_produces_valid_per_device_key() {
        let store = KeyStore::new();
        let pub_line = store.generate_new().unwrap();
        assert!(pub_line.starts_with("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5"));
        assert!(pub_line.ends_with(" zetl-mobile"));
        assert!(store.is_loaded());
        assert!(store.priv_pem().is_some());
    }

    #[test]
    fn generate_new_is_non_deterministic() {
        let a = KeyStore::new();
        let b = KeyStore::new();
        let line_a = a.generate_new().unwrap();
        let line_b = b.generate_new().unwrap();
        assert_ne!(
            line_a, line_b,
            "two fresh generate_new() calls must produce distinct keys"
        );
    }
}
