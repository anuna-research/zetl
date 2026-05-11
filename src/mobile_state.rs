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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{anyhow, Context, Result};
use ed25519_dalek::SigningKey;

// ── Keyring backend (production opt-in) ──────────────────────────────────────
//
// The Tauri shell calls `enable_keyring()` once at boot so `persist` /
// `restore` prefer the OS keychain over the on-disk ssh_key.json. Tests
// leave the flag off (default) so they continue to exercise the file
// path against a tempdir without polluting the real keyring.

static USE_KEYRING: AtomicBool = AtomicBool::new(false);

const KEYRING_SERVICE: &str = "io.anuna.zetl.mobile";
const KEYRING_USER: &str = "ssh_key";

/// Opt the SPEC-040 KeyStore into using the OS keychain (macOS
/// Keychain Services / Windows Credential Vault / Linux Secret
/// Service via the `keyring` crate) for `persist` and `restore`.
/// Production callers (the Tauri shell's setup() hook) flip this once
/// at boot. Tests leave it off so the file path is exercised.
pub fn enable_keyring() {
    USE_KEYRING.store(true, Ordering::Relaxed);
}

fn keyring_enabled() -> bool {
    USE_KEYRING.load(Ordering::Relaxed)
}

/// Process-lifetime SSH key store. Cheap to clone; clones share the
/// same inner `Mutex<Option<StoredKey>>` so updates from one entry
/// point are visible to all later callers across both Tauri commands
/// and embedded-serve handlers.
#[derive(Clone, Default)]
pub struct KeyStore(Arc<Mutex<Option<StoredKey>>>);

#[derive(Clone)]
struct StoredKey {
    /// 12-word BIP39 mnemonic. The master secret — the SSH keypair
    /// is deterministically derived from it via SLIP-0010 path
    /// `m/44'/2'/0'`. Persisted so the user can write it down and
    /// recover the same identity on a new device.
    mnemonic: String,
    /// `ssh-ed25519 AAAA<base64-blob> zetl-mobile` — the line the
    /// user pastes into their git host's "add SSH key" page. Derived
    /// from the mnemonic; cached in-memory to avoid re-derivation on
    /// every read.
    pub_openssh: String,
    /// OpenSSH PEM private key, used by the `git2` credential
    /// callback for clone / pull / push. Derived from the mnemonic.
    priv_pem: String,
}

