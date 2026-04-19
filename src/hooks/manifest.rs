//! Hook-manifest parser (SPEC-032 REQ-3203 / CON-3203).
//!
//! Parses `.zetl/hooks/<stage>.d/<name>.toml` sidecar manifests into a
//! typed [`Manifest`] with full field coverage:
//!
//! - Top-level: `stage`, `mode`, `timeout_ms`, `memory_mib`, `ast_type`,
//!   `ast_version`, `extension_id`, `optional`, `before`, `after`.
//! - `[ordering]`: `before`, `after` (canonical REQ-3217 form, merged
//!   with the top-level aliases).
//! - `[select]`: `include`, `exclude`, `frontmatter_where`,
//!   `content_probe`, `require_probe_match`.
//! - `[contract]`: `preserves`, `idempotent`, `may_restructure`, `pure`,
//!   `expansion_bound` (Tier-1 + Tier-2 fields per REQ-3224 / CON-3224).
//!
//! CON-3203 says every field is optional (stage may be inferred from
//! directory name) and unknown fields SHALL be rejected with a parse
//! error. This module enforces that contract with
//! `#[serde(deny_unknown_fields)]` at every table boundary.
//!
//! # Relationship to [`crate::hooks::composition`]
//!
//! The composition module reads its own loose `CompositionManifest`
//! subset (extension_id, before/after, ordering, ast_type, contract.preserves)
//! without rejecting unknowns — it has to stay forward-compatible with
//! hooks authored against future manifest extensions. This module is
//! the *strict* parser and owns the full grammar. Downstream wiring
//! (replacing the composition subset with this full parser at pipeline
//! init) is deferred.
//!
//! # Diagnostics
//!
//! Parse failures return a [`ManifestError`] that converts into the
//! shared [`crate::hooks::diagnostic::HookDiagnostic`] with
//! [`DiagnosticClass::ManifestParse`] and the five-part body mandated
//! by CON-3225.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer};

use crate::ecosystems::manifest::{
    self as eco_manifest, EcosystemManifestError, EcosystemSpecific, BASE_MANIFEST_KEYS,
};
use crate::hooks::diagnostic::{DiagnosticClass, HookDiagnostic};
use crate::hooks::pipeline::Stage;
use crate::hooks::translators::AstType;

// ── Public types ───────────────────────────────────────────────────────────

/// Execution mode declared by the manifest (REQ-3203).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    OneShot,
    Persistent,
}

impl Default for Mode {
    fn default() -> Self {
        Mode::OneShot
    }
}

impl Mode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Mode::OneShot => "one-shot",
            Mode::Persistent => "persistent",
        }
    }
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Policy for combining `content_probe` hits (REQ-3203 / CON-3204).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProbeMatch {
    Any,
    All,
}

impl Default for ProbeMatch {
    fn default() -> Self {
        ProbeMatch::Any
    }
}

impl ProbeMatch {
    pub const fn as_str(self) -> &'static str {
        match self {
            ProbeMatch::Any => "any",
            ProbeMatch::All => "all",
        }
    }
}

impl std::fmt::Display for ProbeMatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Selector — the path / frontmatter / content filter that decides whether
/// a given page triggers this hook (REQ-3203 grammar, REQ-3204 evaluation
/// order).
#[derive(Debug, Clone, PartialEq)]
pub struct SelectorSpec {
    /// Vault-relative glob patterns; a page's path must match at least one
    /// of these (REQ-3204 step 1). Defaults to `["**/*.md"]` when the
    /// manifest omits the field.
    pub include: Vec<String>,
    /// Exclusion globs applied after `include`. Any match disqualifies
    /// the page. Empty by default.
    pub exclude: Vec<String>,
    /// Optional predicate over parsed frontmatter (REQ-3205 grammar).
    /// Evaluation is deferred to `task-selector-eval`; this parser only
    /// captures the string.
    pub frontmatter_where: Option<String>,
    /// Substring or regex probes tested against the raw Markdown text
    /// (REQ-3204 step 3). Each probe is a literal or `"re:<pattern>"`.
    /// Empty by default.
    pub content_probe: Vec<String>,
    /// Combine policy for [`Self::content_probe`].
    pub require_probe_match: ProbeMatch,
}

impl Default for SelectorSpec {
    fn default() -> Self {
        Self {
            include: default_include(),
            exclude: Vec::new(),
            frontmatter_where: None,
            content_probe: Vec::new(),
            require_probe_match: ProbeMatch::Any,
        }
    }
}

