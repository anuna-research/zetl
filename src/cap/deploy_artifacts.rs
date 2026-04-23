//! Deploy artifacts — top-level static-host wiring
//! (SPEC-034 REQ-3418 / CON-3406; task-cap-deploy-artifacts).
//!
//! This module is the sibling of [`crate::cap::deploy_headers`] that
//! emits *operator-facing* artifacts into the dist tree — the files a
//! static host actually reads at serve time, as opposed to the in-repo
//! snippets an operator copy-pastes into their own config. Layout:
//!
//! ```text
//! <out_dir>/
//!   _redirects                         ← Netlify / Cloudflare Pages redirects
//!   vercel.json                        ← Vercel rewrites + headers
//!   <vault>-<cohort>.html              ← optional single-file bundle
//!   _ztl/
//!     _gone.map                        ← nginx map for 410-Gone tombstones
//!     deploy-nginx.conf                ← complete nginx recipe (headers + _gone.map include)
//!     deploy-caddy.conf                ← complete Caddy recipe
//!     deploy-netlify.conf              ← Netlify _headers-style recipe
//!     deploy-vercel.conf               ← Vercel JSON recipe
//!     deploy-cloudflare.conf           ← Cloudflare Pages _headers-style recipe
//! ```
//!
//! The top-level `_redirects` + `vercel.json` are *in addition to* the
//! copy-paste recipe under `_ztl/`; Netlify and Vercel both read the
//! root-level files verbatim, so shipping them in the dist tree means
//! the operator does not have to manually merge anything to get the
//! baseline Cache-Control + Clear-Site-Data + tombstone wiring.
//!
//! **Tombstones.** `_gone.map` / `_redirects` / `vercel.json` each
//! carry the same list of `/c/<path-cap>/<slug>.html` paths that the
//! operator has explicitly retired (a follow-up task computes these
//! from rotated-salt history; in v1 the list is driver-supplied and
//! defaults to empty). Emitting the scaffold now means the path-cap
//! retirement flow lands as a data-only change.
//!
//! **Single-file bundle.** When `[access.single_file] enabled = true`
//! each cohort gets a `<out>/<vault>-<cohort>.html` companion file
//! that inlines its envelopes as base64-encoded `<template>` blocks
//! keyed by slug. The loader that wires these back to the decryption
//! shim lives in a downstream task; the scaffold is emitted now so
//! the config field is testable end-to-end.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD as B64_STANDARD;
use base64::Engine as _;

use crate::cap::deploy_headers::{
    HeaderSpec, CIPHERTEXT_PATH_PREFIX, CLEAR_SITE_DATA_PATHS, SHIM_PATH,
};
use crate::cap::scoping::access_config::SingleFileConfig;

/// Generated-file marker that every deploy artifact carries at the top
/// of its body. Tests grep for this to assert the file came from the
/// build driver and not an operator-maintained override.
pub const GENERATED_MARKER: &str = "ztl capability-mode deploy artifact";

/// Nginx variable that [`render_gone_map`] writes into and the
/// deploy-nginx recipe reads back.
pub const GONE_MAP_VARIABLE: &str = "$ztl_gone";

/// One envelope inside a [`CohortBundle`] — `(slug, bytes)`. The
/// bundle renderer base64-encodes the bytes verbatim so the envelope
/// round-trips through the scaffold without structural transformation.
#[derive(Debug, Clone)]
pub struct BundledEnvelope {
    pub slug: String,
    pub envelope_bytes: Vec<u8>,
}

/// Single-file bundle input — one per cohort.
#[derive(Debug, Clone)]
pub struct CohortBundle {
    pub cohort_id: String,
    pub envelopes: Vec<BundledEnvelope>,
}

