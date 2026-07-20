//! Asset storage layer — effectful shell.
//!
//! Orchestrates I/O and calls into the pure core (`validation`, `metadata`).

use std::fs::{self};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::assets::metadata::AssetMeta;
use crate::assets::validation::validate_slug;

/// Error type for storage operations.
#[derive(Debug)]
pub enum StoreError {
    Io(io::Error),
    InvalidSlug(String),
    SlugExists,
    SlugNotFound,
    AssetDirOutsideRoot,
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Io(e) => write!(f, "I/O error: {e}"),
            StoreError::InvalidSlug(s) => write!(f, "invalid slug: {s}"),
            StoreError::SlugExists => write!(f, "slug already exists"),
            StoreError::SlugNotFound => write!(f, "slug not found"),
            StoreError::AssetDirOutsideRoot => write!(f, "asset path escapes vault root"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<io::Error> for StoreError {
    fn from(e: io::Error) -> Self {
        StoreError::Io(e)
    }
}

use std::fmt;

/// Encode bytes as lowercase hex string.
fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Compute the SHA-256 hex digest of a byte slice.
pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex_encode(&hasher.finalize())
}

/// Compute SHA-256 incrementally from a reader.
pub fn sha256_from_reader<R: Read>(reader: &mut R) -> io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex_encode(&hasher.finalize()))
}

/// Return the `assets/` directory under the vault root.
///
/// Assets are user content (uploaded images, PDFs, static HTML) and live
/// at the vault top-level so they are visible in file browsers, included
/// in standard `rsync`/backup workflows that skip `.zetl/`, and
/// identified clearly when the vault is opened in another tool. The
/// `assets/` name mirrors the `/assets/{*path}` URL namespace.
///
/// The vault scanner force-ignores this directory (see `src/scanner.rs`)
/// so user-uploaded `.md` files are not indexed as pages.
pub fn assets_dir(root: &Path) -> PathBuf {
    root.join("assets")
}

/// Return the temporary directory for atomic writes.
pub fn tmp_dir(root: &Path) -> PathBuf {
    assets_dir(root).join(".tmp")
}

/// Return the asset metadata directory under the vault root.
pub fn asset_meta_dir(root: &Path) -> PathBuf {
    root.join(".zetl").join("asset-meta")
}

/// Return the filesystem path for an asset slug.
///
/// Verifies that the resolved canonical path stays within `assets/`.
pub fn asset_path(root: &Path, slug: &str) -> Result<PathBuf, StoreError> {
    validate_slug(slug).map_err(|e| StoreError::InvalidSlug(e.to_string()))?;
    let dir = assets_dir(root);
    let target = dir.join(slug);
    // Ensure the assets dir exists so canonicalization resolves symlinks
    // consistently (e.g. /var → /private/var on macOS).
    let _ = fs::create_dir_all(&dir);
    // Canonicalize the assets dir, then verify target starts with it.
    let canon_dir = dir.canonicalize().unwrap_or_else(|_| dir.clone());
    // For files that don't exist yet, canonicalize the parent chain.
    let canon_target = if target.exists() {
        target.canonicalize().unwrap_or_else(|_| target.clone())
    } else {
        let mut base = target.clone();
        let mut tail = PathBuf::new();
        while !base.exists() {
            if let Some(name) = base.file_name() {
                tail = PathBuf::from(name).join(&tail);
            } else {
                break;
            }
            base = base.parent().unwrap_or(&base).to_path_buf();
        }
        base.canonicalize().unwrap_or(base).join(&tail)
    };
    if !canon_target.starts_with(&canon_dir) {
        return Err(StoreError::AssetDirOutsideRoot);
    }
    Ok(target)
}

/// Return the sidecar path for a given asset slug.
pub fn sidecar_path(root: &Path, slug: &str) -> PathBuf {
    asset_meta_dir(root).join(format!("{slug}.json"))
}

/// Recursively walk a directory, calling `f` for each file entry.
fn walk_files(dir: &Path, f: &mut dyn FnMut(&fs::DirEntry)) -> io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk_files(&path, f)?;
        } else {
            f(&entry);
        }
    }
    Ok(())
}

/// Initialise the storage counter by scanning `assets/`.
pub fn init_storage_total(root: &Path) -> io::Result<u64> {
    let dir = assets_dir(root);
    if !dir.exists() {
        return Ok(0);
    }
    let mut total = 0u64;
    let tmp = tmp_dir(root);
    walk_files(&dir, &mut |entry: &fs::DirEntry| {
        let path = entry.path();
        if path.starts_with(&tmp) {
            return;
        }
        if let Ok(meta) = entry.metadata() {
            total += meta.len();
        }
    })?;
    Ok(total)
}