/// Behavioural-contract declaration (REQ-3224 / CON-3224).
///
/// Tier-1 fields (`preserves`, `idempotent`, `may_restructure`) are
/// enforced in v1. Tier-2 fields (`pure`, `expansion_bound`) are
/// reserved: v1 surfaces them in `zetl theme show` / `hook coverage`
/// as trust labels without runtime enforcement.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ContractDecl {
    /// AST node-type names the hook promises not to strip. Enforced by
    /// post-hook marker-strip detection (REQ-3221 / REQ-3224).
    pub preserves: Vec<String>,
    /// Hook asserts `f(f(x)) == f(x)` under NFR-3305 equivalence.
    pub idempotent: bool,
    /// Pre-parse stage only — REQ-3222 block-tree-shape check opts out
    /// of its default-on detector.
    pub may_restructure: bool,
    /// Reserved for v1.1 sandboxing (REQ-3224 Tier-2). Advisory in v1.
    pub pure: bool,
    /// Reserved for v1.1 output-size enforcement (REQ-3224 Tier-2).
    pub expansion_bound: Option<f32>,
}

/// Fully-parsed hook manifest.
///
/// A manifest-less hook SHALL behave as if an empty manifest were
/// present — callers can construct `Manifest::default()` to model that
/// case (REQ-3203). [`LoadedManifest`] distinguishes "file is missing
/// on disk" from "file is present and parses to defaults" so downstream
/// code can emit the REQ-3203 missing-manifest warning.
#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    /// Declared stage. `None` means the manifest omitted the field and
    /// zetl must infer it from the parent `<stage>.d/` directory
    /// (CON-3203).
    pub stage: Option<Stage>,
    /// Execution mode — one-shot subprocess or persistent long-lived
    /// process (CON-3201). Default `one-shot`.
    pub mode: Mode,
    /// Per-invocation wall-clock budget in milliseconds. Default 100.
    pub timeout_ms: u32,
    /// Memory budget in mebibytes. Default 64.
    pub memory_mib: u32,
    /// AST ecosystem this hook operates against (REQ-3221). Default
    /// [`AstType::ZetlExt`].
    pub ast_type: AstType,
    /// npm-style semver range against [`Self::ast_type`]'s version
    /// scheme (REQ-3221). `None` means no range declared.
    pub ast_version: Option<String>,
    /// Override for the default filename-derived extension id (REQ-3214 /
    /// REQ-3217). `None` means "use the default".
    pub extension_id: Option<String>,
    /// When true, a missing `before` / `after` reference drops its
    /// constraint with a warning instead of erroring (REQ-3217).
    pub optional: bool,
    /// Union of top-level `before` and `[ordering].before` (REQ-3217).
    /// The manifest accepts either spelling; callers only see the merged
    /// result.
    pub before: Vec<String>,
    /// Union of top-level `after` and `[ordering].after` (REQ-3217).
    pub after: Vec<String>,
    pub select: SelectorSpec,
    pub contract: ContractDecl,
    /// Per-ecosystem manifest block (SPEC-033 REQ-3312 / CON-3312).
    /// `None` when the manifest does not declare an `ecosystem = "..."`
    /// field — those hooks are strict SPEC-032 zetl-native hooks and
    /// carry no adapter-specific fields.
    pub extra: Option<EcosystemSpecific>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            stage: None,
            mode: Mode::default(),
            timeout_ms: DEFAULT_TIMEOUT_MS,
            memory_mib: DEFAULT_MEMORY_MIB,
            ast_type: AstType::default(),
            ast_version: None,
            extension_id: None,
            optional: false,
            before: Vec::new(),
            after: Vec::new(),
            select: SelectorSpec::default(),
            contract: ContractDecl::default(),
            extra: None,
        }
    }
}

/// Result of loading a manifest file from disk.
///
/// `Missing` means no sidecar `.toml` exists next to the executable —
/// REQ-3203 says zetl SHALL still run the hook with default selector
/// and emit a "recommend a manifest" warning. `Present` carries the
/// parsed result.
#[derive(Debug, Clone, PartialEq)]
pub enum LoadedManifest {
    Missing,
    Present(Manifest),
}

impl LoadedManifest {
    /// Extract the manifest, substituting defaults for the `Missing`
    /// case. Drops the "present vs missing" distinction — use the enum
    /// variants directly when that matters (e.g. for the REQ-3203
    /// missing-manifest warning).
    pub fn into_manifest(self) -> Manifest {
        match self {
            LoadedManifest::Missing => Manifest::default(),
            LoadedManifest::Present(m) => m,
        }
    }
}

pub const DEFAULT_TIMEOUT_MS: u32 = 100;
pub const DEFAULT_MEMORY_MIB: u32 = 64;

// ── Errors ──────────────────────────────────────────────────────────────────