/// Everything [`write_deploy_artifacts`] needs. Grouped into a struct
/// so the driver can thread new knobs through without breaking call
/// sites.
#[derive(Debug, Clone)]
pub struct DeployArtifactsInput<'a> {
    pub spec: &'a HeaderSpec,
    /// Paths like `/c/<path-cap>/<slug>.html` that should 410-Gone.
    /// Empty is a valid value — the scaffold still lands so operators
    /// know where to extend later.
    pub tombstones: Vec<String>,
    /// Vault name used to key the single-file bundle filename. Safe
    /// to pass `""` when single-file bundles are disabled.
    pub vault_name: &'a str,
    /// Opt-in single-file bundle config (`[access.single_file]`).
    pub single_file: &'a SingleFileConfig,
    /// Cohort bundles to emit. Ignored when `single_file.enabled`
    /// is `false`.
    pub cohort_bundles: Vec<CohortBundle>,
}

/// Render `<out_dir>/_ztl/_gone.map` — an nginx `map` block that
/// maps tombstone paths to a flag variable the server config inspects
/// to return 410 Gone.
pub fn render_gone_map(tombstones: &[String]) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {GENERATED_MARKER}: nginx tombstone map.\n"));
    out.push_str("# Include from an `http {}` context, then have\n");
    out.push_str("# `if (");
    out.push_str(GONE_MAP_VARIABLE);
    out.push_str(" = 1) { return 410; }` in your `/c/` location.\n");
    out.push_str(&format!("map $request_uri {GONE_MAP_VARIABLE} {{\n"));
    out.push_str("    default 0;\n");
    for path in tombstones {
        out.push_str("    ");
        out.push_str(&nginx_quote(path));
        out.push_str(" 1;\n");
    }
    out.push_str("}\n");
    out
}

/// Render the top-level Netlify / Cloudflare Pages `_redirects` file.
/// Each tombstone becomes a `/path 410!` line; the leading comment
/// carries the generated-file marker so tests + operators can tell it
/// apart from a hand-curated overlay.
pub fn render_netlify_redirects(tombstones: &[String]) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {GENERATED_MARKER}: Netlify redirects.\n"));
    out.push_str("# Tombstones emit HTTP 410 Gone; the shim surfaces a matching\n");
    out.push_str("# \"this capability has been revoked\" error page client-side.\n");
    if tombstones.is_empty() {
        out.push_str("# (No tombstones emitted — append `/c/<path-cap>/<slug>.html 410!`\n");
        out.push_str("#  lines when retiring a capability.)\n");
    }
    for path in tombstones {
        out.push_str(path);
        out.push_str(" 410!\n");
    }
    out
}

/// Render the top-level `vercel.json`. Combines (a) the header rules
/// from [`HeaderSpec`] with (b) a `redirects` array carrying every
/// tombstone path at `statusCode: 410`. Vercel's schema does not have
/// a dedicated "gone" verb; `statusCode: 410` on a `redirects` rule
/// is the idiomatic substitute.
pub fn render_top_vercel_json(spec: &HeaderSpec, tombstones: &[String]) -> String {
    let mut headers = String::new();
    headers.push_str("  \"headers\": [\n");
    headers.push_str(&vercel_header_entry(
        &format!("{CIPHERTEXT_PATH_PREFIX}(.*)"),
        &[
            ("Cache-Control", &spec.cap_cache_control),
            ("Content-Security-Policy", &spec.csp),
        ],
    ));
    headers.push_str(",\n");
    for path in CLEAR_SITE_DATA_PATHS {
        let pairs: Vec<(&str, &str)> = if *path == "/enroll.html" {
            vec![
                ("Clear-Site-Data", spec.clear_site_data.as_str()),
                ("Content-Security-Policy", spec.csp.as_str()),
            ]
        } else {
            vec![("Clear-Site-Data", spec.clear_site_data.as_str())]
        };
        headers.push_str(&vercel_header_entry(path, &pairs));
        headers.push_str(",\n");
    }
    headers.push_str(&vercel_header_entry(
        SHIM_PATH,
        &[("Cache-Control", &spec.shim_cache_control)],
    ));
    headers.push_str("\n  ]");

    let mut redirects = String::new();
    redirects.push_str("  \"redirects\": [");
    if tombstones.is_empty() {
        redirects.push(']');
    } else {
        redirects.push('\n');
        for (i, path) in tombstones.iter().enumerate() {
            redirects.push_str("    { \"source\": ");
            redirects.push_str(&json_string(path));
            redirects.push_str(", \"destination\": ");
            redirects.push_str(&json_string(path));
            redirects.push_str(", \"statusCode\": 410 }");
            if i + 1 < tombstones.len() {
                redirects.push(',');
            }
            redirects.push('\n');
        }
        redirects.push_str("  ]");
    }

    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&headers);
    out.push_str(",\n");
    out.push_str(&redirects);
    out.push_str("\n}\n");
    out
}

