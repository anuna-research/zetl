//! Build context surfaced to every render-pipeline hook (SPEC-032 REQ-3220).
//!
//! A [`BuildContext`] carries the hook's view of the world at invocation
//! time:
//!
//! - **Session** — [`BuildMode`] (build vs. serve), `verbose`, `at`.
//! - **Locations** — `vault_root`, `out_dir` (`None` under serve),
//!   `hook_path`, `extension_id`.
//! - **Identity** — active `theme` name, zetl binary version, AST schema
//!   version ([`crate::hooks::ast::AST_VERSION`]).
//! - **Page** — [`PageMeta`] with name, relative path, slug, frontmatter.
//! - **Build-data** — a read-only JSON snapshot of the shared key/value
//!   channel (REQ-3219). Snapshot is materialised at
//!   [`BuildContext::from_page_shard`] time; mid-hook writes are emitted
//!   separately through the owning [`PageShard`] and are visible to later
//!   hooks on the same page.
//!
//! The struct is both the in-process Rust surface for trait-object hooks and
//! the serialisation source for the JSON payload sent to subprocess hooks
//! over the REQ-3220 wire protocol. All fields serialise with the exact key
//! names REQ-3220 specifies (`vault_root`, `out_dir`, …) so the wire shape
//! and the Rust shape stay locked together — one `#[derive]` covers both.
//!
//! The `ZETL_*` environment variables REQ-3220 requires for subprocess hooks
//! are derived from this struct by [`BuildContext::env_vars`].

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::hooks::ast::{Frontmatter, AST_VERSION};
use crate::hooks::build_data::PageView;

/// Run mode the build is currently operating in.
///
/// Mirrors the REQ-3220 `ZETL_MODE` values directly so the env-var export
/// path is a single lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BuildMode {
    Build,
    Serve,
}

impl BuildMode {
    /// Stable on-the-wire spelling matching REQ-3220's `ZETL_MODE` contract.
    pub fn as_str(self) -> &'static str {
        match self {
            BuildMode::Build => "build",
            BuildMode::Serve => "serve",
        }
    }
}

impl std::fmt::Display for BuildMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Metadata about the page a pipeline hook is currently processing.
///
/// Scoped to the *current* page render — pages not under render are
/// reachable only through the cross-page `build_data` channel, not through
/// `ctx`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageMeta {
    /// Page name (filename stem, e.g. `"Daily 2026-04-19"`).
    pub name: String,
    /// Repo-relative path from the vault root
    /// (e.g. `"daily/2026-04-19.md"`).
    pub path: PathBuf,
    /// URL slug form (e.g. `"daily/2026-04-19"`).
    pub slug: String,
    /// Parsed YAML frontmatter as a JSON map. Empty when the page has no
    /// frontmatter block.
    #[serde(default)]
    pub frontmatter: Frontmatter,
}

impl PageMeta {
    /// Minimal meta for synthetic pages (tests, fixtures). Frontmatter
    /// starts empty.
    pub fn synthetic(name: impl Into<String>, slug: impl Into<String>) -> Self {
        let name = name.into();
        let path = PathBuf::from(format!("{name}.md"));
        Self {
            name,
            path,
            slug: slug.into(),
            frontmatter: Frontmatter::new(),
        }
    }
}

/// Context passed to every render-pipeline hook invocation (REQ-3220).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuildContext {
    /// `"build"` under `zetl build`, `"serve"` under `zetl serve`.
    pub mode: BuildMode,
    /// Absolute path to the vault root (REQ-3220 `ZETL_VAULT_ROOT`).
    pub vault_root: PathBuf,
    /// Active theme name. Empty string when no theme is configured.
    pub theme: String,
    /// Build output directory. `None` in serve mode per REQ-3220
    /// (serialises as JSON `null`).
    pub out_dir: Option<PathBuf>,
    /// Whether the user requested verbose logging (`--verbose`).
    pub verbose: bool,
    /// Historical-build timestamp if `zetl build --at <expr>` is in effect,
    /// else `None` — the field is omitted from the JSON payload so the
    /// REQ-3220 "else absent" contract holds exactly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    /// Absolute path of the hook's own executable (REQ-3220
    /// `ZETL_HOOK_PATH`). `None` for in-process Rust pipeline hooks;
    /// populated for subprocess dispatch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_path: Option<PathBuf>,
    /// Resolved `extension_id` for this hook (manifest override or
    /// filename default). `None` while the composition layer hasn't
    /// assigned one (e.g. the in-process scaffolding fixtures).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension_id: Option<String>,
    /// zetl binary version string (REQ-3220 `ZETL_VERSION`).
    pub zetl_version: String,
    /// AST schema version the pipeline is emitting at the transform
    /// boundary — tracks [`AST_VERSION`] at the call site so hooks
    /// receiving older or newer shapes can reject loudly.
    pub schema_version: String,
    /// Metadata about the page currently under render.
    pub page: PageMeta,
    /// Read-only JSON snapshot of the shared `build_data` channel
    /// (REQ-3219). Nested shape:
    /// `{ "<extension_id>": { "<key>": <value>, ... }, ... }`.
    #[serde(default = "empty_object")]
    pub build_data: Value,
}