/// Typed errors surfaced by the manifest parser.
///
/// Each variant converts into a [`HookDiagnostic`] with class
/// [`DiagnosticClass::ManifestParse`] via the `From` impl below, so
/// callers that just want to render a diagnostic do not need to match
/// on the variants. Variants stay exposed for callers that branch on
/// the error class (e.g. the selector-eval and ecosystem-check paths).
#[derive(Debug)]
pub enum ManifestError {
    /// TOML lexing or deserialisation failed — includes unknown fields,
    /// invalid enum values, syntax errors, and type mismatches. The
    /// toml crate's error message carries line/column information when
    /// available.
    Parse {
        path: Option<PathBuf>,
        message: String,
    },
    /// Filesystem read failed for a reason other than "not found"
    /// (which [`load_manifest`] maps to [`LoadedManifest::Missing`]).
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl ManifestError {
    pub fn path(&self) -> Option<&Path> {
        match self {
            ManifestError::Parse { path, .. } => path.as_deref(),
            ManifestError::Io { path, .. } => Some(path),
        }
    }

    /// Render into the shared [`HookDiagnostic`] surface. The five-part
    /// body matches CON-3225's manifest-parse example.
    pub fn to_diagnostic(&self) -> HookDiagnostic {
        match self {
            ManifestError::Parse { path, message } => {
                let summary = match path {
                    Some(p) => format!("hook manifest error at {}", p.display()),
                    None => "hook manifest error".to_string(),
                };
                let mut d = HookDiagnostic::new(DiagnosticClass::ManifestParse, summary)
                    .with_context(message.clone());
                d = d.with_hint(
                    "valid top-level fields: stage, mode, timeout_ms, memory_mib,\n\
                     ast_type, ast_version, extension_id, optional, before, after,\n\
                     ordering, select, contract, ecosystem. See:\n  \
                     https://zetl.codeberg.page/docs/hook-authoring/manifest-fields",
                );
                d
            }
            ManifestError::Io { path, source } => {
                HookDiagnostic::new(
                    DiagnosticClass::ManifestParse,
                    format!("cannot read hook manifest {}", path.display()),
                )
                .with_context(source.to_string())
            }
        }
    }
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::Parse { path, message } => match path {
                Some(p) => write!(f, "failed to parse hook manifest {}: {message}", p.display()),
                None => write!(f, "failed to parse hook manifest: {message}"),
            },
            ManifestError::Io { path, source } => {
                write!(f, "cannot read hook manifest {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for ManifestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ManifestError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<ManifestError> for HookDiagnostic {
    fn from(e: ManifestError) -> Self {
        e.to_diagnostic()
    }
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Parse a manifest from its raw TOML text.
///
/// `path_hint`, when supplied, is attached to any resulting error so
/// diagnostics can name the offending file. Passing `None` is fine for
/// in-memory parsing (e.g. tests, `zetl hook new`'s dry-run preview).
///
/// When the manifest declares `ecosystem = "..."`, every top-level key
/// outside [`crate::ecosystems::manifest::BASE_MANIFEST_KEYS`] is
/// extracted into a sub-table and handed to
/// [`crate::ecosystems::manifest::parse`] for deserialisation as the
/// matching [`EcosystemSpecific`] variant. That way a remark-only
/// field on a pandoc manifest surfaces as an unknown-field parse error
/// pointing at the ecosystem block, not at the SPEC-032 base parser
/// (SPEC-033 REQ-3312 / TEST-3312).
pub fn parse_manifest(text: &str, path_hint: Option<&Path>) -> Result<Manifest, ManifestError> {
    let table: toml::value::Table =
        toml::from_str(text).map_err(|e| ManifestError::Parse {
            path: path_hint.map(Path::to_path_buf),
            message: e.to_string(),
        })?;

    let (base_table, eco_table) = split_base_and_ecosystem(table);

    let extra = match eco_table {
        Some(t) => Some(eco_manifest::parse(t).map_err(|e| {
            ManifestError::Parse {
                path: path_hint.map(Path::to_path_buf),
                message: format_ecosystem_error(&e),
            }
        })?),
        None => None,
    };

    let wire: ManifestWire = toml::Value::Table(base_table)
        .try_into()
        .map_err(|e: toml::de::Error| ManifestError::Parse {
            path: path_hint.map(Path::to_path_buf),
            message: e.to_string(),
        })?;

    let mut manifest = wire.into_manifest();
    manifest.extra = extra;
    Ok(manifest)
}

/// Split a raw manifest table into the SPEC-032 base half and the
/// SPEC-033 ecosystem half (REQ-3312).
///
/// The split is keyed on the presence of an `ecosystem = "..."` entry:
/// - absent → every key stays in the base table, returned `(base, None)`.
/// - present → `ecosystem` plus every non-base top-level key moves into
///   the ecosystem table, returned `(base, Some(eco))`.
///
/// Non-base keys that slipped past the SPEC-032 parser's
/// `deny_unknown_fields` (because we extracted them) still fail at the
/// ecosystem layer: each variant struct has its own `deny_unknown_fields`,
/// so typos and cross-ecosystem fields surface as parse errors there.
fn split_base_and_ecosystem(
    mut table: toml::value::Table,
) -> (toml::value::Table, Option<toml::value::Table>) {
    if !table.contains_key("ecosystem") {
        return (table, None);
    }

    let mut eco = toml::value::Table::new();
    let keys: Vec<String> = table.keys().cloned().collect();
    for key in keys {
        if !BASE_MANIFEST_KEYS.contains(&key.as_str()) {
            if let Some(v) = table.remove(&key) {
                eco.insert(key, v);
            }
        }
    }

    (table, Some(eco))
}

fn format_ecosystem_error(err: &EcosystemManifestError) -> String {
    match err {
        EcosystemManifestError::Parse(msg) => msg.clone(),
        other => other.to_string(),
    }
}

/// Load and parse the manifest at `path`, treating "file not found" as
/// [`LoadedManifest::Missing`] (REQ-3203 default behaviour).
///
/// Any other I/O error surfaces as [`ManifestError::Io`]; parse failures
/// surface as [`ManifestError::Parse`] with `path` set.
pub fn load_manifest(path: &Path) -> Result<LoadedManifest, ManifestError> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LoadedManifest::Missing);
        }
        Err(e) => {
            return Err(ManifestError::Io {
                path: path.to_path_buf(),
                source: e,
            });
        }
    };
    parse_manifest(&text, Some(path)).map(LoadedManifest::Present)
}