/// Render `<out_dir>/_ztl/deploy-nginx.conf` — the complete nginx
/// recipe an operator can copy-paste into a `server { }` block. Pulls
/// the header directives from [`HeaderSpec`] and references the
/// `_gone.map` file by path so tombstones flow through automatically.
pub fn render_deploy_nginx(spec: &HeaderSpec, tombstones: &[String]) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {GENERATED_MARKER}: nginx.\n"));
    out.push_str("# Paste inside an nginx `server { }` block. Also include\n");
    out.push_str("# `include _ztl/_gone.map;` in your enclosing `http {}` context.\n");
    out.push_str(&format!(
        "location ^~ {CIPHERTEXT_PATH_PREFIX} {{\n    \
         if ({GONE_MAP_VARIABLE} = 1) {{ return 410; }}\n    \
         add_header Cache-Control \"{}\" always;\n    \
         add_header Content-Security-Policy \"{}\" always;\n}}\n\n",
        spec.cap_cache_control, spec.csp
    ));
    for path in CLEAR_SITE_DATA_PATHS {
        out.push_str(&format!(
            "location = {path} {{\n    \
             add_header Clear-Site-Data '{}' always;\n",
            spec.clear_site_data
        ));
        if *path == "/enroll.html" {
            out.push_str(&format!(
                "    add_header Content-Security-Policy \"{}\" always;\n",
                spec.csp
            ));
        }
        out.push_str("}\n\n");
    }
    out.push_str(&format!(
        "location = {SHIM_PATH} {{\n    \
         add_header Cache-Control \"{}\" always;\n}}\n",
        spec.shim_cache_control
    ));
    if !tombstones.is_empty() {
        out.push_str("\n# Known tombstones (also listed in _gone.map):\n");
        for path in tombstones {
            out.push_str("#   ");
            out.push_str(path);
            out.push('\n');
        }
    }
    out
}

/// Render `<out_dir>/_ztl/deploy-caddy.conf`.
pub fn render_deploy_caddy(spec: &HeaderSpec, tombstones: &[String]) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {GENERATED_MARKER}: Caddy.\n"));
    out.push_str("# Paste inside your site block.\n\n");
    out.push_str(&format!(
        "@ztl_cap path {CIPHERTEXT_PATH_PREFIX}*\n\
         header @ztl_cap Cache-Control \"{}\"\n\
         header @ztl_cap Content-Security-Policy \"{}\"\n\n",
        spec.cap_cache_control, spec.csp
    ));
    for path in tombstones {
        out.push_str(&format!(
            "@ztl_gone_{hash} path {path}\nrespond @ztl_gone_{hash} 410\n\n",
            hash = short_hash(path),
        ));
    }
    for (idx, path) in CLEAR_SITE_DATA_PATHS.iter().enumerate() {
        let matcher = format!("@ztl_csd_{idx}");
        out.push_str(&format!(
            "{matcher} path {path}\nheader {matcher} Clear-Site-Data `{}`\n",
            spec.clear_site_data
        ));
        if *path == "/enroll.html" {
            out.push_str(&format!(
                "header {matcher} Content-Security-Policy \"{}\"\n",
                spec.csp
            ));
        }
        out.push('\n');
    }
    out.push_str(&format!(
        "@ztl_shim path {SHIM_PATH}\nheader @ztl_shim Cache-Control \"{}\"\n",
        spec.shim_cache_control
    ));
    out
}

