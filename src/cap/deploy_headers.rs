//! Cache-Control + Clear-Site-Data + CSP deploy-header recipes
//! (SPEC-034 REQ-3407, REQ-3418, REQ-3421, REQ-3428 / CON-3406,
//! CON-3410 — BUG-006 resolution).
//!
//! Operators of capability-mode static deployments configure their
//! HTTP server (nginx, Caddy) or CDN (Netlify, Vercel, Cloudflare
//! Pages) to emit four headers that the capability shim + spec rely
//! on for revocation latency bounds, ServiceWorker hygiene, and
//! XSS / CDN-substitution resistance:
//!
//! - `Cache-Control: private, max-age=<N>, must-revalidate` on
//!   `/c/*` ciphertext responses. `max-age` is operator-tunable via
//!   `[access.cache] max_age` with hard bounds `[60, 3600]`
//!   (NFR-3409 revocation-latency envelope). Default 300.
//! - `Clear-Site-Data: "cache", "storage", "executionContexts"` on
//!   `/enroll.html` and the `/logout` SPA endpoint (REQ-3428 /
//!   BUG-008).
//! - `Cache-Control: public, max-age=31536000, immutable` on
//!   `/assets/shim.js`. Fixed because the shim filename is content-
//!   hashed on emission, so a new shim gets a new path and the
//!   immutable cache never stales.
//! - `Content-Security-Policy: default-src 'none'; script-src
//!   'self'; …; require-trusted-types-for 'script'; trusted-types
//!   'none';` on `/c/*` and `/enroll.html` (REQ-3421 / CON-3410).
//!   The shell HTML also carries the same directive as a
//!   `<meta http-equiv>` fallback, so a CDN that forgets to emit
//!   the header still enforces CSP in the browser.
//!
//! This module is the single source of truth for the header values
//! and path-to-header mapping; each recipe emitter consults the same
//! [`HeaderSpec`] struct so the rules stay byte-identical across
//! formats.
//!
//! Output layout (written by [`write_deploy_recipes`]):
//!
//! ```text
//! <out_dir>/_zetl/deploy/
//!   ├── README.md
//!   ├── nginx.conf.snippet
//!   ├── Caddyfile.snippet
//!   ├── _headers              (Netlify + Cloudflare Pages — same format)
//!   └── vercel.json
//! ```
//!
//! Emission is idempotent — rerunning `run_capability_build` against
//! the same `out_dir` rewrites each recipe byte-for-byte.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::cap::scoping::access_config::CacheConfig;

/// Fixed `max-age` for `/assets/shim.js` (SPEC-034 CON-3406). The
/// shim's filename is content-hashed at build time, so a new shim
/// ships under a new path — a year-long immutable cache is safe.
pub const SHIM_MAX_AGE: u32 = 31_536_000;

/// Fixed `Clear-Site-Data` header value. Quoted tokens per the
/// W3C `Clear-Site-Data` spec.
pub const CLEAR_SITE_DATA_VALUE: &str = "\"cache\", \"storage\", \"executionContexts\"";

/// Paths that must receive `Clear-Site-Data` (REQ-3428). Ordered for
/// byte-stable recipe emission.
pub const CLEAR_SITE_DATA_PATHS: &[&str] = &["/enroll.html", "/logout"];

/// Path to the shim bundle (SPEC-034 CON-3406).
pub const SHIM_PATH: &str = "/assets/shim.js";

/// Path prefix for ciphertext envelopes (SPEC-034 CON-3401).
pub const CIPHERTEXT_PATH_PREFIX: &str = "/c/";

