//! Hardened-mode reader self-enrolment page (SPEC-034 REQ-3404 /
//! REQ-3414, CON-3409).
//!
//! This module owns the pure-core bits of `/enroll.html`: the per-
//! cohort PRF salt SHA-256 derivation (REQ-3414) and the static HTML
//! shell the reader's browser navigates to. The effectful shell is
//! the bundled JS (`src/cap/shim/enroll.ts`) loaded via
//! `/assets/enroll.js` and SRI-pinned the same way
//! `/assets/shim.js` is (REQ-3421 / CON-3410).
//!
//! # Wire layout
//!
//! The page expects a `?cohort=<cohort-id>` query parameter. The
//! browser then:
//!
//! 1. Computes `prf_salt = SHA-256("ztl/webauthn-prf/v1/" || origin
//!    || "/" || cohort_id)` — the string in [`PRF_SALT_PREFIX`]
//!    followed by the concrete origin and cohort id. The Rust half
//!    of that derivation lives in [`compute_prf_salt`] so Rust and
//!    browser code can be cross-checked byte-for-byte in tests.
//! 2. Calls `navigator.credentials.create()` with the salt in
//!    `extensions.prf.eval.first`.
//! 3. Derives the reader's long-term X25519 identity from the PRF
//!    output via typage's PRF-recipient helpers and renders the
//!    public half as `age-recipient-v1:<b64url>` (REQ-3409) with
//!    copy and QR affordances.
//!
//! No network traffic to ztl endpoints is required at runtime:
//! once `/enroll.html` + `/assets/enroll.js` are fetched, the whole
//! flow runs locally and the reader sends the resulting pubkey to
//! the operator out of band.
//!
//! # Purity boundary (SPEC-034 §8)
//!
//! - **Pure core:** [`compute_prf_salt`] (no I/O, deterministic
//!   SHA-256) and [`render_enroll_html`] (string formatting only,
//!   byte-stable across rebuilds for a given SRI token).
//! - **Effectful shell:** [`write_enroll_html`] — writes the
//!   rendered page under `<out_dir>/enroll.html`. Idempotent
//!   overwrite.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::cap::deploy_headers::CAP_CSP;

/// REQ-3414 PRF-salt-prefix literal. Pinned exactly as the spec
/// spells it — a change here is a wire-format break that would
/// cause every hardened-cohort enrolment to derive a different
/// pubkey. Leading + trailing slashes included.
pub const PRF_SALT_PREFIX: &str = "ztl/webauthn-prf/v1/";

/// Filename the capability build writes the enrolment page to,
/// relative to `<out_dir>`. Paired with the deploy recipes in
/// `cap::deploy_headers` which configure `Clear-Site-Data` on
/// `/enroll.html` (REQ-3428 / BUG-008).
pub const ENROLL_HTML_FILENAME: &str = "enroll.html";

/// Absolute URL path the enrolment page loads its JS bundle from.
/// Mirrors [`crate::cap::deploy_headers::SHIM_PATH`] — both live
/// under `/assets/` so a single static-host config covers them.
pub const ENROLL_JS_PATH: &str = "/assets/enroll.js";

/// DOM element id the bundled JS mounts into. Kept as a constant
/// so the Rust shell and the TS runtime can cross-check the
/// selector.
pub const ENROLL_MOUNT_ID: &str = "ztl-enroll";

/// Filename the shim bundler writes the enrolment-bundle SRI hash
/// to, next to `enroll.js`, inside `src/cap/shim/dist/`. Paired
/// with [`crate::cap::html_shell::SHIM_SRI_FILENAME`].
pub const ENROLL_SRI_FILENAME: &str = "enroll.sri";

/// Prefix a well-formed SRI hash begins with. We only support
/// SHA-384 (per REQ-3421) — identical policy to
/// [`crate::cap::html_shell::SRI_HASH_PREFIX`].
pub const SRI_HASH_PREFIX: &str = "sha384-";

/// Errors returned by [`load_enroll_integrity`].
#[derive(Debug, thiserror::Error)]
pub enum EnrolmentError {
    #[error("failed to read enrolment SRI file at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "enrolment SRI file at {path} did not contain an sha384 integrity token \
         (found {got:?}); rerun `node src/cap/shim/build.mjs` to regenerate it"
    )]
    Malformed { path: PathBuf, got: String },
}

