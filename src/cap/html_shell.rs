//! Capability-mode HTML shell emission (SPEC-034 REQ-3421 / CON-3410,
//! BUG-006 resolution).
//!
//! The shell is the HTML document the browser loads when a reader
//! navigates to an encrypted page. It carries three things the shim
//! needs:
//!
//! 1. `<meta http-equiv="Content-Security-Policy" content="…">` — the
//!    exact CAP_CSP directive string, byte-for-byte identical to the
//!    HTTP response header emitted by the deploy recipes
//!    (`cap::deploy_headers`). Serving the meta fallback alongside
//!    the header is belt-and-braces: if a CDN drops the header, the
//!    browser still enforces CSP from the meta tag.
//! 2. `<script src="/assets/shim.js" integrity="sha384-…"
//!    crossorigin="anonymous"></script>` — the shim loader, SRI-
//!    pinned so a CDN-substituted shim fails integrity verification
//!    before it runs (REQ-3427 resolution for BUG-004).
//! 3. `<main data-ztl-capability></main>` — the mount point the
//!    shim's render step injects sanitised HTML into
//!    (`cap/shim/render.ts::HOST_SELECTOR`).
//!
//! The SHA-384 SRI hash is produced by the shim bundler
//! (`src/cap/shim/build.mjs`) alongside `shim.js` as a sibling
//! `shim.sri` file. The build driver reads that file at emission
//! time and pastes the hash into the shell's `integrity` attribute
//! via [`load_shim_integrity`].
//!
//! # Purity boundary (SPEC-034 §8)
//!
//! - **Pure core:** [`render_shell`] — given an SRI hash string it
//!   deterministically produces the shell HTML. Byte-stable across
//!   rebuilds.
//! - **Effectful shell:** [`load_shim_integrity`] reads
//!   `<shim_dist_dir>/shim.sri` from disk and returns the trimmed
//!   contents. Every file/directory-touching step lives here and
//!   is called once per build from the capability-build driver.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::cap::deploy_headers::{CAP_CSP, SHIM_PATH};

/// The filename the shim bundler writes the SRI hash to, next to
/// `shim.js`, inside `src/cap/shim/dist/`. Relative to the shim dist
/// directory so callers can hand in either a relative or absolute
/// path.
pub const SHIM_SRI_FILENAME: &str = "shim.sri";

/// Filename under `<out_dir>/_ztl/` where the shared HTML shell is
/// emitted. A downstream `task-cap-deploy-artifacts` change wires
/// CDN rewrites so the browser receives this file when navigating to
/// any `/c/<path-cap>/<slug>.html`. Until then, the file exists as
/// an artifact operators + CI can inspect.
pub const CAPABILITY_SHELL_FILENAME: &str = "capability-shell.html";

/// DOM selector the shim uses to find its mount point. Kept as a
/// constant so tests in both modules can cross-check against the
/// exact string.
pub const HOST_SELECTOR: &str = "main[data-ztl-capability]";

/// Document-wide referrer policy meta tag (SPEC-034 REQ-3413 /
/// OBS-3407). Pinned byte-for-byte so CI and operators can grep the
/// shell for the exact line. Complements the per-external-link
/// `rel="noopener noreferrer"` emitted by
/// [`crate::cap::referrer_scrub`]: the `<meta>` is the
/// document-default (honoured by all modern browsers) and the
/// `rel` attribute on `<a>` is the per-link override.
pub const REFERRER_META: &str = "<meta name=\"referrer\" content=\"no-referrer\">";

/// Prefix a well-formed SRI hash begins with. We deliberately only
/// support SHA-384 (per REQ-3421) — no negotiation with SHA-256 or
/// SHA-512 alternatives, so the shim bundle's hash format is a
/// single stable surface.
pub const SRI_HASH_PREFIX: &str = "sha384-";

/// Errors returned by [`load_shim_integrity`]. `Io` wraps the
/// underlying read failure; `Malformed` fires when the file exists
/// but its contents don't look like an `sha384-…` integrity token.
#[derive(Debug, thiserror::Error)]
pub enum ShellError {
    #[error("failed to read shim SRI file at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "shim SRI file at {path} did not contain an sha384 integrity token (found {got:?}); \
         rerun `node src/cap/shim/build.mjs` to regenerate it"
    )]
    Malformed { path: PathBuf, got: String },
}