/// Content-Security-Policy header value mandated by SPEC-034
/// REQ-3421 / CON-3410 (BUG-006 resolution). The exact directive
/// order is pinned so the HTTP header, the HTML shell's
/// `<meta http-equiv>` fallback, and the spec all carry the same
/// byte-for-byte string.
///
/// - `default-src 'none'`        — deny everything by default.
/// - `script-src 'self'`         — only the SRI-pinned shim.
/// - `style-src 'self'`          — stylesheets from same origin.
/// - `img-src 'self' data:`      — inline data URIs allowed for
///   sanitised-HTML image payloads (no remote fetches).
/// - `connect-src 'self'`        — the shim's `fetch(location.pathname)`
///   for the envelope is a same-origin request but modern browsers
///   gate it on `connect-src` regardless, so `'none'` would block
///   the envelope fetch. `'self'` restricts fetches to the serving
///   origin; the envelope is the only resource the shim fetches
///   over the network.
/// - `font-src 'self'`           — fonts from same origin only.
/// - `frame-ancestors 'none'`    — unframeable.
/// - `base-uri 'none'`           — `<base>` injection blocked.
/// - `form-action 'none'`        — blocks exfil via form POST.
/// - `require-trusted-types-for 'script'` + `trusted-types zetl-cap`
///   — Trusted Types gate for the shim's `innerHTML` assignment
///   path. The shim registers a `zetl-cap` policy in
///   `src/cap/shim/render.ts` that identity-wraps the already-
///   sanitised HTML into TrustedHTML before assignment. Only the
///   `zetl-cap` policy name is allowed so foreign scripts cannot
///   create their own sinks, and `require-trusted-types-for 'script'`
///   blocks raw string assignments to DOM sinks document-wide.
pub const CAP_CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; \
    img-src 'self' data:; connect-src 'self'; font-src 'self'; \
    frame-ancestors 'none'; base-uri 'none'; form-action 'none'; \
    require-trusted-types-for 'script'; trusted-types zetl-cap;";

/// Paths that must receive the CSP response header (REQ-3421). The
/// shell HTML under `/c/*` is the browser's first-navigation target;
/// `/enroll.html` is the hardened-mode self-enrol page. Both need
/// the header so CSP is enforced even before `<meta http-equiv>` is
/// parsed. Ordered for byte-stable recipe emission.
pub const CSP_PATHS: &[&str] = &[CIPHERTEXT_PATH_PREFIX, "/enroll.html"];

/// Logical header set captured once and rendered into every recipe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderSpec {
    /// `Cache-Control` value for `/c/*` ciphertext paths. Built from
    /// [`CacheConfig::max_age`] (operator-tunable).
    pub cap_cache_control: String,
    /// `Cache-Control` value for `/assets/shim.js`. Fixed.
    pub shim_cache_control: String,
    /// `Clear-Site-Data` value for `/enroll.html` and `/logout`.
    /// Fixed.
    pub clear_site_data: String,
    /// `Content-Security-Policy` value for `/c/*` and `/enroll.html`.
    /// Pinned to the SPEC-034 CON-3410 directive string.
    pub csp: String,
}

impl HeaderSpec {
    /// Build the effective header spec from the operator's
    /// `[access.cache]` config. The caller is expected to have run
    /// [`super::scoping::access_config::AccessConfig::validate`]
    /// first — this constructor does not re-validate the bounds.
    pub fn from_cache_config(cache: &CacheConfig) -> Self {
        Self {
            cap_cache_control: format!("private, max-age={}, must-revalidate", cache.max_age),
            shim_cache_control: format!("public, max-age={SHIM_MAX_AGE}, immutable"),
            clear_site_data: CLEAR_SITE_DATA_VALUE.to_string(),
            csp: CAP_CSP.to_string(),
        }
    }
}