/// Write an asset atomically: stream to tmp, then rename.
///
/// `body` is the raw file bytes.
pub fn write_asset(
    root: &Path,
    slug: &str,
    body: &[u8],
    mime_type: &str,
    user_id: &str,
    original_filename: &str,
) -> Result<AssetMeta, StoreError> {
    let target = asset_path(root, slug)?;
    let dir = assets_dir(root);
    fs::create_dir_all(tmp_dir(root))?;
    fs::create_dir_all(target.parent().unwrap_or(&dir))?;

    let tmp_name = format!("tmp-{}", uuid::Uuid::new_v4());
    let tmp_path = tmp_dir(root).join(&tmp_name);

    // Write tmp file
    if let Err(e) = fs::write(&tmp_path, body) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e.into());
    }

    // Compute SHA-256
    let sha = sha256_hex(body);

    let now = {
        let d = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let secs = d.as_secs();
        let days = secs / 86400;
        let day_secs = secs % 86400;
        let h = day_secs / 3600;
        let m = (day_secs % 3600) / 60;
        let s = day_secs % 60;
        let (y, mo, d) = days_to_ymd(days);
        format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
    };

    // Check if replacing
    let replaced_at = if target.exists() {
        Some(now.clone())
    } else {
        None
    };

    let meta = AssetMeta {
        slug: slug.to_string(),
        original_filename: original_filename.to_string(),
        mime_type: mime_type.to_string(),
        size_bytes: body.len() as u64,
        sha256: sha,
        uploaded_by: user_id.to_string(),
        uploaded_at: now,
        replaced_at,
        version: 1,
    };

    let sidecar = sidecar_path(root, slug);
    let sidecar_tmp = tmp_dir(root).join(format!("{}.json", tmp_name));

    // Write sidecar tmp
    if let Err(e) = fs::write(&sidecar_tmp, serde_json::to_vec_pretty(&meta).unwrap()) {
        let _ = fs::remove_file(&tmp_path);
        let _ = fs::remove_file(&sidecar_tmp);
        return Err(e.into());
    }

    // Atomic rename
    if let Err(e) = fs::rename(&tmp_path, &target) {
        let _ = fs::remove_file(&tmp_path);
        let _ = fs::remove_file(&sidecar_tmp);
        return Err(e.into());
    }
    if let Err(e) = fs::create_dir_all(sidecar.parent().unwrap_or(&asset_meta_dir(root))) {
        let _ = fs::remove_file(&target);
        let _ = fs::remove_file(&sidecar_tmp);
        return Err(e.into());
    }
    if let Err(e) = fs::rename(&sidecar_tmp, &sidecar) {
        // Try to roll back asset rename on sidecar failure
        let _ = fs::remove_file(&target);
        let _ = fs::remove_file(&sidecar_tmp);
        return Err(e.into());
    }

    Ok(meta)
}

/// Delete an asset and its sidecar atomically.
pub fn delete_asset(root: &Path, slug: &str) -> Result<(), StoreError> {
    let target = asset_path(root, slug)?;
    if !target.exists() {
        return Err(StoreError::SlugNotFound);
    }
    let sidecar = sidecar_path(root, slug);
    let tmp = tmp_dir(root);
    fs::create_dir_all(&tmp)?;
    let tmp_base = tmp.join(format!("del-{}", uuid::Uuid::new_v4()));
    let tmp_asset = tmp_base.with_extension("asset");
    let tmp_sidecar = tmp_base.with_extension("sidecar");
    fs::rename(&target, &tmp_asset)?;
    if sidecar.exists() {
        fs::rename(&sidecar, &tmp_sidecar)?;
    }
    let _ = fs::remove_file(&tmp_asset);
    let _ = fs::remove_file(&tmp_sidecar);
    Ok(())
}

/// List all assets, optionally filtered by prefix.
pub fn list_assets(root: &Path, prefix: Option<&str>) -> io::Result<Vec<AssetMeta>> {
    let dir = asset_meta_dir(root);
    let mut out = Vec::new();
    if !dir.exists() {
        return Ok(out);
    }
    walk_files(&dir, &mut |entry: &fs::DirEntry| {
        let path = entry.path();
        if path.extension().map(|e| e == "json").unwrap_or(false) {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(meta) = serde_json::from_str::<AssetMeta>(&content) {
                    if let Some(p) = prefix {
                        if meta.slug.starts_with(p) {
                            out.push(meta);
                        }
                    } else {
                        out.push(meta);
                    }
                }
            }
        }
    })?;
    out.sort_by(|a, b| a.slug.cmp(&b.slug));
    Ok(out)
}