fn empty_object() -> Value {
    Value::Object(Default::default())
}

impl BuildContext {
    /// Construct a context seeded with the zetl binary version string and
    /// the current [`AST_VERSION`] constant.
    ///
    /// Callers populate the remaining fields by chained setters (the
    /// builder-style `with_*` methods) or field assignment. This is the
    /// common entry point for tests and in-process assembly.
    pub fn new(mode: BuildMode, vault_root: impl Into<PathBuf>, page: PageMeta) -> Self {
        Self {
            mode,
            vault_root: vault_root.into(),
            theme: String::new(),
            out_dir: None,
            verbose: false,
            at: None,
            hook_path: None,
            extension_id: None,
            zetl_version: env!("CARGO_PKG_VERSION").to_string(),
            schema_version: AST_VERSION.to_string(),
            page,
            build_data: empty_object(),
        }
    }

    /// Build a context from a live [`PageView`] — materialises the
    /// `build_data` JSON snapshot exactly once here so later hook reads are
    /// O(1) and cannot observe another page's mid-flight writes
    /// (CON-3219's inter-page freeze).
    pub fn from_page_view(
        mode: BuildMode,
        vault_root: impl Into<PathBuf>,
        page: PageMeta,
        build_data: &PageView<'_>,
    ) -> Self {
        let mut ctx = Self::new(mode, vault_root, page);
        ctx.build_data = build_data.to_json();
        ctx
    }

    /// Builder-style setter for the active theme name.
    pub fn with_theme(mut self, theme: impl Into<String>) -> Self {
        self.theme = theme.into();
        self
    }

    /// Builder-style setter for the build output directory.
    pub fn with_out_dir(mut self, out_dir: impl Into<PathBuf>) -> Self {
        self.out_dir = Some(out_dir.into());
        self
    }

    /// Builder-style setter for the verbose flag.
    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    /// Builder-style setter for the historical-build `--at` expression.
    pub fn with_at(mut self, at: impl Into<String>) -> Self {
        self.at = Some(at.into());
        self
    }

    /// Builder-style setter for `ZETL_HOOK_PATH`.
    pub fn with_hook_path(mut self, hook_path: impl Into<PathBuf>) -> Self {
        self.hook_path = Some(hook_path.into());
        self
    }

    /// Builder-style setter for `ZETL_EXTENSION_ID`.
    pub fn with_extension_id(mut self, extension_id: impl Into<String>) -> Self {
        self.extension_id = Some(extension_id.into());
        self
    }

    /// Builder-style setter for the raw `build_data` snapshot. Primarily
    /// for tests; [`Self::from_page_view`] is the production path.
    pub fn with_build_data(mut self, build_data: Value) -> Self {
        self.build_data = build_data;
        self
    }

    /// Look up a value at `build_data[extension_id][key]` in the hook's
    /// frozen snapshot.
    pub fn build_data_get(&self, extension_id: &str, key: &str) -> Option<&Value> {
        self.build_data.get(extension_id).and_then(|m| m.get(key))
    }

