//! Cache-Control + Clear-Site-Data deploy-header recipes (SPEC-034
//! REQ-3407, REQ-3418, REQ-3428 / CON-3406).
//!
//! Operators of capability-mode static deployments configure their
//! HTTP server (nginx, Caddy) or CDN (Netlify, Vercel, Cloudflare
//! Pages) to emit three headers that the capability shim + spec rely
//! on for revocation latency bounds and ServiceWorker hygiene:
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
}

impl HeaderSpec {
    /// Build the effective header spec from the operator's
    /// `[access.cache]` config. The caller is expected to have run
    /// [`super::scoping::access_config::AccessConfig::validate`]
    /// first — this constructor does not re-validate the bounds.
    pub fn from_cache_config(cache: &CacheConfig) -> Self {
        Self {
            cap_cache_control: format!(
                "private, max-age={}, must-revalidate",
                cache.max_age
            ),
            shim_cache_control: format!("public, max-age={SHIM_MAX_AGE}, immutable"),
            clear_site_data: CLEAR_SITE_DATA_VALUE.to_string(),
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
         add_header Cache-Control \"{}\" always;\n}}\n\n",
        spec.cap_cache_control
    ));
    for path in CLEAR_SITE_DATA_PATHS {
        out.push_str(&format!(
            "location = {path} {{\n    \
             add_header Clear-Site-Data '{}' always;\n}}\n\n",
            spec.clear_site_data
        ));
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
        "@zetl_cap path {CIPHERTEXT_PATH_PREFIX}*\nheader @zetl_cap Cache-Control \"{}\"\n\n",
        spec.cap_cache_control
    ));
    for (idx, path) in CLEAR_SITE_DATA_PATHS.iter().enumerate() {
        let matcher = format!("@zetl_csd_{idx}");
        out.push_str(&format!(
            "{matcher} path {path}\nheader {matcher} Clear-Site-Data `{}`\n\n",
            spec.clear_site_data
        ));
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
        "{CIPHERTEXT_PATH_PREFIX}*\n  Cache-Control: {}\n\n",
        spec.cap_cache_control
    ));
    for path in CLEAR_SITE_DATA_PATHS {
        out.push_str(&format!(
            "{path}\n  Clear-Site-Data: {}\n\n",
            spec.clear_site_data
        ));
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
    entries.push(vercel_entry(&ciphertext_source, "Cache-Control", &spec.cap_cache_control));
    for path in CLEAR_SITE_DATA_PATHS {
        entries.push(vercel_entry(path, "Clear-Site-Data", &spec.clear_site_data));
    }
    entries.push(vercel_entry(SHIM_PATH, "Cache-Control", &spec.shim_cache_control));

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

fn vercel_entry(source: &str, key: &str, value: &str) -> String {
    format!(
        "    {{\n      \"source\": \"{src}\",\n      \"headers\": [\n        \
         {{ \"key\": \"{k}\", \"value\": {v} }}\n      ]\n    }}",
        src = source,
        k = key,
        v = json_string(value),
    )
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
    for path in CLEAR_SITE_DATA_PATHS {
        out.push_str(&format!(
            "- `{path}` → `Clear-Site-Data: {}`\n",
            spec.clear_site_data
        ));
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
         caches. See NFR-3409.\n",
    );
    out
}

const RECIPE_HEADER_COMMENT: &str = "\
# zetl capability-mode HTTP header recipe
# Generated by `zetl build` from SPEC-034 REQ-3407 / REQ-3418 / REQ-3428.
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
        assert!(spec.cap_cache_control.contains(&format!("max-age={DEFAULT_CAP_MAX_AGE}")));
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
        assert!(rendered.contains("header @zetl_cap Cache-Control \"private, max-age=300, must-revalidate\""));
        assert!(rendered.contains("@zetl_shim path /assets/shim.js"));
        assert!(rendered.contains("public, max-age=31536000, immutable"));
        for path in CLEAR_SITE_DATA_PATHS {
            assert!(
                rendered.contains(&format!("path {path}\n")),
                "missing Caddy matcher for {path}"
            );
        }
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
        assert!(
            rendered.contains("/enroll.html\n  Clear-Site-Data: \"cache\", \"storage\", \"executionContexts\"")
        );
        assert!(
            rendered.contains("/logout\n  Clear-Site-Data: \"cache\", \"storage\", \"executionContexts\"")
        );
        assert!(
            rendered.contains("/assets/shim.js\n  Cache-Control: public, max-age=31536000, immutable")
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

        let cap_entry = &headers[0]["headers"][0];
        assert_eq!(cap_entry["key"], "Cache-Control");
        assert_eq!(
            cap_entry["value"],
            "private, max-age=300, must-revalidate"
        );

        let enroll_entry = &headers[1]["headers"][0];
        assert_eq!(enroll_entry["key"], "Clear-Site-Data");
        assert_eq!(
            enroll_entry["value"],
            "\"cache\", \"storage\", \"executionContexts\""
        );

        let shim_entry = &headers[3]["headers"][0];
        assert_eq!(shim_entry["key"], "Cache-Control");
        assert_eq!(
            shim_entry["value"],
            "public, max-age=31536000, immutable"
        );
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