// ── Wire types ──────────────────────────────────────────────────────────────
//
// The serde-facing shape. Public callers see [`Manifest`] with plain
// defaults applied and the top-level / `[ordering]` before+after spellings
// already merged.

fn default_include() -> Vec<String> {
    vec!["**/*.md".to_string()]
}

#[derive(Deserialize)]
#[serde(remote = "Stage", rename_all = "kebab-case")]
enum StageWire {
    PreParse,
    Transform,
    PostRender,
}

fn de_stage<'de, D>(deserializer: D) -> Result<Option<Stage>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    struct Wrapper(#[serde(with = "StageWire")] Stage);
    Option::<Wrapper>::deserialize(deserializer).map(|o| o.map(|Wrapper(s)| s))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestWire {
    #[serde(default, deserialize_with = "de_stage")]
    stage: Option<Stage>,
    #[serde(default)]
    mode: Option<Mode>,
    #[serde(default)]
    timeout_ms: Option<u32>,
    #[serde(default)]
    memory_mib: Option<u32>,
    #[serde(default)]
    ast_type: Option<AstType>,
    #[serde(default)]
    ast_version: Option<String>,
    #[serde(default)]
    extension_id: Option<String>,
    #[serde(default)]
    optional: Option<bool>,
    #[serde(default)]
    before: Vec<String>,
    #[serde(default)]
    after: Vec<String>,
    #[serde(default)]
    ordering: Option<OrderingWire>,
    #[serde(default)]
    select: Option<SelectWire>,
    #[serde(default)]
    contract: Option<ContractWire>,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct OrderingWire {
    before: Vec<String>,
    after: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct SelectWire {
    include: Option<Vec<String>>,
    exclude: Vec<String>,
    frontmatter_where: Option<String>,
    content_probe: Vec<String>,
    require_probe_match: Option<ProbeMatch>,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct ContractWire {
    preserves: Vec<String>,
    idempotent: bool,
    may_restructure: bool,
    pure: bool,
    expansion_bound: Option<f32>,
}

impl ManifestWire {
    fn into_manifest(self) -> Manifest {
        let (ordering_before, ordering_after) = match self.ordering {
            Some(o) => (o.before, o.after),
            None => (Vec::new(), Vec::new()),
        };
        // Merge [ordering] with top-level aliases — canonical form first,
        // alias second, preserving duplicates so authors migrating one
        // side at a time don't silently lose constraints (matches
        // composition.rs resolved_before/after).
        let mut before = ordering_before;
        before.extend(self.before);
        let mut after = ordering_after;
        after.extend(self.after);

        let select = self
            .select
            .map(|s| SelectorSpec {
                include: s.include.unwrap_or_else(default_include),
                exclude: s.exclude,
                frontmatter_where: s.frontmatter_where,
                content_probe: s.content_probe,
                require_probe_match: s.require_probe_match.unwrap_or_default(),
            })
            .unwrap_or_default();

        let contract = self
            .contract
            .map(|c| ContractDecl {
                preserves: c.preserves,
                idempotent: c.idempotent,
                may_restructure: c.may_restructure,
                pure: c.pure,
                expansion_bound: c.expansion_bound,
            })
            .unwrap_or_default();

        Manifest {
            stage: self.stage,
            mode: self.mode.unwrap_or_default(),
            timeout_ms: self.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS),
            memory_mib: self.memory_mib.unwrap_or(DEFAULT_MEMORY_MIB),
            ast_type: self.ast_type.unwrap_or_default(),
            ast_version: self.ast_version,
            extension_id: self.extension_id,
            optional: self.optional.unwrap_or(false),
            before,
            after,
            select,
            contract,
            // `extra` is populated by parse_manifest after the base
            // wire pass, not derived from ManifestWire.
            extra: None,
        }
    }
}

// ── Tests (TEST-3203 matrix) ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // ── Matrix row 1: valid manifest, all fields ──────────────────────────

    #[test]
    fn all_fields_roundtrip_through_parser() {
        let text = r#"
stage = "transform"
mode = "persistent"
timeout_ms = 250
memory_mib = 128
ast_type = "pandoc-ext"
ast_version = ">=1.22 <2"
extension_id = "my_callouts"
optional = true
before = ["admonition"]
after = ["prelude"]

[ordering]
before = ["tasks"]
after = ["callouts"]

[select]
include = ["**/*.md", "posts/**/*.md"]
exclude = ["drafts/**"]
frontmatter_where = 'status == "published" && !draft'
content_probe = [":::note", "re:^TODO"]
require_probe_match = "all"

[contract]
preserves = ["Wikilink", "Embed"]
idempotent = true
may_restructure = false
pure = true
expansion_bound = 3.0
"#;
        let m = parse_manifest(text, None).unwrap();
        assert_eq!(m.stage, Some(Stage::Transform));
        assert_eq!(m.mode, Mode::Persistent);
        assert_eq!(m.timeout_ms, 250);
        assert_eq!(m.memory_mib, 128);
        assert_eq!(m.ast_type, AstType::PandocExt);
        assert_eq!(m.ast_version.as_deref(), Some(">=1.22 <2"));
        assert_eq!(m.extension_id.as_deref(), Some("my_callouts"));
        assert!(m.optional);
        // before/after merge [ordering] then top-level alias.
        assert_eq!(m.before, vec!["tasks".to_string(), "admonition".to_string()]);
        assert_eq!(m.after, vec!["callouts".to_string(), "prelude".to_string()]);
        assert_eq!(
            m.select.include,
            vec!["**/*.md".to_string(), "posts/**/*.md".to_string()]
        );
        assert_eq!(m.select.exclude, vec!["drafts/**".to_string()]);
        assert_eq!(
            m.select.frontmatter_where.as_deref(),
            Some("status == \"published\" && !draft")
        );
        assert_eq!(
            m.select.content_probe,
            vec![":::note".to_string(), "re:^TODO".to_string()]
        );
        assert_eq!(m.select.require_probe_match, ProbeMatch::All);
        assert_eq!(
            m.contract.preserves,
            vec!["Wikilink".to_string(), "Embed".to_string()]
        );
        assert!(m.contract.idempotent);
        assert!(!m.contract.may_restructure);
        assert!(m.contract.pure);
        assert_eq!(m.contract.expansion_bound, Some(3.0));
    }

    // ── Matrix row 2: valid manifest, minimal fields ──────────────────────

    #[test]
    fn empty_manifest_yields_all_defaults() {
        let m = parse_manifest("", None).unwrap();
        assert_eq!(m, Manifest::default());
        assert_eq!(m.stage, None);
        assert_eq!(m.mode, Mode::OneShot);
        assert_eq!(m.timeout_ms, DEFAULT_TIMEOUT_MS);
        assert_eq!(m.memory_mib, DEFAULT_MEMORY_MIB);
        assert_eq!(m.ast_type, AstType::ZetlExt);
        assert!(m.ast_version.is_none());
        assert!(!m.optional);
        assert!(m.before.is_empty());
        assert!(m.after.is_empty());
        assert_eq!(m.select.include, vec!["**/*.md".to_string()]);
        assert!(m.select.exclude.is_empty());
        assert_eq!(m.select.require_probe_match, ProbeMatch::Any);
        assert!(m.contract.preserves.is_empty());
        assert!(!m.contract.idempotent);
    }

    #[test]
    fn single_field_leaves_other_defaults_intact() {
        let m = parse_manifest(r#"stage = "transform""#, None).unwrap();
        assert_eq!(m.stage, Some(Stage::Transform));
        assert_eq!(m.mode, Mode::OneShot);
        assert_eq!(m.timeout_ms, DEFAULT_TIMEOUT_MS);
        // Defaults for select still apply.
        assert_eq!(m.select.include, vec!["**/*.md".to_string()]);
    }

    #[test]
    fn stage_enum_accepts_every_variant() {
        for (spelling, expected) in [
            ("pre-parse", Stage::PreParse),
            ("transform", Stage::Transform),
            ("post-render", Stage::PostRender),
        ] {
            let text = format!("stage = \"{spelling}\"");
            let m = parse_manifest(&text, None).unwrap();
            assert_eq!(m.stage, Some(expected), "spelling={spelling}");
        }
    }

    #[test]
    fn mode_enum_accepts_every_variant() {
        let one_shot = parse_manifest(r#"mode = "one-shot""#, None).unwrap();
        assert_eq!(one_shot.mode, Mode::OneShot);
        let persistent = parse_manifest(r#"mode = "persistent""#, None).unwrap();
        assert_eq!(persistent.mode, Mode::Persistent);
    }

    #[test]
    fn ast_type_short_aliases_accepted() {
        let m = parse_manifest(r#"ast_type = "pandoc-types""#, None).unwrap();
        assert_eq!(m.ast_type, AstType::PandocExt);
        let m = parse_manifest(r#"ast_type = "mdast""#, None).unwrap();
        assert_eq!(m.ast_type, AstType::MdastExt);
    }

    // ── Matrix row 3: missing manifest ────────────────────────────────────

    #[test]
    fn load_manifest_returns_missing_when_file_absent() {
        let tmp = TempDir::new().unwrap();
        let absent = tmp.path().join("nope.toml");
        let result = load_manifest(&absent).unwrap();
        assert_eq!(result, LoadedManifest::Missing);
        assert_eq!(result.into_manifest(), Manifest::default());
    }

    #[test]
    fn load_manifest_returns_present_for_existing_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("hook.toml");
        fs::write(&path, r#"timeout_ms = 500"#).unwrap();
        let result = load_manifest(&path).unwrap();
        match result {
            LoadedManifest::Present(m) => assert_eq!(m.timeout_ms, 500),
            LoadedManifest::Missing => panic!("expected Present, got Missing"),
        }
    }

    // ── Matrix row 4: unknown field ───────────────────────────────────────

    #[test]
    fn unknown_top_level_field_rejected() {
        let text = r#"ecosystm = "pandoc""#; // typo for "ecosystem"
        let err = parse_manifest(text, None).unwrap_err();
        match err {
            ManifestError::Parse { message, .. } => {
                assert!(
                    message.contains("unknown field") || message.contains("ecosystm"),
                    "expected unknown-field error, got: {message}"
                );
            }
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn unknown_nested_field_rejected() {
        let text = r#"
[select]
includ = ["**/*.md"]
"#;
        let err = parse_manifest(text, None).unwrap_err();
        match err {
            ManifestError::Parse { message, .. } => {
                assert!(message.contains("unknown field") || message.contains("includ"));
            }
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn unknown_contract_field_rejected() {
        let text = r#"
[contract]
preserves_xxx = ["Wikilink"]
"#;
        let err = parse_manifest(text, None).unwrap_err();
        match err {
            ManifestError::Parse { .. } => {}
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn unknown_ordering_field_rejected() {
        let text = r#"
[ordering]
sideways = ["foo"]
"#;
        let err = parse_manifest(text, None).unwrap_err();
        match err {
            ManifestError::Parse { .. } => {}
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    // ── Matrix row 5: invalid `stage` value ───────────────────────────────

    #[test]
    fn invalid_stage_value_rejected() {
        let err = parse_manifest(r#"stage = "middle""#, None).unwrap_err();
        match err {
            ManifestError::Parse { message, .. } => {
                assert!(message.contains("middle") || message.contains("variant"));
            }
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn invalid_mode_value_rejected() {
        let err = parse_manifest(r#"mode = "async""#, None).unwrap_err();
        match err {
            ManifestError::Parse { .. } => {}
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn invalid_ast_type_rejected() {
        let err = parse_manifest(r#"ast_type = "djot""#, None).unwrap_err();
        match err {
            ManifestError::Parse { message, .. } => {
                assert!(message.contains("djot") || message.contains("variant"));
            }
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn invalid_probe_match_rejected() {
        let err = parse_manifest(
            r#"
[select]
require_probe_match = "some"
"#,
            None,
        )
        .unwrap_err();
        match err {
            ManifestError::Parse { .. } => {}
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    // ── Matrix row 6: invalid TOML syntax ─────────────────────────────────

    #[test]
    fn malformed_toml_surfaces_line_info() {
        let text = "this is = not = valid toml\n";
        let err = parse_manifest(text, None).unwrap_err();
        match err {
            ManifestError::Parse { message, .. } => {
                // toml 0.8 emits "at line N, column M" in its error text.
                assert!(
                    message.contains("line") || message.contains("column"),
                    "expected line/column info, got: {message}"
                );
            }
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn wrong_type_rejected_with_diagnostic() {
        // timeout_ms expects an integer; passing a string is a type error.
        let err = parse_manifest(r#"timeout_ms = "slow""#, None).unwrap_err();
        match err {
            ManifestError::Parse { .. } => {}
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    // ── Ordering merge semantics (REQ-3217) ───────────────────────────────

    #[test]
    fn top_level_only_before_after() {
        let m = parse_manifest(
            r#"
before = ["x"]
after = ["y"]
"#,
            None,
        )
        .unwrap();
        assert_eq!(m.before, vec!["x".to_string()]);
        assert_eq!(m.after, vec!["y".to_string()]);
    }

    #[test]
    fn ordering_table_only_before_after() {
        let m = parse_manifest(
            r#"
[ordering]
before = ["x"]
after = ["y"]
"#,
            None,
        )
        .unwrap();
        assert_eq!(m.before, vec!["x".to_string()]);
        assert_eq!(m.after, vec!["y".to_string()]);
    }

    #[test]
    fn ordering_table_and_top_level_both_present_are_unioned() {
        let m = parse_manifest(
            r#"
before = ["top"]
after = ["bottom"]

[ordering]
before = ["canonical"]
after = ["canonical-after"]
"#,
            None,
        )
        .unwrap();
        // Canonical first, top-level alias second.
        assert_eq!(
            m.before,
            vec!["canonical".to_string(), "top".to_string()]
        );
        assert_eq!(
            m.after,
            vec!["canonical-after".to_string(), "bottom".to_string()]
        );
    }

    // ── Diagnostic rendering ──────────────────────────────────────────────

    #[test]
    fn parse_error_renders_through_hook_diagnostic() {
        let text = r#"garbage = "#;
        let err = parse_manifest(text, Some(Path::new("fake.toml"))).unwrap_err();
        let diag: HookDiagnostic = err.into();
        assert_eq!(diag.class, DiagnosticClass::ManifestParse);
        let rendered = diag.to_string();
        // Summary names the file; body carries the toml error.
        assert!(rendered.contains("fake.toml"));
        // The CON-3225 hint pointer lands in the output.
        assert!(rendered.contains("valid top-level fields"));
    }

    #[test]
    fn parse_error_without_path_still_renders() {
        let err = parse_manifest("garbage = ", None).unwrap_err();
        let diag: HookDiagnostic = err.into();
        assert_eq!(diag.class, DiagnosticClass::ManifestParse);
        let rendered = diag.to_string();
        assert!(rendered.starts_with("[zetl] "));
    }

    // ── Round-trip: the CON-3224 worked example parses ────────────────────

    #[test]
    fn con_3224_contract_example_parses_with_defaults() {
        // Taken verbatim from SPEC-032 CON-3224.
        let text = r#"
[contract]
preserves = ["Wikilink", "Embed", "CodeBlock", "FrontMatter"]
idempotent = true
may_restructure = false
"#;
        let m = parse_manifest(text, None).unwrap();
        assert_eq!(
            m.contract.preserves,
            vec![
                "Wikilink".to_string(),
                "Embed".to_string(),
                "CodeBlock".to_string(),
                "FrontMatter".to_string(),
            ]
        );
        assert!(m.contract.idempotent);
        assert!(!m.contract.may_restructure);
        // Tier-2 fields absent → default false / None.
        assert!(!m.contract.pure);
        assert!(m.contract.expansion_bound.is_none());
    }

    // ── SPEC-033 REQ-3312 / TEST-3312: ecosystem manifest block ──────────

    #[test]
    fn empty_manifest_has_no_extra_block() {
        let m = parse_manifest("", None).unwrap();
        assert!(m.extra.is_none());
    }

    #[test]
    fn base_only_manifest_has_no_extra_block() {
        // No `ecosystem` key ⇒ every top-level field belongs to SPEC-032.
        let m = parse_manifest(r#"stage = "transform""#, None).unwrap();
        assert!(m.extra.is_none());
    }

    #[test]
    fn pandoc_manifest_parses_with_base_and_ecosystem_fields() {
        let text = r#"
stage = "transform"
timeout_ms = 250
ecosystem = "pandoc"
exec = "pandoc-crossref"
args = ["--csl", "apa.csl"]

[select]
include = ["**/*.md"]
"#;
        let m = parse_manifest(text, None).unwrap();
        // Base fields still parsed.
        assert_eq!(m.stage, Some(Stage::Transform));
        assert_eq!(m.timeout_ms, 250);
        assert_eq!(m.select.include, vec!["**/*.md".to_string()]);
        // Ecosystem block populated.
        match m.extra.expect("ecosystem block should be present") {
            crate::ecosystems::manifest::EcosystemSpecific::Pandoc(p) => {
                assert_eq!(p.exec.as_deref(), Some("pandoc-crossref"));
                assert_eq!(p.args, vec!["--csl".to_string(), "apa.csl".to_string()]);
                assert!(p.lua_filter.is_none());
            }
            other => panic!("expected Pandoc variant, got {other:?}"),
        }
    }

    #[test]
    fn mdbook_manifest_parses_with_default_scope() {
        let text = r#"
stage = "pre-parse"
ecosystem = "mdbook"
exec = "mdbook-mermaid"
"#;
        let m = parse_manifest(text, None).unwrap();
        assert_eq!(m.stage, Some(Stage::PreParse));
        match m.extra.expect("ecosystem block should be present") {
            crate::ecosystems::manifest::EcosystemSpecific::Mdbook(b) => {
                assert_eq!(b.exec, "mdbook-mermaid");
                assert_eq!(
                    b.scope,
                    crate::ecosystems::manifest::MdbookScope::Page
                );
            }
            other => panic!("expected Mdbook variant, got {other:?}"),
        }
    }

    #[test]
    fn remark_manifest_parses_with_options() {
        let text = r#"
ecosystem = "remark"
package = "remark-gfm"
version = ">=3.0 <4"

[options]
singleTilde = false
"#;
        let m = parse_manifest(text, None).unwrap();
        match m.extra.expect("ecosystem block should be present") {
            crate::ecosystems::manifest::EcosystemSpecific::Remark(r) => {
                assert_eq!(r.package, "remark-gfm");
                assert_eq!(r.version.as_deref(), Some(">=3.0 <4"));
                assert_eq!(
                    r.options.get("singleTilde").and_then(|v| v.as_bool()),
                    Some(false)
                );
            }
            other => panic!("expected Remark variant, got {other:?}"),
        }
    }

    #[test]
    fn package_on_pandoc_manifest_rejected() {
        // TEST-3312 main assertion: remark-only field on pandoc manifest.
        let text = r#"
ecosystem = "pandoc"
exec = "pandoc-crossref"
package = "remark-gfm"
"#;
        let err = parse_manifest(text, None).unwrap_err();
        match err {
            ManifestError::Parse { message, .. } => {
                assert!(
                    message.contains("unknown field") || message.contains("package"),
                    "expected unknown-field error mentioning `package`, got: {message}"
                );
            }
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn pandoc_missing_exec_and_lua_filter_rejected() {
        let text = r#"
ecosystem = "pandoc"
args = ["--csl", "apa.csl"]
"#;
        let err = parse_manifest(text, None).unwrap_err();
        match err {
            ManifestError::Parse { message, .. } => {
                assert!(
                    message.contains("exec") || message.contains("lua_filter"),
                    "expected missing-exec error, got: {message}"
                );
            }
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn pandoc_exec_and_lua_filter_together_rejected() {
        let text = r#"
ecosystem = "pandoc"
exec = "pandoc-crossref"
lua_filter = "filters/counter.lua"
"#;
        let err = parse_manifest(text, None).unwrap_err();
        match err {
            ManifestError::Parse { message, .. } => {
                assert!(
                    message.contains("exec") && message.contains("lua_filter"),
                    "expected exec-xor-lua_filter error, got: {message}"
                );
            }
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn unknown_ecosystem_value_rejected() {
        let err = parse_manifest(r#"ecosystem = "djot""#, None).unwrap_err();
        match err {
            ManifestError::Parse { message, .. } => {
                assert!(
                    message.contains("djot")
                        || message.contains("unknown variant")
                        || message.contains("ecosystem"),
                    "expected unknown-variant error, got: {message}"
                );
            }
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn scope_on_pandoc_manifest_rejected() {
        // `scope` is mdBook-only.
        let text = r#"
ecosystem = "pandoc"
exec = "pandoc-crossref"
scope = "page"
"#;
        let err = parse_manifest(text, None).unwrap_err();
        match err {
            ManifestError::Parse { message, .. } => {
                assert!(message.contains("unknown field") || message.contains("scope"));
            }
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn exec_on_remark_manifest_rejected() {
        let text = r#"
ecosystem = "remark"
package = "remark-gfm"
exec = "pandoc-crossref"
"#;
        let err = parse_manifest(text, None).unwrap_err();
        match err {
            ManifestError::Parse { message, .. } => {
                assert!(message.contains("unknown field") || message.contains("exec"));
            }
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn zetl_native_ecosystem_allowed_with_base_fields() {
        // Declaring `ecosystem = "zetl-native"` explicitly must compose
        // with the base SPEC-032 fields — the extraction must not
        // swallow base keys or reject the manifest.
        let text = r#"
ecosystem = "zetl-native"
stage = "transform"
timeout_ms = 150
"#;
        let m = parse_manifest(text, None).unwrap();
        assert_eq!(m.stage, Some(Stage::Transform));
        assert_eq!(m.timeout_ms, 150);
        assert!(matches!(
            m.extra,
            Some(crate::ecosystems::manifest::EcosystemSpecific::ZetlNative(_))
        ));
    }

    #[test]
    fn ecosystem_block_survives_alongside_select_and_contract() {
        // Select and contract keys are in BASE_MANIFEST_KEYS and must
        // stay with the base parser even when `ecosystem = "..."` is
        // present. Exercises the key-split on a fully-populated manifest.
        let text = r#"
stage = "transform"
ecosystem = "pandoc"
exec = "pandoc-crossref"

[select]
include = ["posts/**/*.md"]

[contract]
preserves = ["Wikilink"]
idempotent = true
"#;
        let m = parse_manifest(text, None).unwrap();
        assert_eq!(m.select.include, vec!["posts/**/*.md".to_string()]);
        assert_eq!(m.contract.preserves, vec!["Wikilink".to_string()]);
        assert!(m.contract.idempotent);
        assert!(matches!(
            m.extra,
            Some(crate::ecosystems::manifest::EcosystemSpecific::Pandoc(_))
        ));
    }

    #[test]
    fn req_3203_canonical_example_parses() {
        // Verbatim from SPEC-032 REQ-3203.
        let text = r#"
stage = "transform"
mode = "persistent"
timeout_ms = 100
memory_mib = 64
ast_type = "zetl-ext"
ast_version = ">=1.0 <2"

[select]
include = ["**/*.md"]
exclude = []
frontmatter_where = 'tags contains "project"'
content_probe = []
require_probe_match = "any"
"#;
        let m = parse_manifest(text, None).unwrap();
        assert_eq!(m.stage, Some(Stage::Transform));
        assert_eq!(m.mode, Mode::Persistent);
        assert_eq!(m.timeout_ms, 100);
        assert_eq!(m.memory_mib, 64);
        assert_eq!(m.ast_type, AstType::ZetlExt);
        assert_eq!(m.ast_version.as_deref(), Some(">=1.0 <2"));
    }
}
