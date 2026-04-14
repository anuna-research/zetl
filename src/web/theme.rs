//! Theme manifest parsing for zetl themes.
//!
//! Handles loading and validation of `theme.toml` files from both on-disk
//! user-installed themes and compile-time-bundled themes. Also provides the
//! `ThemeSource` struct for parsing `.zetl-source.toml` provenance files.

use std::path::Path;

use anyhow::{bail, Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};

use super::engine::bundled_template;

// ── Install source ────────────────────────────────────────────────────────────

/// Parsed representation of a `zetl theme install <source>` argument.
///
/// This is distinct from [`ThemeSource`], which is the provenance record stored
/// in `.zetl-source.toml` after installation.
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeInstallSource {
    pub url: String,             // Resolved git URL
    pub git_ref: Option<String>, // Branch, tag, or SHA
    pub path: Option<String>,    // Subdirectory within repo
}

/// Parse a theme install source string into a [`ThemeInstallSource`].
///
/// Accepts:
/// - `user/repo` — GitHub shorthand (exactly one slash, no protocol)
/// - `user/repo#ref` — same with an optional git ref
/// - `https://...` or `http://...` — full URL, optional `#ref` fragment
/// - `git@...` — SCP-style SSH URL, optional `#ref` fragment
///
/// Rejects anything that doesn't match one of these patterns.
pub fn parse_install_source(source: &str) -> Result<ThemeInstallSource> {
    if source.starts_with("https://")
        || source.starts_with("http://")
        || source.starts_with("file://")
    {
        let (url, git_ref) = split_ref(source);
        return Ok(ThemeInstallSource {
            url: url.to_string(),
            git_ref: git_ref.map(str::to_string),
            path: None,
        });
    }

    if source.starts_with("git@") {
        let (url, git_ref) = split_ref(source);
        return Ok(ThemeInstallSource {
            url: url.to_string(),
            git_ref: git_ref.map(str::to_string),
            path: None,
        });
    }

    // GitHub shorthand: exactly one slash, no protocol
    let (without_ref, git_ref) = split_ref(source);
    let slash_count = without_ref.matches('/').count();
    if slash_count == 1 {
        let (user, repo) = without_ref.split_once('/').unwrap();
        if !user.is_empty() && !repo.is_empty() {
            let url = format!("https://github.com/{user}/{repo}.git");
            return Ok(ThemeInstallSource {
                url,
                git_ref: git_ref.map(str::to_string),
                path: None,
            });
        }
    }

    bail!(
        "unrecognized source {source:?}: expected 'user/repo', 'https://...', 'file://...', \
         or 'git@...' (optionally with '#ref')"
    )
}

/// Derive a safe theme name from an arbitrary raw string.
///
/// Lowercases the input, replaces every non-alphanumeric character with a
/// hyphen, then strips leading/trailing hyphens.
pub fn sanitize_theme_name(raw: &str) -> String {
    let replaced: String = raw
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    replaced.trim_matches('-').to_string()
}

/// Determine the theme name to use for a freshly installed theme.
///
/// Precedence (REQ-014-009):
/// 1. `--name` flag
/// 2. `manifest.theme.name`
/// 3. Last path component of `--path`
/// 4. Repo name derived from the resolved git URL
///
/// The chosen name is validated with [`validate_theme_name`] before returning.
pub fn resolve_theme_name(
    name_flag: Option<&str>,
    manifest: Option<&ThemeManifest>,
    path: Option<&str>,
    source: &ThemeInstallSource,
) -> Result<String> {
    // 1. --name flag
    if let Some(name) = name_flag {
        validate_theme_name(name)?;
        return Ok(name.to_string());
    }

    // 2. manifest.theme.name
    if let Some(m) = manifest {
        validate_theme_name(&m.theme.name)?;
        return Ok(m.theme.name.clone());
    }

    // 3. Last component of --path
    if let Some(p) = path {
        if let Some(last) = Path::new(p).file_name().and_then(|n| n.to_str()) {
            if !last.is_empty() {
                let sanitized = sanitize_theme_name(last);
                validate_theme_name(&sanitized).with_context(|| {
                    format!("cannot derive a valid theme name from path component {last:?}")
                })?;
                return Ok(sanitized);
            }
        }
    }

    // 4. Repo name from URL
    if let Some(raw) = repo_name_from_url(&source.url) {
        let sanitized = sanitize_theme_name(&raw);
        validate_theme_name(&sanitized).with_context(|| {
            format!("cannot derive a valid theme name from URL {:?}", source.url)
        })?;
        return Ok(sanitized);
    }

    bail!("could not determine a theme name; please specify --name")
}