    /// Environment variables REQ-3220 requires zetl to set before spawning
    /// a subprocess hook.
    ///
    /// `ZETL_OUT_DIR` is emitted as `""` when `out_dir` is `None` (serve
    /// mode) — env vars can't be `null`, and REQ-3220's wording
    /// ("null under serve") refers to the JSON payload. Callers wanting
    /// to distinguish "unset" from "empty" should consult
    /// [`Self::out_dir`] directly.
    ///
    /// Per REQ-3215 the binary version and AST schema version ship as two
    /// independent strings: `ZETL_VERSION` tracks the zetl release train
    /// and changes every build; `ZETL_AST_SCHEMA_VERSION` tracks the AST
    /// shape and only moves when [`AST_VERSION`] does. Hook authors pin
    /// against the schema version to survive non-AST-breaking binary
    /// upgrades.
    pub fn env_vars(&self) -> Vec<(String, String)> {
        let mut vars = vec![
            ("ZETL_MODE".to_string(), self.mode.as_str().to_string()),
            ("ZETL_THEME".to_string(), self.theme.clone()),
            (
                "ZETL_VAULT_ROOT".to_string(),
                self.vault_root.to_string_lossy().into_owned(),
            ),
            (
                "ZETL_OUT_DIR".to_string(),
                self.out_dir
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            ),
            (
                "ZETL_VERBOSE".to_string(),
                if self.verbose { "true" } else { "false" }.to_string(),
            ),
            ("ZETL_VERSION".to_string(), self.zetl_version.clone()),
            (
                "ZETL_AST_SCHEMA_VERSION".to_string(),
                self.schema_version.clone(),
            ),
        ];
        if let Some(at) = &self.at {
            vars.push(("ZETL_AT".to_string(), at.clone()));
        }
        if let Some(hook_path) = &self.hook_path {
            vars.push((
                "ZETL_HOOK_PATH".to_string(),
                hook_path.to_string_lossy().into_owned(),
            ));
        }
        if let Some(ext_id) = &self.extension_id {
            vars.push(("ZETL_EXTENSION_ID".to_string(), ext_id.clone()));
        }
        vars
    }