/// Render `<out_dir>/_ztl/deploy-netlify.conf` — same format as the
/// Netlify `_headers` file, emitted under `_ztl/` so the operator
/// treats it as a copy-paste recipe. The top-level `_redirects` owns
/// the tombstone list (Netlify applies it at edge before headers).
pub fn render_deploy_netlify(spec: &HeaderSpec, tombstones: &[String]) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {GENERATED_MARKER}: Netlify.\n"));
    out.push_str("# Netlify reads `_headers` / `_redirects` from the site root.\n");
    out.push_str("# Tombstones are shipped in the top-level `_redirects`; the\n");
    out.push_str("# header rules below mirror the `_headers` file for operators\n");
    out.push_str("# who already maintain one and want to merge by hand.\n\n");
    out.push_str(&netlify_headers_body(spec));
    if !tombstones.is_empty() {
        out.push_str("\n# Known tombstones (also listed in /_redirects):\n");
        for path in tombstones {
            out.push_str("#   ");
            out.push_str(path);
            out.push_str(" 410!\n");
        }
    }
    out
}

/// Render `<out_dir>/_ztl/deploy-vercel.conf` — a JSON snippet an
/// operator can merge into their own `vercel.json`. Mirrors the
/// top-level `vercel.json` shape, minus the outer wrapper, so it is
/// easy to paste into an existing `{ "headers": [...] }` array.
pub fn render_deploy_vercel(spec: &HeaderSpec, tombstones: &[String]) -> String {
    let mut out = String::new();
    out.push_str(&format!("// {GENERATED_MARKER}: Vercel.\n"));
    out.push_str("// Drop the `headers` / `redirects` arrays into your own vercel.json.\n\n");
    out.push_str(&render_top_vercel_json(spec, tombstones));
    out
}

/// Render `<out_dir>/_ztl/deploy-cloudflare.conf`. Cloudflare Pages
/// accepts the Netlify `_headers` + `_redirects` format verbatim, but
/// the recipe notes the redirects file carries tombstones + that
/// Workers / Page Rules are the alternative path.
pub fn render_deploy_cloudflare(spec: &HeaderSpec, tombstones: &[String]) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {GENERATED_MARKER}: Cloudflare Pages.\n"));
    out.push_str("# Cloudflare Pages reads `_headers` + `_redirects` from the\n");
    out.push_str("# site root (same format as Netlify). For Workers / Page Rules\n");
    out.push_str("# deployments, translate the header rows below into your own\n");
    out.push_str("# response-header configuration.\n\n");
    out.push_str(&netlify_headers_body(spec));
    if !tombstones.is_empty() {
        out.push_str("\n# Known tombstones (also listed in /_redirects):\n");
        for path in tombstones {
            out.push_str("#   ");
            out.push_str(path);
            out.push_str(" 410\n");
        }
    }
    out
}

/// Render a single-file offline bundle HTML (SPEC-034 REQ-3418,
/// `[access.single_file]`). Each envelope is base64-encoded and
/// inlined as a `<template data-ztl-envelope data-slug="...">` block
/// keyed by slug. The v1 bundle is a scaffold — the loader wiring
/// envelopes back to the decryption shim is a downstream task.
pub fn render_single_file_bundle(
    vault_name: &str,
    cohort_id: &str,
    envelopes: &[BundledEnvelope],
) -> String {
    let mut out = String::new();
    out.push_str("<!doctype html>\n");
    out.push_str("<html lang=\"en\">\n");
    out.push_str("<head>\n");
    out.push_str("<meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    out.push_str(&format!(
        "<title>{} ({}) — ztl capability bundle</title>\n",
        html_escape(vault_name),
        html_escape(cohort_id),
    ));
    out.push_str(&format!(
        "<meta name=\"generator\" content=\"{GENERATED_MARKER}: single-file bundle\">\n"
    ));
    out.push_str(&format!(
        "<meta name=\"ztl-vault\" content=\"{}\">\n",
        html_escape(vault_name),
    ));
    out.push_str(&format!(
        "<meta name=\"ztl-cohort\" content=\"{}\">\n",
        html_escape(cohort_id),
    ));
    out.push_str(&format!(
        "<meta name=\"ztl-envelope-count\" content=\"{}\">\n",
        envelopes.len(),
    ));
    out.push_str("</head>\n<body>\n");
    out.push_str("<main data-ztl-capability data-ztl-bundle></main>\n");
    for env in envelopes {
        out.push_str(&format!(
            "<template data-ztl-envelope data-slug=\"{}\">\n",
            html_escape(&env.slug),
        ));
        out.push_str(&B64_STANDARD.encode(&env.envelope_bytes));
        out.push_str("\n</template>\n");
    }
    out.push_str("</body>\n</html>\n");
    out
}