/// Read `<shim_dist_dir>/enroll.sri` and return the trimmed
/// integrity token (e.g. `"sha384-AbCdEf…"`). Called from the
/// effectful shell once per build, mirroring
/// [`crate::cap::html_shell::load_shim_integrity`].
pub fn load_enroll_integrity(shim_dist_dir: &Path) -> Result<String, EnrolmentError> {
    let path = shim_dist_dir.join(ENROLL_SRI_FILENAME);
    let raw = fs::read_to_string(&path).map_err(|source| EnrolmentError::Io {
        path: path.clone(),
        source,
    })?;
    let hash = raw.trim().to_string();
    if !hash.starts_with(SRI_HASH_PREFIX) || hash.len() == SRI_HASH_PREFIX.len() {
        return Err(EnrolmentError::Malformed { path, got: hash });
    }
    Ok(hash)
}

/// Compute the REQ-3414 per-cohort PRF salt.
///
/// `prf_salt = SHA-256("ztl/webauthn-prf/v1/" || origin || "/" ||
/// cohort_id)`
///
/// `origin` is the browser-visible origin (`scheme://host[:port]`)
/// exactly as `window.location.origin` would produce it; the
/// browser-side JS passes the live origin through to `SubtleCrypto`
/// and this Rust half is used for cross-check tests (TEST-3414).
///
/// Consequence (BUG-003): a reader in two hardened cohorts produces
/// two distinct salts → two distinct PRF outputs → two distinct
/// X25519 pubkeys, so ciphertext observers cannot link recipient
/// entries across cohorts.
pub fn compute_prf_salt(origin: &str, cohort_id: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(PRF_SALT_PREFIX.as_bytes());
    hasher.update(origin.as_bytes());
    hasher.update(b"/");
    hasher.update(cohort_id.as_bytes());
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

/// Render `/enroll.html` as a deterministic UTF-8 string.
///
/// `enroll_js_sri` is the SHA-384 SRI integrity token the shim
/// bundler emits alongside `enroll.js` (e.g. `"sha384-AbCd…"`). It
/// is HTML-attribute-escaped on insert so a malformed token cannot
/// break out of the `integrity=""` attribute.
///
/// The CSP meta fallback carries the exact [`CAP_CSP`] directive so
/// the page stays protected even if a CDN drops the
/// `Content-Security-Policy` HTTP header. Every surface the reader
/// sees (title, diagnostic panes, copy/QR affordances) renders from
/// the bundled JS — the static HTML here is intentionally minimal
/// so the page is searchable and explainable by reading the shim
/// source alone.
pub fn render_enroll_html(enroll_js_sri: &str) -> String {
    let mut out = String::new();
    out.push_str("<!DOCTYPE html>\n");
    out.push_str("<html lang=\"en\">\n");
    out.push_str("<head>\n");
    out.push_str("<meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    out.push_str("<meta name=\"referrer\" content=\"no-referrer\">\n");
    out.push_str("<meta http-equiv=\"Content-Security-Policy\" content=\"");
    out.push_str(&attr_escape(CAP_CSP));
    out.push_str("\">\n");
    out.push_str("<title>ztl — enrol</title>\n");
    out.push_str("<link rel=\"stylesheet\" href=\"/assets/enroll.css\">\n");
    out.push_str("<script defer src=\"");
    out.push_str(ENROLL_JS_PATH);
    out.push_str("\" integrity=\"");
    out.push_str(&attr_escape(enroll_js_sri));
    out.push_str("\" crossorigin=\"anonymous\"></script>\n");
    out.push_str("</head>\n");
    out.push_str("<body>\n");
    out.push_str("<main id=\"");
    out.push_str(ENROLL_MOUNT_ID);
    out.push_str("\" data-ztl-enroll>\n");
    out.push_str("<h1>ztl — hardened-mode enrolment</h1>\n");
    out.push_str("<noscript>\n");
    out.push_str(
        "<p>This page needs JavaScript enabled to create a \
                 passkey and derive your cohort-scoped public key.</p>\n",
    );
    out.push_str("</noscript>\n");
    out.push_str("<section data-state=\"loading\"><p>Loading enrolment flow…</p></section>\n");
    out.push_str("</main>\n");
    out.push_str("</body>\n");
    out.push_str("</html>\n");
    out
}

/// Write the rendered enrolment page to
/// `<out_dir>/enroll.html`. Overwrites any existing file so
/// successive rebuilds stay idempotent alongside the shared HTML
/// shell and deploy recipes.
pub fn write_enroll_html(out_dir: &Path, enroll_js_sri: &str) -> Result<PathBuf, io::Error> {
    fs::create_dir_all(out_dir)?;
    let path = out_dir.join(ENROLL_HTML_FILENAME);
    fs::write(&path, render_enroll_html(enroll_js_sri))?;
    Ok(path)
}

/// Minimal HTML-attribute escaping. Mirrors `cap::html_shell` so
/// the exact same escaping applies to the CSP directive and the
/// SRI token in both emission surfaces.
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
    fn prf_salt_matches_spec_formula() {
        // Spec spells the formula as
        //     SHA-256("ztl/webauthn-prf/v1/" || origin || "/" ||
        //             cohort_id)
        // so we recompute the literal bytes here and compare.
        let origin = "https://example.org";
        let cohort = "engineering";
        let mut reference = Sha256::new();
        reference.update(b"ztl/webauthn-prf/v1/");
        reference.update(origin.as_bytes());
        reference.update(b"/");
        reference.update(cohort.as_bytes());
        let expected = reference.finalize();

        let got = compute_prf_salt(origin, cohort);
        assert_eq!(&got[..], expected.as_slice());
    }

    #[test]
    fn prf_salt_prefix_is_pinned() {
        // Byte-stable surface — a change here is a wire-format
        // break that invalidates every prior hardened-cohort
        // enrolment.
        assert_eq!(PRF_SALT_PREFIX, "ztl/webauthn-prf/v1/");
    }

    #[test]
    fn prf_salt_differs_across_cohorts_same_origin() {
        // REQ-3414 cross-cohort unlinkability (BUG-003 fix): the
        // same reader enrolling in two cohorts on the same origin
        // must derive distinct salts → distinct PRF outputs →
        // distinct pubkeys.
        let origin = "https://example.org";
        let a = compute_prf_salt(origin, "engineering");
        let b = compute_prf_salt(origin, "ops");
        assert_ne!(a, b);
    }

    #[test]
    fn prf_salt_differs_across_origins_same_cohort() {
        // Different origins must also produce different salts so
        // an attacker who enrols the same cohort id at their own
        // domain cannot obtain a linkable output.
        let cohort = "engineering";
        let a = compute_prf_salt("https://example.org", cohort);
        let b = compute_prf_salt("https://attacker.example", cohort);
        assert_ne!(a, b);
    }

    #[test]
    fn prf_salt_is_32_bytes_long() {
        let salt = compute_prf_salt("https://example.org", "eng");
        assert_eq!(salt.len(), 32);
    }

    #[test]
    fn prf_salt_handles_empty_cohort_id() {
        // Empty cohort id is a pathological input (the CLI
        // rejects it upstream), but the salt function itself
        // must be total — returning SHA-256 of the
        // prefix||origin||"/"||"" without panicking.
        let _ = compute_prf_salt("https://example.org", "");
    }

    #[test]
    fn prf_salt_is_slash_delimited_not_concatenated() {
        // Guard against a refactor that drops the `/` delimiter:
        // with the delimiter, origin="http://a" + cohort="bc" and
        // origin="http://abc" + cohort="" must produce different
        // salts (they encode different strings).
        let with_delim = compute_prf_salt("http://a", "bc");
        let collapsed = compute_prf_salt("http://abc", "");
        assert_ne!(with_delim, collapsed);
    }

    #[test]
    fn enroll_html_is_deterministic() {
        assert_eq!(
            render_enroll_html(SAMPLE_SRI),
            render_enroll_html(SAMPLE_SRI)
        );
    }

    #[test]
    fn enroll_html_starts_with_doctype() {
        let html = render_enroll_html(SAMPLE_SRI);
        assert!(html.starts_with("<!DOCTYPE html>\n"), "got:\n{html}");
    }

    #[test]
    fn enroll_html_carries_csp_meta_fallback() {
        let html = render_enroll_html(SAMPLE_SRI);
        let escaped = CAP_CSP.replace('\'', "&#39;");
        let expected_meta =
            format!("<meta http-equiv=\"Content-Security-Policy\" content=\"{escaped}\">");
        assert!(
            html.contains(&expected_meta),
            "missing CSP meta fallback, got:\n{html}"
        );
    }

    #[test]
    fn enroll_html_carries_no_referrer_meta() {
        let html = render_enroll_html(SAMPLE_SRI);
        assert!(
            html.contains("<meta name=\"referrer\" content=\"no-referrer\">"),
            "missing no-referrer meta, got:\n{html}"
        );
    }

    #[test]
    fn enroll_html_has_sri_tagged_enroll_script() {
        let html = render_enroll_html(SAMPLE_SRI);
        let expected = format!(
            "<script defer src=\"{ENROLL_JS_PATH}\" integrity=\"{SAMPLE_SRI}\" \
             crossorigin=\"anonymous\"></script>"
        );
        assert!(
            html.contains(&expected),
            "missing SRI-tagged enroll script, got:\n{html}"
        );
    }

    #[test]
    fn enroll_html_mounts_to_pinned_id() {
        let html = render_enroll_html(SAMPLE_SRI);
        let expected = format!("<main id=\"{ENROLL_MOUNT_ID}\" data-ztl-enroll>");
        assert!(
            html.contains(&expected),
            "missing mount point #{ENROLL_MOUNT_ID}, got:\n{html}"
        );
    }

    #[test]
    fn enroll_html_includes_noscript_diagnostic() {
        let html = render_enroll_html(SAMPLE_SRI);
        assert!(
            html.contains("<noscript>") && html.contains("</noscript>"),
            "missing noscript diagnostic for JS-disabled browsers, got:\n{html}"
        );
    }

    #[test]
    fn attr_escape_handles_quote_and_ampersand() {
        assert_eq!(attr_escape("a&b"), "a&amp;b");
        assert_eq!(attr_escape("a\"b"), "a&quot;b");
        assert_eq!(attr_escape("a'b"), "a&#39;b");
        assert_eq!(attr_escape("a<b>c"), "a&lt;b&gt;c");
    }

    #[test]
    fn write_enroll_html_is_idempotent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p1 = write_enroll_html(tmp.path(), SAMPLE_SRI).unwrap();
        let b1 = fs::read_to_string(&p1).unwrap();
        let p2 = write_enroll_html(tmp.path(), SAMPLE_SRI).unwrap();
        let b2 = fs::read_to_string(&p2).unwrap();
        assert_eq!(p1, p2);
        assert_eq!(b1, b2);
        assert!(b1.starts_with("<!DOCTYPE html>\n"));
    }

    #[test]
    fn write_enroll_html_creates_missing_out_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let nested = tmp.path().join("a").join("b");
        let path = write_enroll_html(&nested, SAMPLE_SRI).unwrap();
        assert!(path.exists());
        assert_eq!(path.parent().unwrap(), nested.as_path());
    }

    #[test]
    fn enroll_js_path_is_under_assets() {
        // Deploy recipes assume `/assets/*` is the static-asset
        // prefix (see `cap::deploy_headers::SHIM_PATH`). Pin the
        // enroll.js path here so a rename is caught by a failing
        // test rather than a silently-broken deploy.
        assert!(
            ENROLL_JS_PATH.starts_with("/assets/"),
            "enroll JS path should live under /assets/, got: {ENROLL_JS_PATH}"
        );
    }

    #[test]
    fn load_enroll_integrity_trims_newline() {
        let tmp = tempfile::TempDir::new().unwrap();
        fs::write(
            tmp.path().join(ENROLL_SRI_FILENAME),
            format!("{SAMPLE_SRI}\n"),
        )
        .unwrap();
        let hash = load_enroll_integrity(tmp.path()).unwrap();
        assert_eq!(hash, SAMPLE_SRI);
    }

    #[test]
    fn load_enroll_integrity_rejects_empty_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        fs::write(tmp.path().join(ENROLL_SRI_FILENAME), "").unwrap();
        let err = load_enroll_integrity(tmp.path()).unwrap_err();
        assert!(matches!(err, EnrolmentError::Malformed { .. }));
    }

    #[test]
    fn load_enroll_integrity_rejects_non_sha384_prefix() {
        let tmp = tempfile::TempDir::new().unwrap();
        fs::write(tmp.path().join(ENROLL_SRI_FILENAME), "sha256-0000\n").unwrap();
        let err = load_enroll_integrity(tmp.path()).unwrap_err();
        assert!(matches!(err, EnrolmentError::Malformed { .. }));
    }

    #[test]
    fn load_enroll_integrity_surfaces_missing_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let err = load_enroll_integrity(tmp.path()).unwrap_err();
        assert!(matches!(err, EnrolmentError::Io { .. }));
    }

    #[test]
    fn enroll_html_filename_matches_clear_site_data_path() {
        // REQ-3428 / BUG-008: deploy recipes emit
        // `Clear-Site-Data` on `/enroll.html`. The emitted file's
        // relative path under `<out_dir>` must therefore be
        // exactly `enroll.html`.
        assert_eq!(ENROLL_HTML_FILENAME, "enroll.html");
    }
}