/// Render the nginx `location` recipe. Designed to be `include`d
/// from the operator's `server { }` block; `always` on each
/// `add_header` ensures the header is applied to non-2xx responses
/// too (nginx's default otherwise suppresses headers on 4xx/5xx).
pub fn render_nginx(spec: &HeaderSpec) -> String {
    let mut out = String::new();
    out.push_str(RECIPE_HEADER_COMMENT);
    out.push_str("# Format: nginx `location` blocks; include from a server { } block.\n\n");
    out.push_str(&format!(
        "location ^~ {CIPHERTEXT_PATH_PREFIX} {{\n    \
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
        if path == &"/enroll.html" {
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
    out
}

/// Render the Caddy `header` recipe. Named matchers scope each rule
/// without interfering with the operator's other matchers.
pub fn render_caddy(spec: &HeaderSpec) -> String {
    let mut out = String::new();
    out.push_str(RECIPE_HEADER_COMMENT);
    out.push_str("# Format: Caddyfile directives; paste inside your site block.\n\n");
    out.push_str(&format!(
        "@zetl_cap path {CIPHERTEXT_PATH_PREFIX}*\n\
         header @zetl_cap Cache-Control \"{}\"\n\
         header @zetl_cap Content-Security-Policy \"{}\"\n\n",
        spec.cap_cache_control, spec.csp
    ));
    for (idx, path) in CLEAR_SITE_DATA_PATHS.iter().enumerate() {
        let matcher = format!("@zetl_csd_{idx}");
        out.push_str(&format!(
            "{matcher} path {path}\nheader {matcher} Clear-Site-Data `{}`\n",
            spec.clear_site_data
        ));
        if path == &"/enroll.html" {
            out.push_str(&format!(
                "header {matcher} Content-Security-Policy \"{}\"\n",
                spec.csp
            ));
        }
        out.push('\n');
    }
    out.push_str(&format!(
        "@zetl_shim path {SHIM_PATH}\nheader @zetl_shim Cache-Control \"{}\"\n",
        spec.shim_cache_control
    ));
    out
}

/// Render Netlify (and Cloudflare Pages) `_headers` file. Both
/// platforms consume the same format.
pub fn render_netlify_headers(spec: &HeaderSpec) -> String {
    let mut out = String::new();
    out.push_str(RECIPE_HEADER_COMMENT);
    out.push_str(
        "# Format: Netlify / Cloudflare Pages `_headers` file (deploy at the site root).\n\n",
    );
    out.push_str(&format!(
        "{CIPHERTEXT_PATH_PREFIX}*\n  Cache-Control: {}\n  Content-Security-Policy: {}\n\n",
        spec.cap_cache_control, spec.csp
    ));
    for path in CLEAR_SITE_DATA_PATHS {
        out.push_str(&format!(
            "{path}\n  Clear-Site-Data: {}\n",
            spec.clear_site_data
        ));
        if path == &"/enroll.html" {
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

/// Render Vercel `vercel.json`. Emitted as a minimal `{"headers": [...]}`
/// document; operators with an existing `vercel.json` merge the
/// `headers` array manually.
pub fn render_vercel_json(spec: &HeaderSpec) -> String {
    let ciphertext_source = format!("{CIPHERTEXT_PATH_PREFIX}(.*)");
    let mut entries: Vec<String> = Vec::new();
    entries.push(vercel_entry(
        &ciphertext_source,
        &[
            ("Cache-Control", spec.cap_cache_control.as_str()),
            ("Content-Security-Policy", spec.csp.as_str()),
        ],
    ));
    for path in CLEAR_SITE_DATA_PATHS {
        if path == &"/enroll.html" {
            entries.push(vercel_entry(
                path,
                &[
                    ("Clear-Site-Data", spec.clear_site_data.as_str()),
                    ("Content-Security-Policy", spec.csp.as_str()),
                ],
            ));
        } else {
            entries.push(vercel_entry(
                path,
                &[("Clear-Site-Data", spec.clear_site_data.as_str())],
            ));
        }
    }
    entries.push(vercel_entry(
        SHIM_PATH,
        &[("Cache-Control", spec.shim_cache_control.as_str())],
    ));

    let mut out = String::new();
    out.push_str("{\n  \"headers\": [\n");
    for (i, entry) in entries.iter().enumerate() {
        out.push_str(entry);
        if i + 1 < entries.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  ]\n}\n");
    out
}

fn vercel_entry(source: &str, headers: &[(&str, &str)]) -> String {
    let mut s = String::new();
    s.push_str("    {\n      \"source\": \"");
    s.push_str(source);
    s.push_str("\",\n      \"headers\": [\n");
    for (i, (k, v)) in headers.iter().enumerate() {
        s.push_str("        { \"key\": \"");
        s.push_str(k);
        s.push_str("\", \"value\": ");
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

/// Readme that explains which recipe belongs to which platform so an
/// operator dropping into `<out_dir>/_zetl/deploy/` can self-serve.
pub fn render_readme(spec: &HeaderSpec) -> String {
    let mut out = String::new();
    out.push_str("# zetl capability-mode deploy recipes\n\n");
    out.push_str(
        "Drop the snippet that matches your HTTP server / CDN into the site \
         config. Every recipe below emits the same three header rules; they are \
         the only runtime requirement the capability shim has on your hosting \
         platform (SPEC-034 REQ-3407, REQ-3418, REQ-3428).\n\n",
    );
    out.push_str("## Header rules\n\n");
    out.push_str(&format!(
        "- `{CIPHERTEXT_PATH_PREFIX}*` → `Cache-Control: {}`\n",
        spec.cap_cache_control
    ));
    out.push_str(&format!(
        "- `{CIPHERTEXT_PATH_PREFIX}*` → `Content-Security-Policy: {}`\n",
        spec.csp
    ));
    for path in CLEAR_SITE_DATA_PATHS {
        out.push_str(&format!(
            "- `{path}` → `Clear-Site-Data: {}`\n",
            spec.clear_site_data
        ));
        if path == &"/enroll.html" {
            out.push_str(&format!(
                "- `{path}` → `Content-Security-Policy: {}`\n",
                spec.csp
            ));
        }
    }
    out.push_str(&format!(
        "- `{SHIM_PATH}` → `Cache-Control: {}`\n\n",
        spec.shim_cache_control
    ));
    out.push_str("## Files\n\n");
    out.push_str("- `nginx.conf.snippet` — nginx `location` blocks.\n");
    out.push_str("- `Caddyfile.snippet` — Caddyfile matcher + `header` directives.\n");
    out.push_str(
        "- `_headers` — Netlify **and** Cloudflare Pages (both consume the same format).\n",
    );
    out.push_str("- `vercel.json` — minimal `{\"headers\": [...]}` document for Vercel.\n\n");
    out.push_str("## Revocation latency\n\n");
    out.push_str(
        "The `/c/*` max-age is operator-tunable via `[access.cache] max_age` \
         (bounds [60, 3600]). Lowering it shortens the window between `zetl cap \
         revoke` and readers no longer being able to load the page from CDN \
         caches. See NFR-3409.\n\n",
    );
    out.push_str("## CSP fallback\n\n");
    out.push_str(
        "The HTML shell emitted under `_zetl/capability-shell.html` carries the \
         same `Content-Security-Policy` directive as a `<meta http-equiv>` tag. \
         If a CDN drops the HTTP header, the meta tag still enforces CSP in the \
         browser — the recipe + shell are belt-and-braces. See SPEC-034 \
         REQ-3421 / CON-3410.\n",
    );
    out
}

const RECIPE_HEADER_COMMENT: &str = "\
# zetl capability-mode HTTP header recipe
# Generated by `zetl build` from SPEC-034 REQ-3407 / REQ-3418 / REQ-3421 / REQ-3428.
# Do not edit by hand — re-run the build if you want different values.
";

/// Write all recipes into `<out_dir>/_zetl/deploy/`. Creates missing
/// directories and overwrites any existing file.
pub fn write_deploy_recipes(out_dir: &Path, spec: &HeaderSpec) -> Result<PathBuf, io::Error> {
    let deploy_dir = out_dir.join("_zetl").join("deploy");
    fs::create_dir_all(&deploy_dir)?;
    fs::write(deploy_dir.join("README.md"), render_readme(spec))?;
    fs::write(deploy_dir.join("nginx.conf.snippet"), render_nginx(spec))?;
    fs::write(deploy_dir.join("Caddyfile.snippet"), render_caddy(spec))?;
    fs::write(deploy_dir.join("_headers"), render_netlify_headers(spec))?;
    fs::write(deploy_dir.join("vercel.json"), render_vercel_json(spec))?;
    Ok(deploy_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::cap::scoping::access_config::{AccessConfig, CacheConfig, DEFAULT_CAP_MAX_AGE};
    use tempfile::TempDir;

    fn default_spec() -> HeaderSpec {
        HeaderSpec::from_cache_config(&AccessConfig::default().cache)
    }

    #[test]
    fn default_spec_uses_300s_cap_max_age() {
        let spec = default_spec();
        assert!(spec
            .cap_cache_control
            .contains(&format!("max-age={DEFAULT_CAP_MAX_AGE}")));
        assert!(spec.cap_cache_control.starts_with("private, "));
        assert!(spec.cap_cache_control.ends_with(", must-revalidate"));
    }

    #[test]
    fn shim_cache_control_is_year_long_immutable() {
        let spec = default_spec();
        assert_eq!(
            spec.shim_cache_control,
            "public, max-age=31536000, immutable"
        );
    }

    #[test]
    fn clear_site_data_is_fixed_spec_string() {
        let spec = default_spec();
        assert_eq!(
            spec.clear_site_data,
            "\"cache\", \"storage\", \"executionContexts\""
        );
    }

    #[test]
    fn operator_override_threaded_through_spec() {
        let cache = CacheConfig { max_age: 900 };
        let spec = HeaderSpec::from_cache_config(&cache);
        assert!(spec.cap_cache_control.contains("max-age=900"));
    }

    #[test]
    fn nginx_recipe_contains_every_rule() {
        let rendered = render_nginx(&default_spec());
        assert!(rendered.contains("location ^~ /c/"));
        assert!(rendered.contains("Cache-Control \"private, max-age=300, must-revalidate\""));
        assert!(rendered.contains("location = /enroll.html"));
        assert!(rendered.contains("location = /logout"));
        assert!(rendered.contains("location = /assets/shim.js"));
        assert!(rendered.contains("public, max-age=31536000, immutable"));
        assert!(rendered.contains("Clear-Site-Data"));
        // REQ-3421: CSP is emitted on both /c/* and /enroll.html, with
        // the exact directive from CAP_CSP. /logout is Clear-Site-Data
        // only — it is not an HTML path a reader lands on before the
        // shim runs, so the CSP header would only confuse the operator.
        assert!(
            rendered.contains(&format!(
                "add_header Content-Security-Policy \"{CAP_CSP}\" always;"
            )),
            "expected CSP header in nginx recipe, got:\n{rendered}"
        );
        // nginx uses single-quote wrapping for CSD so the double-quoted
        // token list inside the value survives verbatim.
        assert!(
            rendered.contains("'\"cache\", \"storage\", \"executionContexts\"'"),
            "expected single-quoted CSD payload, got:\n{rendered}"
        );
    }

    #[test]
    fn caddy_recipe_contains_every_rule() {
        let rendered = render_caddy(&default_spec());
        assert!(rendered.contains("@zetl_cap path /c/*"));
        assert!(rendered
            .contains("header @zetl_cap Cache-Control \"private, max-age=300, must-revalidate\""));
        assert!(rendered.contains("@zetl_shim path /assets/shim.js"));
        assert!(rendered.contains("public, max-age=31536000, immutable"));
        for path in CLEAR_SITE_DATA_PATHS {
            assert!(
                rendered.contains(&format!("path {path}\n")),
                "missing Caddy matcher for {path}"
            );
        }
        assert!(
            rendered.contains(&format!(
                "header @zetl_cap Content-Security-Policy \"{CAP_CSP}\""
            )),
            "expected CSP on @zetl_cap in Caddy recipe, got:\n{rendered}"
        );
        assert!(
            rendered.contains(&format!("Content-Security-Policy \"{CAP_CSP}\"")),
            "expected CSP header (on /enroll.html matcher) in Caddy recipe, got:\n{rendered}"
        );
        // Backtick-wrapped CSD payload so Caddy's tokenizer doesn't
        // consume the inner double quotes.
        assert!(
            rendered.contains("`\"cache\", \"storage\", \"executionContexts\"`"),
            "expected backtick-wrapped CSD payload, got:\n{rendered}"
        );
    }

    #[test]
    fn netlify_headers_recipe_contains_every_rule() {
        let rendered = render_netlify_headers(&default_spec());
        assert!(rendered.contains("/c/*\n  Cache-Control: private, max-age=300, must-revalidate"));
        assert!(rendered.contains(
            "/enroll.html\n  Clear-Site-Data: \"cache\", \"storage\", \"executionContexts\""
        ));
        assert!(rendered
            .contains("/logout\n  Clear-Site-Data: \"cache\", \"storage\", \"executionContexts\""));
        assert!(rendered
            .contains("/assets/shim.js\n  Cache-Control: public, max-age=31536000, immutable"));
        // REQ-3421: CSP is stacked under /c/* right after Cache-Control,
        // and under /enroll.html right after Clear-Site-Data.
        assert!(
            rendered.contains(&format!("/c/*\n  Cache-Control: private, max-age=300, must-revalidate\n  Content-Security-Policy: {CAP_CSP}")),
            "expected /c/* CSP stacked under Cache-Control in _headers, got:\n{rendered}"
        );
        assert!(
            rendered.contains(&format!("/enroll.html\n  Clear-Site-Data: \"cache\", \"storage\", \"executionContexts\"\n  Content-Security-Policy: {CAP_CSP}")),
            "expected /enroll.html CSP in _headers, got:\n{rendered}"
        );
        // /logout gets Clear-Site-Data but NOT CSP (not an HTML surface).
        assert!(
            !rendered.contains(&format!("/logout\n  Clear-Site-Data: \"cache\", \"storage\", \"executionContexts\"\n  Content-Security-Policy:")),
            "/logout should not carry CSP in _headers"
        );
    }

    #[test]
    fn vercel_json_parses_and_contains_every_rule() {
        let rendered = render_vercel_json(&default_spec());
        let parsed: serde_json::Value =
            serde_json::from_str(&rendered).expect("vercel.json must be valid JSON");
        let headers = parsed
            .get("headers")
            .and_then(|v| v.as_array())
            .expect("top-level `headers` array");
        assert_eq!(headers.len(), 4);

        let sources: Vec<&str> = headers
            .iter()
            .map(|entry| entry.get("source").unwrap().as_str().unwrap())
            .collect();
        assert_eq!(
            sources,
            vec!["/c/(.*)", "/enroll.html", "/logout", "/assets/shim.js"]
        );

        let cap_headers = headers[0]["headers"].as_array().unwrap();
        assert_eq!(cap_headers.len(), 2, "expected Cache-Control + CSP on /c/*");
        assert_eq!(cap_headers[0]["key"], "Cache-Control");
        assert_eq!(
            cap_headers[0]["value"],
            "private, max-age=300, must-revalidate"
        );
        assert_eq!(cap_headers[1]["key"], "Content-Security-Policy");
        assert_eq!(cap_headers[1]["value"], CAP_CSP);

        let enroll_headers = headers[1]["headers"].as_array().unwrap();
        assert_eq!(
            enroll_headers.len(),
            2,
            "expected Clear-Site-Data + CSP on /enroll.html"
        );
        assert_eq!(enroll_headers[0]["key"], "Clear-Site-Data");
        assert_eq!(
            enroll_headers[0]["value"],
            "\"cache\", \"storage\", \"executionContexts\""
        );
        assert_eq!(enroll_headers[1]["key"], "Content-Security-Policy");
        assert_eq!(enroll_headers[1]["value"], CAP_CSP);

        let logout_headers = headers[2]["headers"].as_array().unwrap();
        assert_eq!(logout_headers.len(), 1, "/logout has Clear-Site-Data only");
        assert_eq!(logout_headers[0]["key"], "Clear-Site-Data");

        let shim_headers = headers[3]["headers"].as_array().unwrap();
        assert_eq!(shim_headers.len(), 1);
        assert_eq!(shim_headers[0]["key"], "Cache-Control");
        assert_eq!(
            shim_headers[0]["value"],
            "public, max-age=31536000, immutable"
        );
    }

    #[test]
    fn csp_constant_matches_spec_directive_set() {
        // Byte-exact guard against accidental drift from SPEC-034
        // CON-3410. Any directive reorder or case change trips this.
        assert_eq!(
            CAP_CSP,
            "default-src 'none'; script-src 'self'; style-src 'self'; \
             img-src 'self' data:; connect-src 'self'; font-src 'self'; \
             frame-ancestors 'none'; base-uri 'none'; form-action 'none'; \
             require-trusted-types-for 'script'; trusted-types zetl-cap;"
        );
    }

    #[test]
    fn default_spec_uses_pinned_csp() {
        assert_eq!(default_spec().csp, CAP_CSP);
    }

    #[test]
    fn write_deploy_recipes_creates_every_file() {
        let tmp = TempDir::new().unwrap();
        let dir = write_deploy_recipes(tmp.path(), &default_spec()).unwrap();
        for name in [
            "README.md",
            "nginx.conf.snippet",
            "Caddyfile.snippet",
            "_headers",
            "vercel.json",
        ] {
            let path = dir.join(name);
            assert!(path.is_file(), "missing {name}");
            let body = fs::read_to_string(&path).unwrap();
            assert!(!body.is_empty(), "empty recipe {name}");
        }
    }

    #[test]
    fn write_deploy_recipes_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let dir1 = write_deploy_recipes(tmp.path(), &default_spec()).unwrap();
        let before: Vec<String> = ["README.md", "nginx.conf.snippet", "_headers", "vercel.json"]
            .iter()
            .map(|name| fs::read_to_string(dir1.join(name)).unwrap())
            .collect();
        let dir2 = write_deploy_recipes(tmp.path(), &default_spec()).unwrap();
        assert_eq!(dir1, dir2);
        let after: Vec<String> = ["README.md", "nginx.conf.snippet", "_headers", "vercel.json"]
            .iter()
            .map(|name| fs::read_to_string(dir2.join(name)).unwrap())
            .collect();
        assert_eq!(before, after);
    }
}
