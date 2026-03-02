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
    if source.starts_with("https://") || source.starts_with("http://") {
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
            let url = format!("https://github.com/{}/{}.git", user, repo);
            return Ok(ThemeInstallSource {
                url,
                git_ref: git_ref.map(str::to_string),
                path: None,
            });
        }
    }

    bail!(
        "unrecognized source {:?}: expected 'user/repo', 'https://...', or 'git@...' \
         (optionally with '#ref')",
        source
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
                    format!("cannot derive a valid theme name from path component {:?}", last)
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
    #[serde(default)]
    pub ref_name: Option<String>,
    pub commit: String,
    #[serde(default)]
    pub path: Option<String>,
    pub installed_at: String,
    pub zetl_version: String,
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Validate that `name` matches the pattern `^[a-z0-9][a-z0-9_-]*$`.
///
/// Returns a descriptive error on failure.
pub fn validate_theme_name(name: &str) -> Result<()> {
    let re = Regex::new(r"^[a-z0-9][a-z0-9_-]*$").unwrap();
    if re.is_match(name) {
        Ok(())
    } else {
        bail!(
            "invalid theme name {:?}: must match ^[a-z0-9][a-z0-9_-]*$ \
             (lowercase letters, digits, hyphens, underscores; must start with a letter or digit)",
            name
        )
    }
}

/// Parse TOML content into a [`ThemeManifest`].
///
/// Validates that:
/// - `theme.name` matches `^[a-z0-9][a-z0-9_-]*$`
/// - `theme.version` is a valid SemVer string (e.g. `"1.0.0"`)
pub fn parse_theme_manifest(content: &str) -> Result<ThemeManifest> {
    let manifest: ThemeManifest =
        toml::from_str(content).context("failed to parse theme.toml")?;

    validate_theme_name(&manifest.theme.name)
        .with_context(|| "theme.toml: invalid name field")?;

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
    let manifest = parse_theme_manifest(content).with_context(|| {
        format!(
            "malformed bundled theme manifest for {:?}",
            theme_name
        )
    })?;
    Ok(Some(manifest))
}

/// Parse TOML content into a [`ThemeSource`] provenance record.
pub fn parse_theme_source(content: &str) -> Result<ThemeSource> {
    toml::from_str(content).context("failed to parse .zetl-source.toml")
}

// ── Internal helpers ─────────────────────────────────────────────────────────

/// Split a source string on the first `#`, returning `(base, Some(ref))` or
/// `(source, None)` if no `#` is present.
fn split_ref(s: &str) -> (&str, Option<&str>) {
    match s.find('#') {
        Some(pos) => {
            let git_ref = &s[pos + 1..];
            (&s[..pos], if git_ref.is_empty() { None } else { Some(git_ref) })
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
    let last = url
        .rsplit(|c| c == '/' || c == ':')
        .find(|s| !s.is_empty())?;
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
            "version {:?} is not valid SemVer (expected MAJOR.MINOR.PATCH, e.g. \"1.0.0\")",
            version
        )
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── validate_theme_name ──────────────────────────────────────────────────

    #[test]
    fn test_valid_theme_names() {
        for name in &["default", "my-theme", "theme2", "a", "0", "dark_mode", "a1-b2"] {
            assert!(
                validate_theme_name(name).is_ok(),
                "expected {:?} to be valid",
                name
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
                "expected {:?} to be invalid",
                name
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
                "expected {:?} to be valid semver",
                v
            );
        }
    }

    #[test]
    fn test_invalid_semver() {
        for v in &["1.0", "1", "v1.0.0", "1.0.0.0", "latest", ""] {
            assert!(
                validate_semver(v).is_err(),
                "expected {:?} to be invalid semver",
                v
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
        assert_eq!(m.theme.description.as_deref(), Some("The default zetl theme"));
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
                "expected {:?} to be rejected",
                bad
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
        let name =
            resolve_theme_name(Some("flag-name"), Some(&manifest), Some("themes/path"), &src)
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
ref_name = "main"
commit = "abc123"
path = "themes/dark"
installed_at = "2024-06-15T12:00:00Z"
zetl_version = "0.2.0"
"#;
        let s = parse_theme_source(toml).unwrap();
        assert_eq!(s.source.ref_name.as_deref(), Some("main"));
        assert_eq!(s.source.path.as_deref(), Some("themes/dark"));
    }
}