/// Write every deploy artifact into `out_dir`. Idempotent — rerunning
/// against the same directory overwrites each file byte-for-byte.
pub fn write_deploy_artifacts(
    out_dir: &Path,
    input: &DeployArtifactsInput,
) -> Result<Vec<PathBuf>, io::Error> {
    let mut written = Vec::new();
    let ztl_dir = out_dir.join("_ztl");
    fs::create_dir_all(&ztl_dir)?;

    let gone_map = ztl_dir.join("_gone.map");
    fs::write(&gone_map, render_gone_map(&input.tombstones))?;
    written.push(gone_map);

    let redirects = out_dir.join("_redirects");
    fs::write(&redirects, render_netlify_redirects(&input.tombstones))?;
    written.push(redirects);

    let vercel = out_dir.join("vercel.json");
    fs::write(
        &vercel,
        render_top_vercel_json(input.spec, &input.tombstones),
    )?;
    written.push(vercel);

    for (name, body) in [
        (
            "deploy-nginx.conf",
            render_deploy_nginx(input.spec, &input.tombstones),
        ),
        (
            "deploy-caddy.conf",
            render_deploy_caddy(input.spec, &input.tombstones),
        ),
        (
            "deploy-netlify.conf",
            render_deploy_netlify(input.spec, &input.tombstones),
        ),
        (
            "deploy-vercel.conf",
            render_deploy_vercel(input.spec, &input.tombstones),
        ),
        (
            "deploy-cloudflare.conf",
            render_deploy_cloudflare(input.spec, &input.tombstones),
        ),
    ] {
        let path = ztl_dir.join(name);
        fs::write(&path, body)?;
        written.push(path);
    }

    if input.single_file.enabled {
        for bundle in &input.cohort_bundles {
            let filename = format!(
                "{}-{}.html",
                sanitise_filename_component(input.vault_name),
                sanitise_filename_component(&bundle.cohort_id),
            );
            let path = out_dir.join(filename);
            fs::write(
                &path,
                render_single_file_bundle(input.vault_name, &bundle.cohort_id, &bundle.envelopes),
            )?;
            written.push(path);
        }
    }

    Ok(written)
}

fn netlify_headers_body(spec: &HeaderSpec) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{CIPHERTEXT_PATH_PREFIX}*\n  Cache-Control: {}\n  Content-Security-Policy: {}\n\n",
        spec.cap_cache_control, spec.csp
    ));
    for path in CLEAR_SITE_DATA_PATHS {
        out.push_str(&format!(
            "{path}\n  Clear-Site-Data: {}\n",
            spec.clear_site_data
        ));
        if *path == "/enroll.html" {
            out.push_str(&format!("  Content-Security-Policy: {}\n", spec.csp));
        }
        out.push('\n');
    }
    out.push_str(&format!(
        "{SHIM_PATH}\n  Cache-Control: {}\n",
        spec.shim_cache_control
    ));
    out
}

fn vercel_header_entry(source: &str, headers: &[(&str, &str)]) -> String {
    let mut s = String::new();
    s.push_str("    {\n      \"source\": ");
    s.push_str(&json_string(source));
    s.push_str(",\n      \"headers\": [\n");
    for (i, (k, v)) in headers.iter().enumerate() {
        s.push_str("        { \"key\": ");
        s.push_str(&json_string(k));
        s.push_str(", \"value\": ");
        s.push_str(&json_string(v));
        s.push_str(" }");
        if i + 1 < headers.len() {
            s.push(',');
        }
        s.push('\n');
    }
    s.push_str("      ]\n    }");
    s
}