// ── Manifest structs ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeManifest {
    pub theme: ThemeInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeInfo {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub min_zetl_version: Option<String>,
    #[serde(default)]
    pub templates: Option<ThemeTemplates>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeTemplates {
    #[serde(default)]
    pub overrides: Vec<String>,
}

// ── Provenance struct ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeSource {
    pub source: ThemeSourceInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeSourceInfo {
    pub url: String,
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub ref_name: Option<String>,
    pub commit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub installed_at: String,
    pub zetl_version: String,
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Validate that `name` matches the pattern `^[a-z0-9][a-z0-9_-]*$`.
///
/// Returns a descriptive error on failure, including a sanitised suggestion.
pub fn validate_theme_name(name: &str) -> Result<()> {
    let re = Regex::new(r"^[a-z0-9][a-z0-9_-]*$").unwrap();
    if re.is_match(name) {
        Ok(())
    } else {
        let sanitized = sanitize_theme_name(name);
        if sanitized.is_empty() {
            bail!(
                "invalid theme name {name:?}: must match ^[a-z0-9][a-z0-9_-]*$ \
                 (lowercase letters, digits, hyphens, underscores; must start with a letter or digit)"
            )
        } else {
            bail!(
                "invalid theme name {name:?}: must match ^[a-z0-9][a-z0-9_-]*$ \
                 (lowercase letters, digits, hyphens, underscores; must start with a letter or digit)\n\
                 hint: try {sanitized:?} instead"
            )
        }
    }
}

/// Parse TOML content into a [`ThemeManifest`].
///
/// Validates that:
/// - `theme.name` matches `^[a-z0-9][a-z0-9_-]*$`
/// - `theme.version` is a valid SemVer string (e.g. `"1.0.0"`)
pub fn parse_theme_manifest(content: &str) -> Result<ThemeManifest> {
    let manifest: ThemeManifest = toml::from_str(content).context("failed to parse theme.toml")?;

    validate_theme_name(&manifest.theme.name).with_context(|| "theme.toml: invalid name field")?;

    validate_semver(&manifest.theme.version)
        .with_context(|| format!("theme.toml: invalid version {:?}", manifest.theme.version))?;

    Ok(manifest)
}

/// Load and parse `theme.toml` from `theme_dir`.
///
/// Returns `Ok(None)` if the file does not exist. Returns an error if the file
/// exists but cannot be read or is malformed.
pub fn load_theme_manifest(theme_dir: &Path) -> Result<Option<ThemeManifest>> {
    let toml_path = theme_dir.join("theme.toml");
    if !toml_path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&toml_path)
        .with_context(|| format!("failed to read {}", toml_path.display()))?;
    let manifest = parse_theme_manifest(&content)
        .with_context(|| format!("malformed theme manifest: {}", toml_path.display()))?;
    Ok(Some(manifest))
}

/// Load and parse the compile-time-bundled `theme.toml` for `theme_name`.
///
/// Returns `Ok(None)` if the theme or its `theme.toml` is not embedded.
/// Returns an error if the file is present but malformed.
pub fn load_bundled_manifest(theme_name: &str) -> Result<Option<ThemeManifest>> {
    let Some(content) = bundled_template(theme_name, "theme.toml") else {
        return Ok(None);
    };
    let manifest = parse_theme_manifest(content)
        .with_context(|| format!("malformed bundled theme manifest for {theme_name:?}"))?;
    Ok(Some(manifest))
}

/// Parse TOML content into a [`ThemeSource`] provenance record.
pub fn parse_theme_source(content: &str) -> Result<ThemeSource> {
    toml::from_str(content).context("failed to parse .zetl-source.toml")
}

/// Write a `.zetl-source.toml` provenance file into `theme_dir`.
///
/// Records the git URL, optional ref, resolved commit SHA, optional subdirectory
/// path, installation timestamp (UTC ISO 8601), and the current zetl version
/// from `CARGO_PKG_VERSION`.
pub fn write_provenance(
    theme_dir: &Path,
    source: &ThemeInstallSource,
    clone_result: &CloneResult,
) -> Result<()> {
    let record = ThemeSource {
        source: ThemeSourceInfo {
            url: source.url.clone(),
            ref_name: source.git_ref.clone(),
            commit: clone_result.commit_sha.clone(),
            path: source.path.clone(),
            installed_at: current_utc_iso8601(),
            zetl_version: env!("CARGO_PKG_VERSION").to_string(),
        },
    };

    let content =
        toml::to_string_pretty(&record).context("failed to serialize provenance record")?;

    let dest = theme_dir.join(".zetl-source.toml");
    std::fs::write(&dest, content)
        .with_context(|| format!("failed to write {}", dest.display()))?;

    Ok(())
}

/// Read and parse `.zetl-source.toml` from `theme_dir`.
///
/// Returns `None` if the file does not exist or cannot be parsed.
pub fn read_provenance(theme_dir: &Path) -> Option<ThemeSource> {
    let path = theme_dir.join(".zetl-source.toml");
    let content = std::fs::read_to_string(&path).ok()?;
    parse_theme_source(&content).ok()
}

// ── Internal helpers ─────────────────────────────────────────────────────────

/// Return the current UTC time as an ISO 8601 string (e.g. `"2024-01-15T10:30:00Z"`).
fn current_utc_iso8601() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    unix_secs_to_iso8601(secs)
}