    /// Convenience accessor returning the vault root as a [`Path`].
    pub fn vault_root(&self) -> &Path {
        &self.vault_root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::build_data::BuildDataStore;
    use serde_json::json;

    fn sample_page() -> PageMeta {
        let mut fm = Frontmatter::new();
        fm.insert("title".into(), json!("Hello"));
        fm.insert("tags".into(), json!(["greeting"]));
        PageMeta {
            name: "Hello".into(),
            path: PathBuf::from("Hello.md"),
            slug: "hello".into(),
            frontmatter: fm,
        }
    }

    // ── Schema coverage (TEST-3220) ────────────────────────────────────────

    /// TEST-3220 coverage of the ctx payload schema: every REQ-3220 field
    /// serialises under the exact key name the spec names, and optional
    /// fields (`at`, `hook_path`, `extension_id`) are omitted when unset.
    #[test]
    fn ctx_payload_carries_every_req_3220_field() {
        let ctx = BuildContext::new(BuildMode::Build, "/tmp/vault", sample_page())
            .with_theme("fountain")
            .with_out_dir("/tmp/vault/_site")
            .with_verbose(true)
            .with_at("2026-04-01T00:00:00Z")
            .with_hook_path("/opt/zetl/hooks/my-hook")
            .with_extension_id("my-hook");

        let json = serde_json::to_value(&ctx).unwrap();

        assert_eq!(json["mode"], "build");
        assert_eq!(json["vault_root"], "/tmp/vault");
        assert_eq!(json["theme"], "fountain");
        assert_eq!(json["out_dir"], "/tmp/vault/_site");
        assert_eq!(json["verbose"], true);
        assert_eq!(json["at"], "2026-04-01T00:00:00Z");
        assert_eq!(json["hook_path"], "/opt/zetl/hooks/my-hook");
        assert_eq!(json["extension_id"], "my-hook");
        assert_eq!(json["zetl_version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(json["schema_version"], AST_VERSION);
        assert!(json["page"].is_object());
        assert_eq!(json["page"]["name"], "Hello");
        assert_eq!(json["page"]["slug"], "hello");
        assert_eq!(json["page"]["path"], "Hello.md");
        assert_eq!(json["page"]["frontmatter"]["title"], "Hello");
        assert!(json["build_data"].is_object());
    }

    #[test]
    fn optional_fields_absent_when_unset() {
        // Bare ctx: no --at, no hook_path, no extension_id, no out_dir.
        let ctx = BuildContext::new(BuildMode::Serve, "/tmp/v", sample_page());
        let json = serde_json::to_value(&ctx).unwrap();

        // Present but null for out_dir (Option<PathBuf> has no skip).
        assert!(json.get("out_dir").is_some());
        assert!(json["out_dir"].is_null());

        // Completely absent for the REQ-3220 "else absent" trio.
        assert!(json.get("at").is_none());
        assert!(json.get("hook_path").is_none());
        assert!(json.get("extension_id").is_none());
    }

    #[test]
    fn ctx_payload_round_trips_through_json() {
        let ctx = BuildContext::new(BuildMode::Build, "/v", sample_page())
            .with_theme("default")
            .with_extension_id("citations")
            .with_build_data(json!({"citations": {"keys": ["k1", "k2"]}}));

        let bytes = serde_json::to_vec(&ctx).unwrap();
        let round: BuildContext = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(round, ctx);
    }

    // ── Schema version tracking ────────────────────────────────────────────

    #[test]
    fn schema_version_tracks_ast_version_constant() {
        let ctx = BuildContext::new(BuildMode::Build, "/v", sample_page());
        assert_eq!(ctx.schema_version, AST_VERSION);
    }

    #[test]
    fn zetl_version_tracks_cargo_pkg_version() {
        let ctx = BuildContext::new(BuildMode::Build, "/v", sample_page());
        assert_eq!(ctx.zetl_version, env!("CARGO_PKG_VERSION"));
    }

    // ── BuildMode ──────────────────────────────────────────────────────────

    #[test]
    fn build_mode_wire_spelling_matches_req_3220() {
        assert_eq!(BuildMode::Build.as_str(), "build");
        assert_eq!(BuildMode::Serve.as_str(), "serve");
        assert_eq!(
            serde_json::to_value(BuildMode::Build).unwrap(),
            json!("build")
        );
        assert_eq!(
            serde_json::to_value(BuildMode::Serve).unwrap(),
            json!("serve")
        );
    }

    // ── build_data handle (REQ-3219 integration) ──────────────────────────

    #[test]
    fn build_data_snapshot_materialises_from_page_view() {
        // Commit a write on one page, then begin a fresh page and snapshot
        // it into a BuildContext — the snapshot must reflect the merged
        // store.
        let store = BuildDataStore::new();
        let mut p1 = store.begin_page();
        p1.emit("citations", "keys", json!(["a", "b"]));
        store.merge_shard(p1);

        let shard = store.begin_page();
        let ctx =
            BuildContext::from_page_view(BuildMode::Build, "/v", sample_page(), &shard.view());

        assert_eq!(
            ctx.build_data_get("citations", "keys"),
            Some(&json!(["a", "b"]))
        );
        // Missing keys surface as None.
        assert_eq!(ctx.build_data_get("citations", "missing"), None);
        assert_eq!(ctx.build_data_get("other", "k"), None);
    }

    #[test]
    fn build_data_snapshot_respects_inter_page_freeze() {
        // Begin page A, snapshot ctx, then another page's merge must not
        // retroactively appear in ctx's frozen view (CON-3219).
        let store = BuildDataStore::new();
        let shard_a = store.begin_page();
        let ctx_a =
            BuildContext::from_page_view(BuildMode::Build, "/v", sample_page(), &shard_a.view());

        // Page B commits after the snapshot — ctx_a must not see it.
        let mut shard_b = store.begin_page();
        shard_b.emit("ext", "k", json!("late"));
        store.merge_shard(shard_b);

        assert!(ctx_a.build_data_get("ext", "k").is_none());
    }

    // ── Environment-variable export (REQ-3220) ────────────────────────────

    #[test]
    fn env_vars_cover_every_req_3220_name_when_set() {
        let ctx = BuildContext::new(BuildMode::Build, "/tmp/vault", sample_page())
            .with_theme("fountain")
            .with_out_dir("/tmp/vault/_site")
            .with_verbose(true)
            .with_at("2026-04-01T00:00:00Z")
            .with_hook_path("/opt/zetl/hooks/citations")
            .with_extension_id("citations");

        let vars: std::collections::HashMap<_, _> = ctx.env_vars().into_iter().collect();
        assert_eq!(vars.get("ZETL_MODE"), Some(&"build".to_string()));
        assert_eq!(vars.get("ZETL_THEME"), Some(&"fountain".to_string()));
        assert_eq!(vars.get("ZETL_VAULT_ROOT"), Some(&"/tmp/vault".to_string()));
        assert_eq!(
            vars.get("ZETL_OUT_DIR"),
            Some(&"/tmp/vault/_site".to_string())
        );
        assert_eq!(vars.get("ZETL_VERBOSE"), Some(&"true".to_string()));
        assert_eq!(
            vars.get("ZETL_AT"),
            Some(&"2026-04-01T00:00:00Z".to_string())
        );
        assert_eq!(
            vars.get("ZETL_HOOK_PATH"),
            Some(&"/opt/zetl/hooks/citations".to_string())
        );
        assert_eq!(
            vars.get("ZETL_EXTENSION_ID"),
            Some(&"citations".to_string())
        );
        assert_eq!(
            vars.get("ZETL_VERSION"),
            Some(&env!("CARGO_PKG_VERSION").to_string())
        );
        assert_eq!(
            vars.get("ZETL_AST_SCHEMA_VERSION"),
            Some(&AST_VERSION.to_string())
        );
    }

    #[test]
    fn env_vars_serve_mode_omits_optional_and_empties_out_dir() {
        let ctx = BuildContext::new(BuildMode::Serve, "/v", sample_page());
        let vars: std::collections::HashMap<_, _> = ctx.env_vars().into_iter().collect();
        assert_eq!(vars.get("ZETL_MODE"), Some(&"serve".to_string()));
        // out_dir is null under serve → empty env var (can't be null).
        assert_eq!(vars.get("ZETL_OUT_DIR"), Some(&String::new()));
        assert_eq!(vars.get("ZETL_VERBOSE"), Some(&"false".to_string()));
        // Version pair is mandatory at every invocation per REQ-3215 — it
        // survives serve mode and doesn't care about optional fields.
        assert_eq!(
            vars.get("ZETL_VERSION"),
            Some(&env!("CARGO_PKG_VERSION").to_string())
        );
        assert_eq!(
            vars.get("ZETL_AST_SCHEMA_VERSION"),
            Some(&AST_VERSION.to_string())
        );
        // Optional keys absent.
        assert!(!vars.contains_key("ZETL_AT"));
        assert!(!vars.contains_key("ZETL_HOOK_PATH"));
        assert!(!vars.contains_key("ZETL_EXTENSION_ID"));
    }

    /// REQ-3215 dual-version exposure: the env var must track
    /// [`BuildContext::schema_version`] exactly (not the binary version),
    /// so hooks can pin against the AST shape independently of the zetl
    /// release train.
    #[test]
    fn env_vars_decouple_binary_and_schema_versions() {
        let mut ctx = BuildContext::new(BuildMode::Build, "/v", sample_page());
        ctx.zetl_version = "9.9.9-dev".into();
        ctx.schema_version = "1.2".into();

        let vars: std::collections::HashMap<_, _> = ctx.env_vars().into_iter().collect();
        assert_eq!(vars.get("ZETL_VERSION"), Some(&"9.9.9-dev".to_string()));
        assert_eq!(
            vars.get("ZETL_AST_SCHEMA_VERSION"),
            Some(&"1.2".to_string())
        );
        // The two keys carry *different* strings — a hook pinning on the
        // schema version must not observe the binary version leaking in.
        assert_ne!(
            vars.get("ZETL_VERSION"),
            vars.get("ZETL_AST_SCHEMA_VERSION")
        );
    }

    // ── Page metadata ──────────────────────────────────────────────────────

    #[test]
    fn page_meta_synthetic_helper() {
        let page = PageMeta::synthetic("Daily 2026-04-19", "daily/2026-04-19");
        assert_eq!(page.name, "Daily 2026-04-19");
        assert_eq!(page.slug, "daily/2026-04-19");
        assert_eq!(page.path, PathBuf::from("Daily 2026-04-19.md"));
        assert!(page.frontmatter.is_empty());
    }

    #[test]
    fn vault_root_accessor_returns_path() {
        let ctx = BuildContext::new(BuildMode::Build, "/tmp/v", sample_page());
        assert_eq!(ctx.vault_root(), Path::new("/tmp/v"));
    }
}