fn json_string(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('"');
    for ch in raw.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn nginx_quote(raw: &str) -> String {
    let needs_quoting = raw
        .chars()
        .any(|c| c.is_whitespace() || c == '"' || c == ';');
    if !needs_quoting {
        return raw.to_string();
    }
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('"');
    for ch in raw.chars() {
        if ch == '"' || ch == '\\' {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

/// Deterministic short tag for Caddy matcher names. Safe ASCII; the
/// hash is non-cryptographic and only needs to be collision-resistant
/// within a single deploy-caddy.conf (the input set is a handful of
/// short paths in practice).
fn short_hash(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    format!("{h:016x}")
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
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

/// Constrain single-file bundle filename components to `[a-zA-Z0-9_-]`.
/// Other characters collapse to `_` so a cohort id with `/` or spaces
/// doesn't escape the dist root.
fn sanitise_filename_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push_str("bundle");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::cap::deploy_headers::CAP_CSP;
    use crate::cap::scoping::access_config::{AccessConfig, CacheConfig};
    use tempfile::TempDir;

    fn default_spec() -> HeaderSpec {
        HeaderSpec::from_cache_config(&AccessConfig::default().cache)
    }

    fn default_input<'a>(
        spec: &'a HeaderSpec,
        sf: &'a SingleFileConfig,
    ) -> DeployArtifactsInput<'a> {
        DeployArtifactsInput {
            spec,
            tombstones: Vec::new(),
            vault_name: "wiki",
            single_file: sf,
            cohort_bundles: Vec::new(),
        }
    }

    #[test]
    fn gone_map_empty_still_declares_map_block() {
        let body = render_gone_map(&[]);
        assert!(body.contains("map $request_uri $ztl_gone"));
        assert!(body.contains("default 0;"));
        assert!(body.ends_with("}\n"));
    }

    #[test]
    fn gone_map_includes_every_tombstone() {
        let paths = vec![
            "/c/aaaa1111/welcome.html".to_string(),
            "/c/bbbb2222/onboarding.html".to_string(),
        ];
        let body = render_gone_map(&paths);
        for p in &paths {
            assert!(
                body.contains(&format!("{p} 1;")),
                "missing {p:?} in:\n{body}"
            );
        }
    }

    #[test]
    fn netlify_redirects_empty_has_scaffold_comment() {
        let body = render_netlify_redirects(&[]);
        assert!(body.contains("No tombstones"));
        // Every non-comment line must be bare — the `410!` token only
        // appears inside the leading `#`-prefixed scaffold comment.
        for line in body.lines() {
            if !line.starts_with('#') {
                assert!(
                    !line.contains("410!"),
                    "unexpected tombstone line in empty scaffold: {line:?}"
                );
            }
        }
    }

    #[test]
    fn netlify_redirects_emit_410_bang() {
        let body = render_netlify_redirects(&["/c/abc/x.html".to_string()]);
        assert!(body.contains("/c/abc/x.html 410!"));
    }

    #[test]
    fn top_vercel_json_parses_with_no_tombstones() {
        let body = render_top_vercel_json(&default_spec(), &[]);
        let parsed: serde_json::Value =
            serde_json::from_str(&body).expect("vercel.json must parse");
        let headers = parsed["headers"].as_array().unwrap();
        assert_eq!(headers.len(), 4, "four header rules expected");
        let redirects = parsed["redirects"].as_array().unwrap();
        assert!(redirects.is_empty());
    }

    #[test]
    fn top_vercel_json_includes_tombstone_redirects() {
        let body = render_top_vercel_json(
            &default_spec(),
            &["/c/abc/x.html".to_string(), "/c/def/y.html".to_string()],
        );
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        let redirects = parsed["redirects"].as_array().unwrap();
        assert_eq!(redirects.len(), 2);
        assert_eq!(redirects[0]["source"], "/c/abc/x.html");
        assert_eq!(redirects[0]["statusCode"], 410);
        assert_eq!(redirects[1]["source"], "/c/def/y.html");
    }

    #[test]
    fn deploy_nginx_wires_gone_map_check() {
        let body = render_deploy_nginx(&default_spec(), &[]);
        assert!(
            body.contains("if ($ztl_gone = 1)"),
            "nginx recipe must check gone map: {body}"
        );
        assert!(body.contains("location ^~ /c/"));
        assert!(body.contains("location = /enroll.html"));
        assert!(body.contains("location = /assets/shim.js"));
        assert!(body.contains("private, max-age=300, must-revalidate"));
        assert!(
            body.contains(&format!("Content-Security-Policy \"{CAP_CSP}\"")),
            "nginx recipe must carry CSP: {body}"
        );
    }

    #[test]
    fn deploy_caddy_emits_respond_410_per_tombstone() {
        let body = render_deploy_caddy(&default_spec(), &["/c/xyz/page.html".to_string()]);
        assert!(body.contains("@ztl_cap path /c/*"));
        assert!(body.contains("path /c/xyz/page.html"));
        assert!(body.contains("respond @ztl_gone_"));
        assert!(body.contains("410"));
    }

    #[test]
    fn deploy_netlify_has_cache_and_clear_site_data_body() {
        let body = render_deploy_netlify(&default_spec(), &[]);
        assert!(body.contains("/c/*\n  Cache-Control: private, max-age=300"));
        assert!(body.contains("/enroll.html\n  Clear-Site-Data:"));
        assert!(body.contains("/logout\n  Clear-Site-Data:"));
        assert!(body.contains("/assets/shim.js\n  Cache-Control: public, max-age=31536000"));
    }

    #[test]
    fn deploy_vercel_is_json_with_leading_comment() {
        let body = render_deploy_vercel(&default_spec(), &[]);
        assert!(
            body.starts_with("// "),
            "vercel recipe should open with a JS-style comment"
        );
        let brace = body.find('{').unwrap();
        let json = &body[brace..];
        let _: serde_json::Value = serde_json::from_str(json).expect("body must be JSON");
    }

    #[test]
    fn deploy_cloudflare_mirrors_netlify_body() {
        let body = render_deploy_cloudflare(&default_spec(), &[]);
        assert!(body.contains("/c/*\n  Cache-Control:"));
        assert!(body.contains("Cloudflare"));
    }

    #[test]
    fn single_file_bundle_inlines_envelopes_as_base64_templates() {
        let body = render_single_file_bundle(
            "wiki",
            "engineering",
            &[
                BundledEnvelope {
                    slug: "welcome".to_string(),
                    envelope_bytes: b"raw-bytes-1".to_vec(),
                },
                BundledEnvelope {
                    slug: "deep/slug".to_string(),
                    envelope_bytes: b"raw-bytes-2".to_vec(),
                },
            ],
        );
        assert!(body.contains("<title>wiki (engineering)"));
        assert!(body.contains("<meta name=\"ztl-envelope-count\" content=\"2\">"));
        assert!(body.contains("data-slug=\"welcome\""));
        // Path separators are preserved in data-slug (HTML-safe).
        assert!(body.contains("data-slug=\"deep/slug\""));
        // Envelope body round-trips through base64.
        let b64_1 = B64_STANDARD.encode(b"raw-bytes-1");
        assert!(body.contains(&b64_1));
    }

    #[test]
    fn write_deploy_artifacts_creates_every_file_and_skips_bundle_by_default() {
        let tmp = TempDir::new().unwrap();
        let spec = default_spec();
        let sf = SingleFileConfig::default();
        write_deploy_artifacts(tmp.path(), &default_input(&spec, &sf)).unwrap();

        let ztl = tmp.path().join("_ztl");
        for name in [
            "_gone.map",
            "deploy-nginx.conf",
            "deploy-caddy.conf",
            "deploy-netlify.conf",
            "deploy-vercel.conf",
            "deploy-cloudflare.conf",
        ] {
            let path = ztl.join(name);
            assert!(path.is_file(), "missing {}", path.display());
            let body = fs::read_to_string(&path).unwrap();
            assert!(body.contains(GENERATED_MARKER), "{name} missing marker");
        }
        assert!(tmp.path().join("_redirects").is_file());
        assert!(tmp.path().join("vercel.json").is_file());

        // No bundle emitted when single_file.enabled = false.
        let entries: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.ends_with(".html"))
            .collect();
        assert!(entries.is_empty(), "no bundle expected, found {entries:?}");
    }

    #[test]
    fn write_deploy_artifacts_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let spec = default_spec();
        let sf = SingleFileConfig::default();
        let input = default_input(&spec, &sf);
        let first = write_deploy_artifacts(tmp.path(), &input).unwrap();
        let first_bodies: Vec<_> = first
            .iter()
            .map(|p| fs::read_to_string(p).unwrap())
            .collect();
        let second = write_deploy_artifacts(tmp.path(), &input).unwrap();
        let second_bodies: Vec<_> = second
            .iter()
            .map(|p| fs::read_to_string(p).unwrap())
            .collect();
        assert_eq!(first, second);
        assert_eq!(first_bodies, second_bodies);
    }

    #[test]
    fn write_deploy_artifacts_emits_single_file_when_enabled() {
        let tmp = TempDir::new().unwrap();
        let spec = default_spec();
        let sf = SingleFileConfig { enabled: true };
        let input = DeployArtifactsInput {
            spec: &spec,
            tombstones: Vec::new(),
            vault_name: "my-wiki",
            single_file: &sf,
            cohort_bundles: vec![
                CohortBundle {
                    cohort_id: "engineering".to_string(),
                    envelopes: vec![BundledEnvelope {
                        slug: "welcome".to_string(),
                        envelope_bytes: b"env-1".to_vec(),
                    }],
                },
                CohortBundle {
                    cohort_id: "ops/core".to_string(),
                    envelopes: vec![],
                },
            ],
        };
        write_deploy_artifacts(tmp.path(), &input).unwrap();
        assert!(tmp.path().join("my-wiki-engineering.html").is_file());
        // Path-like cohort ids collapse to `_` in the filename.
        assert!(tmp.path().join("my-wiki-ops_core.html").is_file());
    }

    #[test]
    fn tombstones_propagate_across_all_platforms() {
        let spec = default_spec();
        let tombstones = vec!["/c/deadbeef/secret.html".to_string()];

        assert!(render_gone_map(&tombstones).contains("/c/deadbeef/secret.html 1;"));
        assert!(render_netlify_redirects(&tombstones).contains("/c/deadbeef/secret.html 410!"));
        let vj = render_top_vercel_json(&spec, &tombstones);
        let parsed: serde_json::Value = serde_json::from_str(&vj).unwrap();
        assert_eq!(parsed["redirects"][0]["source"], "/c/deadbeef/secret.html");
        // The recipe under _ztl/ mentions the tombstone inline as a
        // hint so an operator auditing one file sees every platform's
        // view of the retired path.
        let nginx = render_deploy_nginx(&spec, &tombstones);
        assert!(nginx.contains("/c/deadbeef/secret.html"));
        let caddy = render_deploy_caddy(&spec, &tombstones);
        assert!(caddy.contains("/c/deadbeef/secret.html"));
    }

    #[test]
    fn operator_override_threads_through_headers_of_top_vercel_json() {
        let cache = CacheConfig { max_age: 900 };
        let spec = HeaderSpec::from_cache_config(&cache);
        let body = render_top_vercel_json(&spec, &[]);
        assert!(body.contains("max-age=900"));
        assert!(!body.contains("max-age=300"));
    }

    #[test]
    fn sanitise_filename_component_strips_path_separators() {
        assert_eq!(sanitise_filename_component("ok-name_1"), "ok-name_1");
        assert_eq!(sanitise_filename_component("eng/core"), "eng_core");
        assert_eq!(sanitise_filename_component("../up"), "___up");
        assert_eq!(sanitise_filename_component(""), "bundle");
    }
}