/// Convert a Unix timestamp (seconds since 1970-01-01T00:00:00Z) to ISO 8601 UTC.
fn unix_secs_to_iso8601(secs: u64) -> String {
    let tod = secs % 86400;
    let (h, m, s) = (tod / 3600, (tod % 3600) / 60, tod % 60);
    let (year, month, day) = days_to_ymd((secs / 86400) as i64);
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}Z")
}

/// Convert a count of days since the Unix epoch to `(year, month, day)`.
fn days_to_ymd(mut days: i64) -> (i32, u32, u32) {
    let mut year = 1970i32;
    loop {
        let dy = if is_leap_year(year) { 366i64 } else { 365i64 };
        if days < dy {
            break;
        }
        days -= dy;
        year += 1;
    }
    let month_days: [u32; 12] = [
        31,
        if is_leap_year(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1u32;
    for &md in &month_days {
        if days < md as i64 {
            break;
        }
        days -= md as i64;
        month += 1;
    }
    (year, month, days as u32 + 1)
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

/// Split a source string on the first `#`, returning `(base, Some(ref))` or
/// `(source, None)` if no `#` is present.
fn split_ref(s: &str) -> (&str, Option<&str>) {
    match s.find('#') {
        Some(pos) => {
            let git_ref = &s[pos + 1..];
            (
                &s[..pos],
                if git_ref.is_empty() {
                    None
                } else {
                    Some(git_ref)
                },
            )
        }
        None => (s, None),
    }
}

/// Extract the repository name from a git URL.
///
/// Works for both `https://host/user/repo.git` and `git@host:user/repo.git`.
/// Returns the last slash-or-colon-separated segment, with any `.git` suffix
/// stripped.
fn repo_name_from_url(url: &str) -> Option<String> {
    let last = url.rsplit(['/', ':']).find(|s| !s.is_empty())?;
    let name = last.strip_suffix(".git").unwrap_or(last);
    Some(name.to_string())
}

/// Validate that `version` is a well-formed SemVer string.
///
/// Accepts `MAJOR.MINOR.PATCH` with optional pre-release and build metadata
/// (e.g. `"1.0.0"`, `"2.1.0-beta.1"`, `"3.0.0+build.123"`).
fn validate_semver(version: &str) -> Result<()> {
    let re = Regex::new(r"^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$").unwrap();
    if re.is_match(version) {
        Ok(())
    } else {
        bail!(
            "version {version:?} is not valid SemVer (expected MAJOR.MINOR.PATCH, e.g. \"1.0.0\")"
        )
    }
}

// ── Git clone ─────────────────────────────────────────────────────────────────

/// Result of a successful [`clone_theme`] operation.
#[derive(Debug, Clone)]
pub struct CloneResult {
    pub commit_sha: String,
    pub files_copied: usize,
    pub total_bytes: u64,
}

/// Clone a theme repository into `target_dir`.
///
/// 1. Creates a temporary directory.
/// 2. Clones the repository (shallow for branch/tag refs, full for commit SHAs).
/// 3. If `source.path` is set, verifies the subdirectory exists.
/// 4. Copies theme files to `target_dir`, excluding `.git/`, `.github/`,
///    `.gitignore`, and `.gitattributes`.
/// 5. Returns the resolved commit SHA, file count, and total bytes written.
pub fn clone_theme(source: &ThemeInstallSource, target_dir: &Path) -> Result<CloneResult> {
    let tmp = tempfile::tempdir().context("failed to create temporary directory")?;
    let tmpdir = tmp.path();

    let is_sha = source.git_ref.as_deref().map(is_sha_ref).unwrap_or(false);
    if is_sha {
        // Full clone required to check out an arbitrary commit SHA.
        git_do_clone(None, &source.url, tmpdir, false)?;
        git_do_checkout(tmpdir, source.git_ref.as_deref().unwrap())?;
    } else {
        // Shallow clone with optional branch/tag ref.
        git_do_clone(source.git_ref.as_deref(), &source.url, tmpdir, true)?;
    }

    let commit_sha = git_rev_parse_head(tmpdir)?;

    let source_dir = match &source.path {
        Some(path) => {
            let subdir = tmpdir.join(path);
            if !subdir.is_dir() {
                let hints = find_theme_toml_dirs(tmpdir);
                if hints.is_empty() {
                    bail!("path {path:?} was not found in the cloned repository");
                }
                let dirs = hints.join(", ");
                bail!(
                    "path {path:?} was not found in the cloned repository; \
                     directories containing theme.toml: {dirs}"
                );
            }
            subdir
        }
        None => tmpdir.to_path_buf(),
    };

    std::fs::create_dir_all(target_dir)
        .with_context(|| format!("failed to create {}", target_dir.display()))?;
    let (files_copied, total_bytes) = copy_theme_files(&source_dir, target_dir)?;

    Ok(CloneResult {
        commit_sha,
        files_copied,
        total_bytes,
    })
}

// ── Git helpers ───────────────────────────────────────────────────────────────

/// Returns `true` if `r` looks like a commit SHA: at least 7 hex characters.
fn is_sha_ref(r: &str) -> bool {
    r.len() >= 7 && r.chars().all(|c| c.is_ascii_hexdigit())
}

/// Execute `cmd`, mapping "executable not found" to a friendly error.
fn git_run(cmd: &mut std::process::Command) -> Result<std::process::Output> {
    cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            #[cfg(target_os = "macos")]
            let hint = "\nhint: install git with: brew install git  (or: xcode-select --install)";
            #[cfg(target_os = "windows")]
            let hint = "\nhint: install git from https://git-scm.com/download/win or with: winget install Git.Git";
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            let hint = "\nhint: install git with: sudo apt install git  (or: sudo dnf install git)";
            anyhow::anyhow!(
                "git is required for theme installation but was not found on PATH{hint}"
            )
        } else {
            anyhow::anyhow!("failed to run git: {e}")
        }
    })
}