/// Read `<shim_dist_dir>/shim.sri` and return the trimmed integrity
/// token (e.g. `"sha384-AbCdEf…"`). Called from the effectful shell
/// once per build.
pub fn load_shim_integrity(shim_dist_dir: &Path) -> Result<String, ShellError> {
    let path = shim_dist_dir.join(SHIM_SRI_FILENAME);
    let raw = fs::read_to_string(&path).map_err(|source| ShellError::Io {
        path: path.clone(),
        source,
    })?;
    let hash = raw.trim().to_string();
    if !hash.starts_with(SRI_HASH_PREFIX) || hash.len() == SRI_HASH_PREFIX.len() {
        return Err(ShellError::Malformed { path, got: hash });
    }
    Ok(hash)
}

/// Render the capability-mode HTML shell. Deterministic: feeding
/// the same `sri_hash` always produces byte-identical output, so
/// the shell participates cleanly in REQ-3420 build-determinism.
///
/// `sri_hash` is expected to begin with `sha384-` (validated by
/// [`load_shim_integrity`] at the caller). The argument is also
/// HTML-attribute-escaped here so a malformed token cannot break
/// out of the `integrity=""` attribute.
pub fn render_shell(sri_hash: &str) -> String {
    let mut out = String::new();
    out.push_str("<!DOCTYPE html>\n");
    out.push_str("<html lang=\"en\">\n");
    out.push_str("<head>\n");
    out.push_str("<meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    out.push_str(REFERRER_META);
    out.push('\n');
    out.push_str("<meta http-equiv=\"Content-Security-Policy\" content=\"");
    out.push_str(&attr_escape(CAP_CSP));
    out.push_str("\">\n");
    out.push_str("<title>ztl</title>\n");
    out.push_str("<script src=\"");
    out.push_str(SHIM_PATH);
    out.push_str("\" integrity=\"");
    out.push_str(&attr_escape(sri_hash));
    out.push_str("\" crossorigin=\"anonymous\"></script>\n");
    out.push_str("</head>\n");
    out.push_str("<body>\n");
    out.push_str("<main data-ztl-capability></main>\n");
    out.push_str("</body>\n");
    out.push_str("</html>\n");
    out
}

/// Write the shell to `<out_dir>/_ztl/capability-shell.html`.
/// Creates intermediate directories and overwrites an existing file
/// so rebuilds stay idempotent alongside the deploy-recipe tree.
pub fn write_shell(out_dir: &Path, sri_hash: &str) -> Result<PathBuf, io::Error> {
    let dir = out_dir.join("_ztl");
    fs::create_dir_all(&dir)?;
    let path = dir.join(CAPABILITY_SHELL_FILENAME);
    fs::write(&path, render_shell(sri_hash))?;
    Ok(path)
}

/// Minimal HTML-attribute escaping. CSP directives and SRI hashes
/// produced by the build never legitimately contain `<`, `>`, `"`,
/// `'`, or `&`, but the escape is cheap insurance against a future
/// directive gaining a character that would break out of the
/// attribute quoting.
fn attr_escape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_SRI: &str =
        "sha384-deadbeefcafef00d1234567890abcdef1234567890abcdef1234567890abcdef";

    #[test]
    fn shell_renders_doctype_and_csp_meta_byte_for_byte() {
        let html = render_shell(SAMPLE_SRI);
        assert!(html.starts_with("<!DOCTYPE html>\n"));
        // CSP meta fallback carries the CAP_CSP directive with `'`
        // HTML-attribute-escaped to `&#39;` — the CSP parser treats
        // the escaped form as equivalent per the CSP §3.1 grammar
        // (HTML unescapes the attribute before handoff).
        let escaped = CAP_CSP.replace('\'', "&#39;");
        let expected_meta =
            format!("<meta http-equiv=\"Content-Security-Policy\" content=\"{escaped}\">");
        assert!(
            html.contains(&expected_meta),
            "missing CSP meta tag, got:\n{html}"
        );
    }

    #[test]
    fn shell_includes_shim_script_with_sri_and_crossorigin() {
        let html = render_shell(SAMPLE_SRI);
        let expected = format!(
            "<script src=\"{SHIM_PATH}\" integrity=\"{SAMPLE_SRI}\" crossorigin=\"anonymous\"></script>"
        );
        assert!(
            html.contains(&expected),
            "missing SRI-tagged shim script, got:\n{html}"
        );
    }

    #[test]
    fn shell_contains_shim_mount_point() {
        let html = render_shell(SAMPLE_SRI);
        assert!(
            html.contains("<main data-ztl-capability></main>"),
            "missing capability mount point, got:\n{html}"
        );
    }

    #[test]
    fn shell_is_deterministic() {
        assert_eq!(render_shell(SAMPLE_SRI), render_shell(SAMPLE_SRI));
    }

    #[test]
    fn shell_emits_referrer_no_referrer_meta() {
        // SPEC-034 REQ-3413 document-wide referrer policy. Paired
        // with per-link `rel="noopener noreferrer"` in
        // `cap::referrer_scrub` — belt-and-braces defence against
        // the path-cap leaking into outgoing Referer headers.
        let html = render_shell(SAMPLE_SRI);
        assert!(
            html.contains(REFERRER_META),
            "missing referrer meta, got:\n{html}"
        );
        // Byte-pinned shape so CI greps stay stable.
        assert_eq!(
            REFERRER_META,
            "<meta name=\"referrer\" content=\"no-referrer\">"
        );
    }

    #[test]
    fn attr_escape_covers_csp_directive() {
        // The CAP_CSP string contains `'` — verify the escape
        // survives into the meta value so a downstream linter (or
        // innerHTML sniffer) cannot close the attribute early.
        let html = render_shell(SAMPLE_SRI);
        assert!(
            html.contains("&#39;none&#39;"),
            "expected single quotes to be escaped in attribute, got:\n{html}"
        );
    }

    #[test]
    fn load_shim_integrity_trims_newline_and_returns_hash() {
        let tmp = tempfile::TempDir::new().unwrap();
        fs::write(
            tmp.path().join(SHIM_SRI_FILENAME),
            format!("{SAMPLE_SRI}\n"),
        )
        .unwrap();
        let hash = load_shim_integrity(tmp.path()).unwrap();
        assert_eq!(hash, SAMPLE_SRI);
    }

    #[test]
    fn load_shim_integrity_rejects_empty_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        fs::write(tmp.path().join(SHIM_SRI_FILENAME), "").unwrap();
        let err = load_shim_integrity(tmp.path()).unwrap_err();
        assert!(matches!(err, ShellError::Malformed { .. }));
    }

    #[test]
    fn load_shim_integrity_rejects_non_sha384_prefix() {
        let tmp = tempfile::TempDir::new().unwrap();
        fs::write(tmp.path().join(SHIM_SRI_FILENAME), "sha256-0000\n").unwrap();
        let err = load_shim_integrity(tmp.path()).unwrap_err();
        assert!(matches!(err, ShellError::Malformed { .. }));
    }

    #[test]
    fn load_shim_integrity_surfaces_missing_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let err = load_shim_integrity(tmp.path()).unwrap_err();
        assert!(matches!(err, ShellError::Io { .. }));
    }

    #[test]
    fn write_shell_is_idempotent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p1 = write_shell(tmp.path(), SAMPLE_SRI).unwrap();
        let b1 = fs::read_to_string(&p1).unwrap();
        let p2 = write_shell(tmp.path(), SAMPLE_SRI).unwrap();
        let b2 = fs::read_to_string(&p2).unwrap();
        assert_eq!(p1, p2);
        assert_eq!(b1, b2);
        assert!(b1.contains("<main data-ztl-capability>"));
    }

    #[test]
    fn host_selector_matches_shim_constant() {
        // The shim's `render.ts` hardcodes `main[data-ztl-capability]`
        // as its host selector. If this selector ever diverges the
        // shell will render but the shim will fail with `host-missing`,
        // so the constant is pinned here as a regression gate.
        assert_eq!(HOST_SELECTOR, "main[data-ztl-capability]");
    }
}