/// Read an asset's metadata.
pub fn serve_asset(root: &Path, path: &str) -> Result<AssetMeta, StoreError> {
    let target = asset_path(root, path)?;
    if !target.exists() {
        return Err(StoreError::SlugNotFound);
    }
    let sidecar = sidecar_path(root, path);
    if sidecar.exists() {
        let content = fs::read_to_string(&sidecar)?;
        serde_json::from_str(&content).map_err(|e| StoreError::InvalidSlug(e.to_string()))
    } else {
        Err(StoreError::SlugNotFound)
    }
}

/// Thread-safe storage counter.
#[derive(Debug, Clone)]
pub struct StorageCounterGuard {
    inner: Arc<AtomicU64>,
}

impl StorageCounterGuard {
    pub fn new(initial: u64) -> Self {
        Self {
            inner: Arc::new(AtomicU64::new(initial)),
        }
    }

    pub fn increment(&self, n: u64) -> u64 {
        self.inner.fetch_add(n, Ordering::SeqCst) + n
    }

    pub fn decrement(&self, n: u64) -> u64 {
        self.inner
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                Some(current.saturating_sub(n))
            })
            .map(|old| old.saturating_sub(n))
            .unwrap()
    }

    pub fn total(&self) -> u64 {
        self.inner.load(Ordering::SeqCst)
    }

    pub fn try_increment(&self, n: u64, limit: u64) -> bool {
        self.inner
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                if current + n > limit {
                    None
                } else {
                    Some(current + n)
                }
            })
            .is_ok()
    }

    pub fn try_adjust(&self, delta: i64, limit: u64) -> bool {
        if delta <= 0 {
            let abs = (-delta) as u64;
            let _ = self.decrement(abs);
            return true;
        }
        let n = delta as u64;
        self.inner
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                let next = current.saturating_add(n);
                if next > limit {
                    None
                } else {
                    Some(next)
                }
            })
            .is_ok()
    }
}

/// Convert days since Unix epoch to (year, month, day).
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_basic() {
        assert_eq!(
            sha256_hex(b"hello"),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn storage_counter_basic() {
        let guard = StorageCounterGuard::new(100);
        assert_eq!(guard.total(), 100);
        assert_eq!(guard.increment(50), 150);
        assert_eq!(guard.decrement(30), 120);
    }

    #[test]
    fn storage_counter_saturating_decrement() {
        let guard = StorageCounterGuard::new(10);
        assert_eq!(guard.decrement(100), 0);
        assert_eq!(guard.total(), 0);
    }

    #[test]
    fn storage_counter_try_increment() {
        let guard = StorageCounterGuard::new(100);
        assert!(guard.try_increment(50, 200));
        assert_eq!(guard.total(), 150);
        assert!(!guard.try_increment(100, 200));
        assert_eq!(guard.total(), 150);
    }

    #[test]
    fn storage_counter_try_adjust_new_upload() {
        let guard = StorageCounterGuard::new(100);
        assert!(guard.try_adjust(50, 200));
        assert_eq!(guard.total(), 150);
    }

    #[test]
    fn storage_counter_try_adjust_rejects_over_quota() {
        let guard = StorageCounterGuard::new(100);
        assert!(!guard.try_adjust(200, 200));
        assert_eq!(guard.total(), 100);
    }

    #[test]
    fn storage_counter_try_adjust_negative_delta_shrinks() {
        let guard = StorageCounterGuard::new(100);
        assert!(guard.try_adjust(-30, 200));
        assert_eq!(guard.total(), 70);
    }

    #[test]
    fn storage_counter_concurrent_replace_no_drift() {
        let guard = StorageCounterGuard::new(100);
        guard.increment(50);
        assert_eq!(guard.total(), 150);
        let _ = guard.try_adjust(30, 200);
        assert_eq!(guard.total(), 180);
    }

    #[test]
    fn regression_bug001_decrement_underflow() {
        let guard = StorageCounterGuard::new(10);
        guard.decrement(5);
        assert_eq!(guard.total(), 5);
        guard.decrement(100);
        assert_eq!(guard.total(), 0);
        guard.increment(1);
        assert_eq!(guard.total(), 1);
    }

    #[test]
    fn sidecar_path_basic() {
        let root = Path::new("/vault");
        assert_eq!(
            sidecar_path(root, "foo.png"),
            PathBuf::from("/vault/.zetl/asset-meta/foo.png.json")
        );
        assert_eq!(
            sidecar_path(root, "images/diagram.png"),
            PathBuf::from("/vault/.zetl/asset-meta/images/diagram.png.json")
        );
    }
}