/// Run `git clone [--depth 1] [--branch <ref>] <url> <target>`.
fn git_do_clone(git_ref: Option<&str>, url: &str, target: &Path, shallow: bool) -> Result<()> {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("clone");
    if shallow {
        cmd.args(["--depth", "1"]);
    }
    if let Some(r) = git_ref {
        cmd.args(["--branch", r]);
    }
    cmd.arg(url).arg(target);

    let out = git_run(&mut cmd)?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stderr_str = stderr.trim();

        let looks_like_network = stderr_str.contains("Could not resolve host")
            || stderr_str.contains("Failed to connect")
            || stderr_str.contains("Connection refused")
            || stderr_str.contains("Network is unreachable")
            || stderr_str.contains("timed out");

        let looks_like_not_found = stderr_str.contains("Repository not found")
            || stderr_str.contains("repository not found")
            || stderr_str.contains("does not exist")
            || stderr_str.contains("Authentication failed");

        if looks_like_network {
            bail!(
                "git clone failed: {stderr_str}\n\
                 hint: check your network connectivity and try again"
            );
        } else if looks_like_not_found {
            bail!(
                "git clone failed: {stderr_str}\n\
                 hint: repository not found at {url} — verify the URL is correct and you have access"
            );
        } else {
            bail!(
                "git clone failed: {stderr_str}\n\
                 hint: tried to clone from {url}"
            );
        }
    }
    Ok(())
}

/// Run `git -C <dir> checkout <sha>`.
fn git_do_checkout(dir: &Path, sha: &str) -> Result<()> {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("-C").arg(dir).args(["checkout", sha]);

    let out = git_run(&mut cmd)?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        bail!("git checkout {:?} failed: {}", sha, stderr.trim());
    }
    Ok(())
}

/// Run `git -C <dir> rev-parse HEAD` and return the trimmed commit SHA.
fn git_rev_parse_head(dir: &Path) -> Result<String> {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("-C").arg(dir).args(["rev-parse", "HEAD"]);

    let out = git_run(&mut cmd)?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        bail!("git rev-parse HEAD failed: {}", stderr.trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Recursively find subdirectories of `root` that contain a `theme.toml`.
///
/// Skips `.git` and `.github` directories. Returns paths relative to `root`,
/// sorted lexicographically.
fn find_theme_toml_dirs(root: &Path) -> Vec<String> {
    let mut hints = Vec::new();
    find_theme_toml_dirs_inner(root, root, &mut hints);
    hints.sort();
    hints
}

fn find_theme_toml_dirs_inner(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == ".git" || name == ".github" {
            continue;
        }
        if path.join("theme.toml").exists() {
            if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_string_lossy().into_owned());
            }
        }
        find_theme_toml_dirs_inner(root, &path, out);
    }
}

// ── File copy helpers ─────────────────────────────────────────────────────────

/// Copy theme files from `src` to `dst`.
///
/// Excludes `.git/`, `.github/`, `.gitignore`, and `.gitattributes`.
/// Preserves directory structure.
///
/// Returns `(files_copied, total_bytes)`.
fn copy_theme_files(src: &Path, dst: &Path) -> Result<(usize, u64)> {
    let mut files = 0usize;
    let mut bytes = 0u64;
    copy_dir_recursive(src, src, dst, &mut files, &mut bytes)?;
    Ok((files, bytes))
}