impl KeyStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adopt an existing 12-word BIP39 mnemonic — the **shared-identity**
    /// onboarding path. Used when the user wants the phone to derive the
    /// same SSH key as a desktop already provisioned via
    /// `zetl derive-ssh-key --mnemonic`.
    pub fn import_mnemonic(&self, mnemonic_phrase: &str) -> Result<String> {
        let trimmed = mnemonic_phrase.trim().to_string();
        let signing: SigningKey =
            crate::user::recovery::derive_ssh_key_from_mnemonic(&trimmed)
                .context("BIP39 → ed25519 derivation failed")?;
        self.install_from_mnemonic(trimmed, signing)
    }

    /// Generate a brand-new 12-word BIP39 mnemonic, derive the SSH
    /// key from it, install it in the store, and return the formatted
    /// public-key line. The mnemonic is the master secret — persist
    /// it via [`Self::persist`] so it survives restarts and the user
    /// can write it down for off-device recovery.
    pub fn generate_new(&self) -> Result<String> {
        use rand_core::OsRng;
        let mnemonic = bip39::Mnemonic::generate_in_with(
            &mut OsRng,
            bip39::Language::English,
            12,
        )
        .context("BIP39 mnemonic generation failed")?
        .to_string();
        let signing: SigningKey =
            crate::user::recovery::derive_ssh_key_from_mnemonic(&mnemonic)
                .context("BIP39 → ed25519 derivation failed")?;
        self.install_from_mnemonic(mnemonic, signing)
    }

    /// Shared install path used by both `import_mnemonic` and
    /// `generate_new`. Stores the mnemonic + derived keys under the
    /// same `StoredKey` slot.
    fn install_from_mnemonic(&self, mnemonic: String, signing: SigningKey) -> Result<String> {
        let pub_bytes: [u8; 32] = signing.verifying_key().to_bytes();
        let priv_bytes: [u8; 32] = signing.to_bytes();

        let priv_pem = crate::user::recovery::encode_openssh_ed25519(&priv_bytes, &pub_bytes);
        let pub_openssh = format_ssh_ed25519_pub_line(&pub_bytes, "zetl-mobile");

        let mut guard = self.0.lock().expect("KeyStore mutex poisoned");
        *guard = Some(StoredKey {
            mnemonic,
            pub_openssh: pub_openssh.clone(),
            priv_pem,
        });
        Ok(pub_openssh)
    }

    /// BIP39 recovery phrase for the currently-loaded key, if any.
    /// Used by the `/_mobile/recovery` route to display the seed to
    /// the user with a "write this down" warning.
    pub fn mnemonic(&self) -> Option<String> {
        self.0
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|k| k.mnemonic.clone()))
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

    /// Persist the in-memory key material. Prefers the OS keychain
    /// when `enable_keyring()` was called at boot (production path);
    /// falls back to `{app_data_dir}/ssh_key.json` (0o600 Unix)
    /// otherwise — which is also the test default.
    ///
    /// v2 schema persists the BIP39 mnemonic; the derived `priv_pem`
    /// + `pub_openssh` fields are kept alongside for backwards-compat
    /// with v1 readers and to avoid re-deriving on every restore.
    pub fn persist(&self, app_data_dir: &std::path::Path) -> Result<()> {
        let guard = self.0.lock().expect("KeyStore mutex poisoned");
        let stored = guard
            .as_ref()
            .context("no key to persist; call import_mnemonic / generate_new first")?;
        let body = serde_json::json!({
            "schema": "zetl-mobile/ssh_key.v2",
            "mnemonic": stored.mnemonic,
            "pub_openssh": stored.pub_openssh,
            "priv_pem": stored.priv_pem,
        });
        let json = serde_json::to_string(&body).context("serialise key json")?;
        drop(guard);

        // Try keyring first when production has opted in. If the
        // platform-specific backend doesn't work (no Secret Service
        // on a headless Linux box, locked Keychain on macOS, …),
        // log + fall through to the on-disk path so the user is
        // never blocked.
        #[cfg(feature = "keyring-storage")]
        if keyring_enabled() {
            match keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
                Ok(entry) => match entry.set_password(&json) {
                    Ok(()) => {
                        // Successful keyring write — clean up any
                        // stale on-disk file so the two backends
                        // don't drift.
                        let _ = std::fs::remove_file(app_data_dir.join("ssh_key.json"));
                        return Ok(());
                    }
                    Err(e) => eprintln!(
                        "[zetl-mobile] keyring set_password failed: {e}; falling back to file"
                    ),
                },
                Err(e) => {
                    eprintln!("[zetl-mobile] keyring Entry::new failed: {e}; falling back to file")
                }
            }
        }

        std::fs::create_dir_all(app_data_dir)
            .with_context(|| format!("create {}", app_data_dir.display()))?;

        let path = app_data_dir.join("ssh_key.json");

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

    /// Try to restore a previously-persisted keypair. When
    /// `enable_keyring()` was called at boot, the keyring entry is
    /// consulted first; missing-entry or platform error → fall back
    /// to `{app_data_dir}/ssh_key.json`. Returns `Ok(true)` if a key
    /// was loaded, `Ok(false)` if neither backend has anything
    /// (fresh install).
    ///
    /// Schema v2 (current) carries the BIP39 mnemonic. Schema v1
    /// (older installs) carries only priv_pem + pub_openssh; we
    /// load it with `mnemonic = ""` so users with v1 storage
    /// continue to work but `/_mobile/recovery` tells them to
    /// re-onboard to get a recoverable phrase.
    pub fn restore(&self, app_data_dir: &std::path::Path) -> Result<bool> {
        // Try keyring first when production opted in.
        #[cfg(feature = "keyring-storage")]
        if keyring_enabled() {
            if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
                match entry.get_password() {
                    Ok(raw) => return self.load_from_json(&raw, "keyring"),
                    Err(keyring::Error::NoEntry) => {
                        // First launch with keyring enabled — fall through
                        // to file. Useful when migrating an existing install.
                    }
                    Err(e) => eprintln!(
                        "[zetl-mobile] keyring get_password failed: {e}; falling back to file"
                    ),
                }
            }
        }

        let path = app_data_dir.join("ssh_key.json");
        if !path.exists() {
            return Ok(false);
        }
        let raw =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let loaded = self.load_from_json(&raw, "ssh_key.json")?;

        // If we read from disk but keyring is enabled, migrate the
        // material into the keyring on the next persist so the disk
        // file can age out. We don't write here — that's persist's
        // job.
        Ok(loaded)
    }

    fn load_from_json(&self, raw: &str, source: &str) -> Result<bool> {
        let v: serde_json::Value =
            serde_json::from_str(raw).with_context(|| format!("parse key json from {source}"))?;
        let pub_openssh = v["pub_openssh"]
            .as_str()
            .with_context(|| format!("{source} missing pub_openssh"))?
            .to_string();
        let priv_pem = v["priv_pem"]
            .as_str()
            .with_context(|| format!("{source} missing priv_pem"))?
            .to_string();
        let mnemonic = v["mnemonic"].as_str().unwrap_or("").to_string();
        let mut guard = self.0.lock().expect("KeyStore mutex poisoned");
        *guard = Some(StoredKey {
            mnemonic,
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

    /// Best-effort wipe of all persisted backends — keyring entry
    /// AND on-disk file. Used by `/_mobile/reset` so the user can
    /// truly start fresh.
    pub fn forget_persistent(&self, app_data_dir: &std::path::Path) {
        #[cfg(feature = "keyring-storage")]
        if keyring_enabled() {
            if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
                let _ = entry.delete_credential();
            }
        }
        let _ = std::fs::remove_file(app_data_dir.join("ssh_key.json"));
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

// ── Share-extension inbox (REQ-4007 SPEC-040) ────────────────────────────────
//
// Native share-extension targets (iOS Share Extension, Android
// ACTION_SEND Activity) write payloads here when the user shares
// to zetl-mobile from another app. The main app drains the inbox on
// launch and prefills `/_mobile/capture` with the first entry.
//
// File layout: `app_data_dir/share-inbox.jsonl` — one JSON object
// per line:
//   {"received_at": "2026-05-11T01:23:45Z",
//    "kind": "text" | "url" | "url_with_title",
//    "title": "...",
//    "body":  "..."}
//
// Append-only so the share extension and the main app can write
// concurrently without locking. The main app reads + truncates.

use serde::{Deserialize, Serialize};
use std::io::Write;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShareInboxEntry {
    pub received_at: String,
    pub kind: String,
    pub title: String,
    pub body: String,
}

fn share_inbox_path(app_data_dir: &std::path::Path) -> PathBuf {
    app_data_dir.join("share-inbox.jsonl")
}

/// Append one entry to the share inbox. Called by the
/// `/_mobile/share` POST handler (used by Android's ShareReceiver
/// and by tests / external tooling). Native iOS Share Extensions
/// write directly to the same file via the app-group container.
pub fn append_share_entry(entry: &ShareInboxEntry) -> Result<()> {
    let app_data = app_data_dir().context("app_data_dir not registered")?;
    std::fs::create_dir_all(&app_data)
        .with_context(|| format!("create {}", app_data.display()))?;
    let path = share_inbox_path(&app_data);
    let line = serde_json::to_string(entry).context("serialize share entry")?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open {}", path.display()))?;
    writeln!(f, "{line}").with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// Non-destructive count of entries currently in the inbox. Used by
/// the Tauri shell to decide whether to navigate the WebView to
/// `/_mobile/capture?from=share` on startup instead of the usual
/// `/_mobile/vaults` landing.
pub fn share_inbox_count() -> usize {
    let Some(app_data) = app_data_dir() else {
        return 0;
    };
    let path = share_inbox_path(&app_data);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return 0;
    };
    raw.lines().filter(|l| !l.trim().is_empty()).count()
}

/// Drain every pending entry from the inbox and delete the file.
/// Called by the `/_mobile/capture?from=share` GET handler to
/// retrieve payloads from native share extensions.
pub fn drain_share_inbox() -> Vec<ShareInboxEntry> {
    let Some(app_data) = app_data_dir() else {
        return Vec::new();
    };
    let path = share_inbox_path(&app_data);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let entries: Vec<ShareInboxEntry> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let _ = std::fs::remove_file(&path);
    entries
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

// ── Multi-vault filesystem layout ────────────────────────────────────────────
//
// app_data_dir/
//   ssh_key.json                # one device key shared across vaults
//   vault → vaults/<active>     # symlink to active working tree (Unix only;
//                               # v0.2 Windows story: a small JSON pointer file)
//   vaults/
//     anuna-zetl/                 # working tree
//       .git/
//       README.md ...
//     anuna-cooperative-agent-comms-wiki/
//       ...
//
// The vault's *label* is its directory name under `vaults/`, derived
// from the remote URL at clone time via `derive_vault_label`. The
// remote URL is recovered on demand from `git remote get-url origin`
// so we don't need a sidecar meta file per vault.

/// One entry in the list of cloned vaults.
#[derive(Clone, Debug)]
pub struct VaultEntry {
    pub label: String,
    pub path: PathBuf,
    pub remote_url: Option<String>,
    pub is_active: bool,
}

/// Path to the `vaults/` container, if `app_data_dir` is registered.
pub fn vaults_dir() -> Option<PathBuf> {
    app_data_dir().map(|d| d.join("vaults"))
}

/// Label of the currently-active vault (the `vault` symlink's
/// target's basename), if any.
pub fn active_vault_label() -> Option<String> {
    let link = vault_root()?;
    let target = std::fs::read_link(&link).ok()?;
    target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
}

/// Point the `vault` symlink at `vaults/<label>[/<subpath>]`. The
/// optional subpath lets the active vault be a nested directory
/// within a cloned repo — useful when the git repo's vault content
/// lives under e.g. `notes/` rather than the repo root.
pub fn set_active_vault(label: &str, subpath: Option<&str>) -> Result<()> {
    let app_data = app_data_dir().context("app_data_dir not registered")?;
    let vaults = app_data.join("vaults");
    let mut abs_target = vaults.join(label);
    if let Some(sub) = subpath {
        let sub = sub.trim().trim_matches('/');
        if !sub.is_empty() {
            abs_target = abs_target.join(sub);
        }
    }
    if !abs_target.exists() {
        return Err(anyhow!(
            "vault target does not exist at {}",
            abs_target.display()
        ));
    }
    let link = vault_root().context("vault_root not registered")?;
    if link.is_symlink() {
        std::fs::remove_file(&link).ok();
    } else if link.exists() {
        std::fs::remove_dir_all(&link).ok();
    }
    #[cfg(unix)]
    {
        let rel_target = match subpath {
            Some(s) if !s.trim().trim_matches('/').is_empty() => {
                format!("vaults/{label}/{}", s.trim().trim_matches('/'))
            }
            _ => format!("vaults/{label}"),
        };
        std::os::unix::fs::symlink(&rel_target, &link)
            .with_context(|| format!("symlink {} → {}", link.display(), rel_target))?;
    }
    #[cfg(not(unix))]
    return Err(anyhow!("multi-vault symlink switching not yet supported on this platform"));
    #[cfg(unix)]
    Ok(())
}

/// One candidate vault location inside a cloned repo. Surfaced to
/// the picker UI after clone when more than one directory looks like
/// a viable zetl vault.
#[derive(Clone, Debug)]
pub struct VaultSubpathCandidate {
    /// Path relative to the repo root. Empty string == repo root.
    pub subpath: String,
    /// Count of `.md` files directly in this directory (not recursive).
    pub md_count: usize,
    /// Whether this dir contains a `.zetl/` config dir — the strongest
    /// signal it's a real zetl vault.
    pub has_zetl_dir: bool,
}

/// Scan a freshly-cloned repo for plausible vault subdirectories.
///
/// Strategy: check the repo root first; then each immediate
/// subdirectory (skipping hidden `.foo/` and common non-vault names
/// like `node_modules`, `target`, `dist`). A directory counts as a
/// candidate if it has at least one `.md` file or a `.zetl/` config
/// dir. Returns candidates sorted by score (zetl-dir first, then
/// md-count desc, then alphabetical).
pub fn detect_vault_subpath_candidates(repo_root: &std::path::Path) -> Vec<VaultSubpathCandidate> {
    let mut out = Vec::new();

    fn scan_one(dir: &std::path::Path, subpath: String, out: &mut Vec<VaultSubpathCandidate>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut md_count = 0;
        let mut has_zetl_dir = false;
        for ent in rd.flatten() {
            let name = ent.file_name();
            let name = name.to_string_lossy();
            let path = ent.path();
            if path.is_dir() && name == ".zetl" {
                has_zetl_dir = true;
            } else if path.is_file()
                && name
                    .to_ascii_lowercase()
                    .ends_with(".md")
            {
                md_count += 1;
            }
        }
        if md_count > 0 || has_zetl_dir {
            out.push(VaultSubpathCandidate {
                subpath,
                md_count,
                has_zetl_dir,
            });
        }
    }

    scan_one(repo_root, String::new(), &mut out);

    if let Ok(rd) = std::fs::read_dir(repo_root) {
        for ent in rd.flatten() {
            let name = ent.file_name();
            let name = name.to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            const SKIP: &[&str] = &[
                "node_modules",
                "target",
                "dist",
                "build",
                "out",
                ".git",
                ".github",
                "vendor",
            ];
            if SKIP.contains(&name.as_str()) {
                continue;
            }
            let p = ent.path();
            if !p.is_dir() {
                continue;
            }
            scan_one(&p, name, &mut out);
        }
    }

    out.sort_by(|a, b| {
        b.has_zetl_dir
            .cmp(&a.has_zetl_dir)
            .then(b.md_count.cmp(&a.md_count))
            .then(a.subpath.cmp(&b.subpath))
    });
    out
}

/// Scan the `vaults/` container and return every cloned vault. The
/// active one (per the symlink) is flagged.
pub fn list_vaults() -> Vec<VaultEntry> {
    let Some(vaults) = vaults_dir() else {
        return Vec::new();
    };
    let active = active_vault_label();
    let mut entries: Vec<VaultEntry> = std::fs::read_dir(&vaults)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|ent| {
            let path = ent.path();
            if !path.join(".git").exists() {
                return None;
            }
            let label = ent.file_name().to_string_lossy().into_owned();
            let remote_url = read_remote_url(&path);
            let is_active = active.as_deref() == Some(label.as_str());
            Some(VaultEntry {
                label,
                path,
                remote_url,
                is_active,
            })
        })
        .collect();
    entries.sort_by(|a, b| a.label.cmp(&b.label));
    entries
}

fn read_remote_url(repo_path: &std::path::Path) -> Option<String> {
    let repo = git2::Repository::open(repo_path).ok()?;
    repo.find_remote("origin")
        .ok()
        .and_then(|r| r.url().map(String::from))
}

/// Migrate a legacy single-vault layout (`app_data_dir/vault/` as a
/// real working tree) into the multi-vault layout
/// (`app_data_dir/vaults/<label>/` + symlink). Idempotent: no-op if
/// the layout is already multi-vault, or if no vault is present.
/// Called from the Tauri shell's `setup()` before the embedded
/// serve spawns.
pub fn migrate_single_vault_layout(app_data: &std::path::Path) -> Result<()> {
    let vault = app_data.join("vault");
    let vaults = app_data.join("vaults");
    std::fs::create_dir_all(&vaults)
        .with_context(|| format!("create {}", vaults.display()))?;

    if !vault.exists() {
        return Ok(());
    }
    if vault.is_symlink() {
        return Ok(()); // already multi-vault
    }
    if !vault.join(".git").exists() {
        // Empty/stub dir from the old single-vault Tauri shell init;
        // safe to remove so a fresh symlink can be placed later.
        std::fs::remove_dir_all(&vault).ok();
        return Ok(());
    }

    // Real legacy working tree — move it into vaults/<label>/ and
    // symlink. Label comes from the remote URL if we can read it,
    // otherwise from a generic fallback.
    let remote = read_remote_url(&vault);
    let label = remote
        .as_deref()
        .map(derive_vault_label)
        .unwrap_or_else(|| "migrated-vault".to_string());
    let target = vaults.join(&label);
    if target.exists() {
        // Collision: append `-migrated`. v0.2 will surface this to
        // the user properly; v0.1 just renames to avoid data loss.
        let alt = vaults.join(format!("{label}-migrated"));
        std::fs::rename(&vault, &alt)
            .with_context(|| format!("rename legacy vault → {}", alt.display()))?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            format!("vaults/{}", alt.file_name().unwrap().to_string_lossy()),
            &vault,
        )
        .ok();
    } else {
        std::fs::rename(&vault, &target)
            .with_context(|| format!("rename legacy vault → {}", target.display()))?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(format!("vaults/{label}"), &vault)
            .with_context(|| format!("symlink {} → vaults/{label}", vault.display()))?;
    }

    // Drop the now-stale per-app-data vault_meta.json — meta is
    // derived from the on-disk vault layout now.
    let _ = std::fs::remove_file(app_data.join("vault_meta.json"));

    Ok(())
}

// ── Legacy single-vault meta (kept for backwards-compat in tests) ────────────

/// Metadata for the currently-active vault. v0.1 single-vault path
/// wrote this to `{app_data_dir}/vault_meta.json`; multi-vault no
/// longer uses the file — meta is derived from the on-disk
/// `vaults/<label>/` directory plus `git remote get-url origin`.
/// The type is retained for the `vault_meta()` accessor which now
/// reads from the active vault entry.
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

    // The label doubles as a directory name under `vaults/`, so we
    // hyphenate `owner/repo` rather than nesting it as two path
    // components. (Display callers receive the same hyphenated form;
    // a v0.2 design can split display vs filesystem labels if the UX
    // calls for it.)
    match parts.as_slice() {
        [repo, owner, ..] => format!("{owner}-{repo}"),
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

/// Convenience: derive vault metadata for the currently-active
/// vault. Multi-vault: looks up the active entry via `list_vaults()`
/// and constructs a `VaultMeta` from it. Falls back to the legacy
/// `read_vault_meta(app_data_dir)` for tests that pre-date the
/// vaults/ layout migration.
pub fn vault_meta() -> Option<VaultMeta> {
    let active = list_vaults().into_iter().find(|v| v.is_active);
    if let Some(entry) = active {
        return Some(VaultMeta {
            label: entry.label,
            remote_url: entry.remote_url.unwrap_or_default(),
            cloned_at: String::new(),
        });
    }
    // Legacy fallback (pre-migration tests / install).
    read_vault_meta(&app_data_dir()?)
}

// ── Vault data handle (live reindex plumbing) ────────────────────────────────

fn vault_data_handle_cell(
) -> &'static Mutex<Option<Arc<std::sync::RwLock<crate::web::VaultData>>>> {
    static CELL: OnceLock<Mutex<Option<Arc<std::sync::RwLock<crate::web::VaultData>>>>> =
        OnceLock::new();
    CELL.get_or_init(|| Mutex::new(None))
}

/// Registered by [`crate::web::launch_default`] at boot. The mobile
/// switch-vault path uses this to swap the embedded serve's in-memory
/// `VaultData` after the symlink moves to a different working tree —
/// the page list would otherwise show the old vault's content.
pub fn set_vault_data_handle(handle: Arc<std::sync::RwLock<crate::web::VaultData>>) {
    if let Ok(mut g) = vault_data_handle_cell().lock() {
        *g = Some(handle);
    }
}

/// Re-scan the active vault and swap the embedded serve's
/// `VaultData` in place. Returns the new page count on success.
pub fn trigger_reindex() -> Result<usize> {
    let handle = {
        let guard = vault_data_handle_cell().lock().expect("handle mutex");
        guard
            .clone()
            .context("vault_data_handle not registered — embedded serve not booted")?
    };
    let vault_root = vault_root().context("vault_root not registered")?;
    let new = crate::web::reindex(&vault_root).context("reindex failed")?;
    let count = new.page_names.len();
    let mut w = handle.write().expect("vault data rwlock");
    *w = new;
    Ok(count)
}

#[cfg(test)]
mod label_tests {
    use super::derive_vault_label;

    #[test]
    fn https_url() {
        assert_eq!(
            derive_vault_label("https://github.com/anuna-cooperative/agent-comms-wiki.git"),
            "anuna-cooperative-agent-comms-wiki"
        );
    }

    #[test]
    fn ssh_url() {
        assert_eq!(
            derive_vault_label("git@codeberg.org:anuna/zetl.git"),
            "anuna-zetl"
        );
    }

    #[test]
    fn ssh_url_no_git_suffix() {
        assert_eq!(
            derive_vault_label("git@gitlab.com:group/project"),
            "group-project"
        );
    }

    #[test]
    fn trailing_slash() {
        assert_eq!(
            derive_vault_label("https://codeberg.org/anuna/zetl/"),
            "anuna-zetl"
        );
    }

    #[test]
    fn whitespace_trimmed() {
        assert_eq!(
            derive_vault_label("  https://github.com/x/y.git  "),
            "x-y"
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
        // BIP39 mnemonic should be available alongside the derived key.
        let phrase = store.mnemonic().expect("mnemonic should be set");
        assert_eq!(
            phrase.split_whitespace().count(),
            12,
            "generated mnemonic should be 12 words; got {phrase:?}"
        );
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
        assert_ne!(
            a.mnemonic().unwrap(),
            b.mnemonic().unwrap(),
            "fresh generate_new() calls must produce distinct mnemonics"
        );
    }

    #[test]
    fn import_mnemonic_round_trip_includes_phrase() {
        let store = KeyStore::new();
        store.import_mnemonic(FIXTURE_MNEMONIC).unwrap();
        assert_eq!(
            store.mnemonic().as_deref(),
            Some(FIXTURE_MNEMONIC),
            "imported mnemonic should be retrievable"
        );
    }

    #[test]
    fn share_inbox_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        super::set_app_data_dir(tmp.path().to_path_buf());
        // Start clean — drain anything left from other tests.
        let _ = super::drain_share_inbox();
        let _ = std::fs::remove_file(super::share_inbox_path(tmp.path()));

        assert_eq!(super::share_inbox_count(), 0);
        super::append_share_entry(&super::ShareInboxEntry {
            received_at: "2026-05-11T12:00:00Z".into(),
            kind: "url".into(),
            title: "Example".into(),
            body: "https://example.com".into(),
        })
        .unwrap();
        super::append_share_entry(&super::ShareInboxEntry {
            received_at: "2026-05-11T12:00:05Z".into(),
            kind: "text".into(),
            title: "".into(),
            body: "a plain note".into(),
        })
        .unwrap();
        assert_eq!(super::share_inbox_count(), 2);

        let drained = super::drain_share_inbox();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].kind, "url");
        assert_eq!(drained[1].body, "a plain note");
        // Inbox file is gone after drain.
        assert_eq!(super::share_inbox_count(), 0);
    }

    #[test]
    fn detect_vault_subpath_root_with_md() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("Welcome.md"), "x").unwrap();
        std::fs::write(tmp.path().join("Other.md"), "y").unwrap();
        let candidates = super::detect_vault_subpath_candidates(tmp.path());
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].subpath, "");
        assert_eq!(candidates[0].md_count, 2);
    }

    #[test]
    fn detect_vault_subpath_nested_only() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("README"), "x").unwrap(); // no .md ext
        let notes = tmp.path().join("notes");
        std::fs::create_dir(&notes).unwrap();
        std::fs::write(notes.join("A.md"), "x").unwrap();
        std::fs::write(notes.join("B.md"), "y").unwrap();
        let candidates = super::detect_vault_subpath_candidates(tmp.path());
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].subpath, "notes");
        assert_eq!(candidates[0].md_count, 2);
    }

    #[test]
    fn detect_vault_subpath_zetl_dir_outranks_md_count() {
        let tmp = tempfile::tempdir().unwrap();
        let big = tmp.path().join("big-md-dir");
        std::fs::create_dir(&big).unwrap();
        for n in 0..10 {
            std::fs::write(big.join(format!("{n}.md")), "x").unwrap();
        }
        let real_vault = tmp.path().join("real-vault");
        std::fs::create_dir_all(real_vault.join(".zetl")).unwrap();
        std::fs::write(real_vault.join("One.md"), "x").unwrap();
        let candidates = super::detect_vault_subpath_candidates(tmp.path());
        assert!(candidates.iter().any(|c| c.subpath == "real-vault"));
        let first = &candidates[0];
        assert_eq!(first.subpath, "real-vault");
        assert!(first.has_zetl_dir);
    }

    #[test]
    fn detect_vault_subpath_skips_excluded_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        for skip in ["node_modules", "target", ".github"] {
            let p = tmp.path().join(skip);
            std::fs::create_dir(&p).unwrap();
            std::fs::write(p.join("noise.md"), "x").unwrap();
        }
        let candidates = super::detect_vault_subpath_candidates(tmp.path());
        for c in &candidates {
            assert!(
                !["node_modules", "target", ".github"].contains(&c.subpath.as_str()),
                "should skip {} but got: {candidates:?}",
                c.subpath
            );
        }
    }

    #[test]
    fn persist_then_restore_carries_mnemonic_through() {
        let dir = tempfile::tempdir().unwrap();
        let store1 = KeyStore::new();
        let original = store1.generate_new().unwrap();
        let original_mnemonic = store1.mnemonic().unwrap();
        store1.persist(dir.path()).unwrap();

        let store2 = KeyStore::new();
        let loaded = store2.restore(dir.path()).unwrap();
        assert!(loaded);
        assert_eq!(store2.pub_openssh().as_deref(), Some(original.as_str()));
        assert_eq!(store2.mnemonic().as_deref(), Some(original_mnemonic.as_str()));
    }
}