fn copy_dir_recursive(
    root: &Path,
    current: &Path,
    dst_root: &Path,
    files: &mut usize,
    bytes: &mut u64,
) -> Result<()> {
    for entry in std::fs::read_dir(current)
        .with_context(|| format!("failed to read directory {}", current.display()))?
        .flatten()
    {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        if path.is_dir() {
            if name == ".git" || name == ".github" {
                continue;
            }
            let rel = path.strip_prefix(root).expect("path is under root");
            let dest = dst_root.join(rel);
            std::fs::create_dir_all(&dest)
                .with_context(|| format!("failed to create directory {}", dest.display()))?;
            copy_dir_recursive(root, &path, dst_root, files, bytes)?;
        } else if path.is_file() {
            if name == ".gitignore" || name == ".gitattributes" {
                continue;
            }
            let rel = path.strip_prefix(root).expect("path is under root");
            let dest = dst_root.join(rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create directory {}", parent.display()))?;
            }
            let n = std::fs::copy(&path, &dest).with_context(|| {
                format!("failed to copy {} to {}", path.display(), dest.display())
            })?;
            *files += 1;
            *bytes += n;
        }
    }
    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── validate_theme_name ──────────────────────────────────────────────────

    #[test]
    fn test_valid_theme_names() {
        for name in &[
            "default",
            "my-theme",
            "theme2",
            "a",
            "0",
            "dark_mode",
            "a1-b2",
        ] {
            assert!(
                validate_theme_name(name).is_ok(),
                "expected {name:?} to be valid"
            );
        }
    }

    #[test]
    fn test_invalid_theme_names() {
        for name in &[
            "",
            "-start",
            "_start",
            "UPPER",
            "has space",
            "has.dot",
            "has@at",
        ] {
            assert!(
                validate_theme_name(name).is_err(),
                "expected {name:?} to be invalid"
            );
        }
    }

    // ── validate_semver ──────────────────────────────────────────────────────

    #[test]
    fn test_valid_semver() {
        for v in &[
            "0.1.0",
            "1.0.0",
            "10.20.30",
            "1.0.0-alpha",
            "1.0.0-beta.1",
            "1.0.0+build.123",
            "1.0.0-rc.1+build.456",
        ] {
            assert!(
                validate_semver(v).is_ok(),
                "expected {v:?} to be valid semver"
            );
        }
    }

    #[test]
    fn test_invalid_semver() {
        for v in &["1.0", "1", "v1.0.0", "1.0.0.0", "latest", ""] {
            assert!(
                validate_semver(v).is_err(),
                "expected {v:?} to be invalid semver"
            );
        }
    }

    // ── parse_theme_manifest ─────────────────────────────────────────────────

    #[test]
    fn test_parse_minimal_manifest() {
        let toml = r#"
[theme]
name = "my-theme"
version = "1.0.0"
"#;
        let m = parse_theme_manifest(toml).unwrap();
        assert_eq!(m.theme.name, "my-theme");
        assert_eq!(m.theme.version, "1.0.0");
        assert!(m.theme.description.is_none());
        assert!(m.theme.templates.is_none());
    }

    #[test]
    fn test_parse_full_manifest() {
        let toml = r#"
[theme]
name = "default"
version = "0.9.0"
description = "The default zetl theme"
author = "zetl contributors"
license = "AGPL-3.0"
homepage = "https://example.com"
min_zetl_version = "0.1.0"

[theme.templates]
overrides = ["base.html", "index.html", "page.html", "folder.html"]
"#;
        let m = parse_theme_manifest(toml).unwrap();
        assert_eq!(m.theme.name, "default");
        assert_eq!(
            m.theme.description.as_deref(),
            Some("The default zetl theme")
        );
        let templates = m.theme.templates.unwrap();
        assert_eq!(templates.overrides.len(), 4);
        assert!(templates.overrides.contains(&"base.html".to_string()));
    }

    #[test]
    fn test_parse_invalid_name_rejected() {
        let toml = r#"
[theme]
name = "MyTheme"
version = "1.0.0"
"#;
        assert!(parse_theme_manifest(toml).is_err());
    }

    #[test]
    fn test_parse_invalid_version_rejected() {
        let toml = r#"
[theme]
name = "good-name"
version = "v1.0"
"#;
        assert!(parse_theme_manifest(toml).is_err());
    }

    #[test]
    fn test_parse_missing_required_field() {
        // missing `version`
        let toml = r#"
[theme]
name = "my-theme"
"#;
        assert!(parse_theme_manifest(toml).is_err());
    }

    // ── load_theme_manifest ──────────────────────────────────────────────────

    #[test]
    fn test_load_theme_manifest_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let result = load_theme_manifest(tmp.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_load_theme_manifest_valid() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("theme.toml"),
            "[theme]\nname = \"test\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        let result = load_theme_manifest(tmp.path()).unwrap().unwrap();
        assert_eq!(result.theme.name, "test");
    }

    #[test]
    fn test_load_theme_manifest_malformed() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("theme.toml"), "not valid toml ][").unwrap();
        let result = load_theme_manifest(tmp.path());
        assert!(result.is_err());
    }

    // ── load_bundled_manifest ────────────────────────────────────────────────

    #[test]
    fn test_load_bundled_manifest_default() {
        let result = load_bundled_manifest("default").unwrap().unwrap();
        assert_eq!(result.theme.name, "default");
        assert!(!result.theme.version.is_empty());
    }

    #[test]
    fn test_load_bundled_manifest_unknown() {
        let result = load_bundled_manifest("no-such-theme").unwrap();
        assert!(result.is_none());
    }

    // ── parse_install_source ─────────────────────────────────────────────────

    #[test]
    fn test_github_shorthand() {
        let src = parse_install_source("user/repo").unwrap();
        assert_eq!(src.url, "https://github.com/user/repo.git");
        assert!(src.git_ref.is_none());
        assert!(src.path.is_none());
    }

    #[test]
    fn test_github_shorthand_with_ref() {
        let src = parse_install_source("user/repo#v2.0").unwrap();
        assert_eq!(src.url, "https://github.com/user/repo.git");
        assert_eq!(src.git_ref.as_deref(), Some("v2.0"));
    }

    #[test]
    fn test_https_url() {
        let src = parse_install_source("https://example.com/user/theme.git").unwrap();
        assert_eq!(src.url, "https://example.com/user/theme.git");
        assert!(src.git_ref.is_none());
    }

    #[test]
    fn test_https_url_with_ref() {
        let src = parse_install_source("https://example.com/user/theme.git#main").unwrap();
        assert_eq!(src.url, "https://example.com/user/theme.git");
        assert_eq!(src.git_ref.as_deref(), Some("main"));
    }

    #[test]
    fn test_ssh_url() {
        let src = parse_install_source("git@github.com:user/repo.git").unwrap();
        assert_eq!(src.url, "git@github.com:user/repo.git");
        assert!(src.git_ref.is_none());
    }

    #[test]
    fn test_ssh_url_with_ref() {
        let src = parse_install_source("git@github.com:user/repo.git#v1.0").unwrap();
        assert_eq!(src.url, "git@github.com:user/repo.git");
        assert_eq!(src.git_ref.as_deref(), Some("v1.0"));
    }

    #[test]
    fn test_invalid_source_rejected() {
        for bad in &[
            "not-a-valid-source",
            "a/b/c",          // too many slashes for shorthand
            "/leading-slash", // empty user part
            "#just-ref",      // no URL
            "ftp://example.com/repo.git",
        ] {
            assert!(
                parse_install_source(bad).is_err(),
                "expected {bad:?} to be rejected"
            );
        }
    }

    // ── sanitize_theme_name ──────────────────────────────────────────────────

    #[test]
    fn test_sanitize_simple() {
        assert_eq!(sanitize_theme_name("my-theme"), "my-theme");
        assert_eq!(sanitize_theme_name("MyTheme"), "mytheme");
        assert_eq!(sanitize_theme_name("my.theme"), "my-theme");
        assert_eq!(sanitize_theme_name("my_theme"), "my-theme");
    }

    #[test]
    fn test_sanitize_strips_edges() {
        assert_eq!(sanitize_theme_name("-foo-"), "foo");
        assert_eq!(sanitize_theme_name("---bar---"), "bar");
        assert_eq!(sanitize_theme_name(".hidden"), "hidden");
    }

    #[test]
    fn test_sanitize_repo_name() {
        assert_eq!(sanitize_theme_name("dark-theme"), "dark-theme");
        assert_eq!(sanitize_theme_name("zetl.theme"), "zetl-theme");
    }

    // ── resolve_theme_name ───────────────────────────────────────────────────

    fn make_source(url: &str) -> ThemeInstallSource {
        ThemeInstallSource {
            url: url.to_string(),
            git_ref: None,
            path: None,
        }
    }

    fn make_manifest(name: &str) -> ThemeManifest {
        ThemeManifest {
            theme: ThemeInfo {
                name: name.to_string(),
                version: "1.0.0".to_string(),
                description: None,
                author: None,
                license: None,
                homepage: None,
                min_zetl_version: None,
                templates: None,
            },
        }
    }

    #[test]
    fn test_resolve_prefers_name_flag() {
        let src = make_source("https://github.com/user/repo.git");
        let manifest = make_manifest("manifest-name");
        let name = resolve_theme_name(
            Some("flag-name"),
            Some(&manifest),
            Some("themes/path"),
            &src,
        )
        .unwrap();
        assert_eq!(name, "flag-name");
    }

    #[test]
    fn test_resolve_prefers_manifest_over_path() {
        let src = make_source("https://github.com/user/repo.git");
        let manifest = make_manifest("manifest-name");
        let name = resolve_theme_name(None, Some(&manifest), Some("themes/path"), &src).unwrap();
        assert_eq!(name, "manifest-name");
    }

    #[test]
    fn test_resolve_uses_path_component() {
        let src = make_source("https://github.com/user/repo.git");
        let name = resolve_theme_name(None, None, Some("themes/dark"), &src).unwrap();
        assert_eq!(name, "dark");
    }

    #[test]
    fn test_resolve_falls_back_to_url() {
        let src = make_source("https://github.com/user/my-theme.git");
        let name = resolve_theme_name(None, None, None, &src).unwrap();
        assert_eq!(name, "my-theme");
    }

    #[test]
    fn test_resolve_ssh_url() {
        let src = make_source("git@github.com:user/cool-theme.git");
        let name = resolve_theme_name(None, None, None, &src).unwrap();
        assert_eq!(name, "cool-theme");
    }

    #[test]
    fn test_resolve_invalid_name_flag_rejected() {
        let src = make_source("https://github.com/user/repo.git");
        assert!(resolve_theme_name(Some("BadName"), None, None, &src).is_err());
    }

    // ── parse_theme_source ───────────────────────────────────────────────────

    #[test]
    fn test_parse_theme_source() {
        let toml = r#"
[source]
url = "https://example.com/theme.git"
commit = "abc123def456"
installed_at = "2024-01-01T00:00:00Z"
zetl_version = "0.1.0"
"#;
        let s = parse_theme_source(toml).unwrap();
        assert_eq!(s.source.url, "https://example.com/theme.git");
        assert_eq!(s.source.commit, "abc123def456");
        assert!(s.source.ref_name.is_none());
        assert!(s.source.path.is_none());
    }

    #[test]
    fn test_parse_theme_source_full() {
        let toml = r#"
[source]
url = "https://example.com/theme.git"
ref = "main"
commit = "abc123"
path = "themes/dark"
installed_at = "2024-06-15T12:00:00Z"
zetl_version = "0.2.0"
"#;
        let s = parse_theme_source(toml).unwrap();
        assert_eq!(s.source.ref_name.as_deref(), Some("main"));
        assert_eq!(s.source.path.as_deref(), Some("themes/dark"));
    }

    // ── is_sha_ref ───────────────────────────────────────────────────────────

    #[test]
    fn test_is_sha_ref_full() {
        // 40 hex chars = full SHA
        assert!(is_sha_ref("a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2"));
    }

    #[test]
    fn test_is_sha_ref_short() {
        // 7–39 hex chars = short SHA prefix
        assert!(is_sha_ref("a1b2c3d"));
        assert!(is_sha_ref("deadbeef"));
    }

    #[test]
    fn test_is_sha_ref_too_short() {
        // Fewer than 7 chars → not treated as SHA
        assert!(!is_sha_ref("abc123"));
        assert!(!is_sha_ref("main"));
        assert!(!is_sha_ref("v1.0"));
    }

    #[test]
    fn test_is_sha_ref_non_hex() {
        // Non-hex characters → not a SHA
        assert!(!is_sha_ref("feature-branch"));
        assert!(!is_sha_ref("release/1.0"));
    }

    // ── find_theme_toml_dirs ─────────────────────────────────────────────────

    #[test]
    fn test_find_theme_toml_dirs_none() {
        let tmp = tempfile::tempdir().unwrap();
        let hints = find_theme_toml_dirs(tmp.path());
        assert!(hints.is_empty());
    }

    #[test]
    fn test_find_theme_toml_dirs_found() {
        let tmp = tempfile::tempdir().unwrap();
        let theme_dir = tmp.path().join("themes").join("dark");
        std::fs::create_dir_all(&theme_dir).unwrap();
        std::fs::write(theme_dir.join("theme.toml"), "").unwrap();

        let hints = find_theme_toml_dirs(tmp.path());
        assert_eq!(hints, vec!["themes/dark"]);
    }

    #[test]
    fn test_find_theme_toml_dirs_skips_git() {
        let tmp = tempfile::tempdir().unwrap();
        // Put a theme.toml inside .git — should be ignored
        let git_dir = tmp.path().join(".git").join("hooks");
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::write(git_dir.join("theme.toml"), "").unwrap();

        let hints = find_theme_toml_dirs(tmp.path());
        assert!(hints.is_empty());
    }

    // ── copy_theme_files ─────────────────────────────────────────────────────

    #[test]
    fn test_copy_theme_files_basic() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();

        std::fs::write(
            src.path().join("theme.toml"),
            "[theme]\nname=\"x\"\nversion=\"1.0.0\"\n",
        )
        .unwrap();
        std::fs::write(src.path().join("base.html"), "<html></html>").unwrap();

        let (count, bytes) = copy_theme_files(src.path(), dst.path()).unwrap();
        assert_eq!(count, 2);
        assert!(bytes > 0);
        assert!(dst.path().join("theme.toml").exists());
        assert!(dst.path().join("base.html").exists());
    }

    #[test]
    fn test_copy_theme_files_excludes_git() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();

        // Regular file
        std::fs::write(src.path().join("index.html"), "<html></html>").unwrap();
        // .git directory — should be skipped
        let git_dir = src.path().join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::write(git_dir.join("config"), "[core]").unwrap();
        // .github directory — should be skipped
        let github_dir = src.path().join(".github");
        std::fs::create_dir_all(&github_dir).unwrap();
        std::fs::write(github_dir.join("workflows.yml"), "on: push").unwrap();

        let (count, _) = copy_theme_files(src.path(), dst.path()).unwrap();
        assert_eq!(count, 1);
        assert!(!dst.path().join(".git").exists());
        assert!(!dst.path().join(".github").exists());
    }

    #[test]
    fn test_copy_theme_files_excludes_gitignore() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();

        std::fs::write(src.path().join("style.css"), "body {}").unwrap();
        std::fs::write(src.path().join(".gitignore"), "*.log").unwrap();
        std::fs::write(src.path().join(".gitattributes"), "* text=auto").unwrap();

        let (count, _) = copy_theme_files(src.path(), dst.path()).unwrap();
        assert_eq!(count, 1);
        assert!(!dst.path().join(".gitignore").exists());
        assert!(!dst.path().join(".gitattributes").exists());
    }

    #[test]
    fn test_copy_theme_files_preserves_structure() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();

        let sub = src.path().join("assets").join("css");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("style.css"), "body {}").unwrap();

        let (count, _) = copy_theme_files(src.path(), dst.path()).unwrap();
        assert_eq!(count, 1);
        assert!(dst
            .path()
            .join("assets")
            .join("css")
            .join("style.css")
            .exists());
    }

    // ── unix_secs_to_iso8601 ─────────────────────────────────────────────────

    #[test]
    fn test_unix_epoch() {
        assert_eq!(unix_secs_to_iso8601(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn test_known_timestamp() {
        // 2026-03-02T14:30:00Z = 1772228200 (pre-computed)
        // 2026-03-02 00:00:00 UTC:
        //   Days from epoch = 20514
        //   Hours: 14, Minutes: 30, Seconds: 0
        //   20514 * 86400 + 14*3600 + 30*60 = 1772409600 + 52200 = 1772461800
        assert_eq!(unix_secs_to_iso8601(1772461800), "2026-03-02T14:30:00Z");
    }

    #[test]
    fn test_leap_year_day() {
        // 2024-02-29T00:00:00Z
        // Days from epoch to 2024-01-01: 19723 days
        // Jan 2024: 31 days, Feb 1..28: 28 days, so Feb 29 = day 31+29-1 = 59 from year start
        // 2024-02-29 = day 19723 + 59 = 19782
        // secs = 19782 * 86400 = 1709164800
        assert_eq!(unix_secs_to_iso8601(1709164800), "2024-02-29T00:00:00Z");
    }

    #[test]
    fn test_current_utc_iso8601_format() {
        let ts = current_utc_iso8601();
        // Must match YYYY-MM-DDTHH:MM:SSZ
        assert_eq!(ts.len(), 20);
        assert!(ts.ends_with('Z'));
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[7..8], "-");
        assert_eq!(&ts[10..11], "T");
        assert_eq!(&ts[13..14], ":");
        assert_eq!(&ts[16..17], ":");
    }

    // ── write_provenance / read_provenance ───────────────────────────────────

    fn make_clone_result(sha: &str) -> CloneResult {
        CloneResult {
            commit_sha: sha.to_string(),
            files_copied: 5,
            total_bytes: 1024,
        }
    }

    #[test]
    fn test_write_provenance_creates_file() {
        let tmp = tempfile::tempdir().unwrap();
        let source = ThemeInstallSource {
            url: "https://github.com/user/repo.git".to_string(),
            git_ref: Some("v2.0.0".to_string()),
            path: Some("themes/garden".to_string()),
        };
        let clone = make_clone_result("abc1234def5678abcdef1234def56789abc12345");

        write_provenance(tmp.path(), &source, &clone).unwrap();

        let dest = tmp.path().join(".zetl-source.toml");
        assert!(dest.exists());
        let content = std::fs::read_to_string(&dest).unwrap();
        assert!(content.contains("url = \"https://github.com/user/repo.git\""));
        assert!(content.contains("ref = \"v2.0.0\""));
        assert!(content.contains("commit = \"abc1234def5678abcdef1234def56789abc12345\""));
        assert!(content.contains("path = \"themes/garden\""));
        assert!(content.contains("installed_at ="));
        assert!(content.contains("zetl_version ="));
    }

    #[test]
    fn test_write_provenance_no_ref_no_path() {
        let tmp = tempfile::tempdir().unwrap();
        let source = ThemeInstallSource {
            url: "https://github.com/user/repo.git".to_string(),
            git_ref: None,
            path: None,
        };
        let clone = make_clone_result("deadbeef1234567890ab");

        write_provenance(tmp.path(), &source, &clone).unwrap();

        let content = std::fs::read_to_string(tmp.path().join(".zetl-source.toml")).unwrap();
        // Optional fields must be absent when None
        assert!(!content.contains("ref ="));
        assert!(!content.contains("path ="));
    }

    #[test]
    fn test_write_then_read_provenance() {
        let tmp = tempfile::tempdir().unwrap();
        let source = ThemeInstallSource {
            url: "https://github.com/user/repo.git".to_string(),
            git_ref: Some("main".to_string()),
            path: None,
        };
        let commit = "1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b";
        let clone = make_clone_result(commit);

        write_provenance(tmp.path(), &source, &clone).unwrap();

        let record = read_provenance(tmp.path()).expect("should parse provenance");
        assert_eq!(record.source.url, "https://github.com/user/repo.git");
        assert_eq!(record.source.ref_name.as_deref(), Some("main"));
        assert_eq!(record.source.commit, commit);
        assert!(record.source.path.is_none());
        assert!(!record.source.installed_at.is_empty());
        assert!(!record.source.zetl_version.is_empty());
    }

    #[test]
    fn test_read_provenance_missing_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(read_provenance(tmp.path()).is_none());
    }

    #[test]
    fn test_read_provenance_malformed_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".zetl-source.toml"), "not valid toml ][").unwrap();
        assert!(read_provenance(tmp.path()).is_none());
    }
}
