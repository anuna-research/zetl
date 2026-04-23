//! Parser registry and per-page parser selection
//! (SPEC-033 REQ-3306 / CON-3306).
//!
//! Different pages in a vault can be parsed by different Markdown parsers
//! — typically `"commonmark"` (pulldown-cmark, ztl's default) or
//! `"pandoc"` (pandoc-types via the Pandoc adapter). A page can opt into
//! a specific parser via YAML frontmatter; a whole directory can default
//! to a parser via vault config globs; a whole vault can default via a
//! top-level config key.
//!
//! ## Precedence (CON-3306)
//!
//! ```text
//!   parser_name = frontmatter.parser                 # priority 1 (highest)
//!             ?? first_matching(config.parse.rule)   # priority 2
//!             ?? config.parse.default                # priority 3
//!             ?? "commonmark"                        # priority 4 (ztl default)
//! ```
//!
//! Resolution is pure — no I/O, no allocation on the hot path beyond the
//! returned name. Callers (the pre-parse stage wire) then look the name
//! up in a [`ParserRegistry`] and invoke [`Parser::parse`]. An unknown
//! name is a per-page error: the caller emits a REQ-3207 failure record
//! and skips the page rather than aborting the build.
//!
//! ## Why this lives outside `hooks::ast::parse`
//!
//! `hooks::ast::parse` is ztl's commonmark implementation. The parser
//! registry is the *dispatch* layer that sits in front of it. Additional
//! parsers (Pandoc today, Djot / asciidoc / … tomorrow) live alongside
//! `hooks::ast::parse` as siblings, all returning the same canonical
//! [`Document`] so the transform stage never has to care which parser
//! produced the AST.
//!
//! The Pandoc parser in v1 is a **stub**: it takes the registry slot so
//! that `parser: pandoc` resolves to a real parser and the selection
//! resolver + manifest round-trip stay honest, but invoking it returns
//! [`ParseError::RuntimeUnavailable`] with an actionable hint until
//! `task-pandoc-adapter` wires real Pandoc invocation.

pub mod mixed_eco;

pub use mixed_eco::{
    detect_mixed_parsers, ecosystem_expected_parser, format_report as format_mixed_parser_report,
    HookForDetection, MixedParserReport, MixedParserViolation, PageForDetection,
};

use std::collections::BTreeMap;
use std::path::Path;

use globset::{Glob, GlobMatcher};
use serde::{Deserialize, Serialize};

use crate::hooks::ast::{parse_markdown, Document, Frontmatter};

// ── Parser trait + built-ins ────────────────────────────────────────────────

/// A Markdown parser that produces the canonical ztl-ext [`Document`].
///
/// Implementations are pure: no I/O beyond what the registered parser
/// inherently needs (the commonmark parser has none; the pandoc parser
/// shells out to `pandoc` once its adapter lands). No global state.
pub trait Parser: Send + Sync {
    /// Stable registry id (`"commonmark"`, `"pandoc"`, …). Matches the
    /// name used in `[parse] default` and `frontmatter.parser`.
    fn id(&self) -> &str;

    /// Parse Markdown (with or without frontmatter) into a canonical
    /// ztl-ext [`Document`].
    ///
    /// Returns [`ParseError::RuntimeUnavailable`] when the parser exists
    /// in the registry but its runtime dependency isn't met (Pandoc
    /// binary missing, feature flag absent, etc.) — the caller skips the
    /// page and surfaces the hint.
    fn parse(&self, content: &str) -> Result<Document, ParseError>;
}

/// ztl's default parser (priority 4): pulldown-cmark via
/// [`crate::hooks::ast::parse_markdown`] with ztl extensions for
/// wikilinks, embeds, and SPL blocks.
#[derive(Debug, Default, Clone, Copy)]
pub struct CommonmarkParser;

impl Parser for CommonmarkParser {
    fn id(&self) -> &str {
        "commonmark"
    }

    fn parse(&self, content: &str) -> Result<Document, ParseError> {
        Ok(parse_markdown(content))
    }
}

/// Pandoc parser stub (SPEC-033 REQ-3306).
///
/// Registers the `"pandoc"` id so selection resolves and manifest
/// validation stays honest, but [`Parser::parse`] returns
/// [`ParseError::RuntimeUnavailable`] until `task-pandoc-adapter` wires
/// real `pandoc --from markdown --to json` invocation. The hint points
/// at `ztl ecosystem check`, which already knows how to diagnose the
/// pandoc runtime (REQ-3313).
#[derive(Debug, Default, Clone, Copy)]
pub struct PandocParser;

impl Parser for PandocParser {
    fn id(&self) -> &str {
        "pandoc"
    }

    fn parse(&self, _content: &str) -> Result<Document, ParseError> {
        Err(ParseError::RuntimeUnavailable {
            parser: "pandoc".to_string(),
            hint: "the Pandoc parser adapter has not landed in this ztl build; \
                   run `ztl ecosystem check` for runtime status, or set \
                   `parser: commonmark` to parse this page with the default parser"
                .to_string(),
        })
    }
}

// ── Registry ────────────────────────────────────────────────────────────────

/// Lookup table from parser id (e.g. `"commonmark"`) to a live
/// [`Parser`] implementation. Built once at pipeline init.
#[derive(Default)]
pub struct ParserRegistry {
    parsers: BTreeMap<String, Box<dyn Parser>>,
}

impl ParserRegistry {
    /// Empty registry. Callers add parsers via [`Self::register`]; use
    /// [`Self::with_builtins`] for the v1 default set.
    pub fn new() -> Self {
        Self {
            parsers: BTreeMap::new(),
        }
    }

    /// v1 defaults: commonmark (always) + pandoc (stub).
    ///
    /// `commonmark` is the ztl-default parser (priority 4); `pandoc`
    /// occupies the registry slot so selection resolves, returning
    /// [`ParseError::RuntimeUnavailable`] from [`Parser::parse`] until
    /// `task-pandoc-adapter` wires real invocation.
    pub fn with_builtins() -> Self {
        let mut reg = Self::new();
        reg.register(Box::new(CommonmarkParser));
        reg.register(Box::new(PandocParser));
        reg
    }

    /// Add a parser to the registry. Overwrites an existing entry with
    /// the same id (tests use this to swap in a stub); production code
    /// calls it at most once per id at init.
    pub fn register(&mut self, parser: Box<dyn Parser>) {
        let id = parser.id().to_string();
        self.parsers.insert(id, parser);
    }

    /// Look up a parser by id. Returns `None` when the id isn't
    /// registered; callers typically want [`Self::require`] instead,
    /// which produces the CON-3306 "unknown parser" diagnostic.
    pub fn get(&self, name: &str) -> Option<&dyn Parser> {
        self.parsers.get(name).map(|b| b.as_ref())
    }

    /// Look up a parser by id or produce [`ParseError::UnknownParser`]
    /// carrying the list of known names (for the diagnostic's "known:"
    /// line — CON-3225).
    pub fn require(&self, name: &str) -> Result<&dyn Parser, ParseError> {
        match self.get(name) {
            Some(p) => Ok(p),
            None => Err(ParseError::UnknownParser {
                name: name.to_string(),
                known: self.names(),
            }),
        }
    }

    /// All registered parser ids in canonical sort order, for
    /// diagnostics and help text.
    pub fn names(&self) -> Vec<String> {
        self.parsers.keys().cloned().collect()
    }

    /// Whether an id is registered.
    pub fn is_registered(&self, name: &str) -> bool {
        self.parsers.contains_key(name)
    }
}

impl std::fmt::Debug for ParserRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParserRegistry")
            .field("parsers", &self.names())
            .finish()
    }
}

// ── Config (`[parse]` table) ────────────────────────────────────────────────

/// Vault-level parse configuration — the `[parse]` table from
/// `.ztl/config.toml`.
///
/// ```toml
/// [parse]
/// default = "pandoc"
///
/// [[parse.rule]]
/// pattern = "papers/**"
/// parser = "pandoc"
///
/// [[parse.rule]]
/// pattern = "notes/**"
/// parser = "commonmark"
/// ```
///
/// Rules are evaluated in declaration order; the first pattern that
/// matches the vault-relative page path wins (CON-3306). Globs follow
/// the `globset` crate's conventions — `**` for recursive directories,
/// `*` for a path segment, `?` for a single character.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ParseConfig {
    /// Vault-wide default parser. `None` falls back to
    /// [`DEFAULT_PARSER`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// Directory-scoped rules, evaluated in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rule: Vec<ParserRule>,
}

/// ztl's intrinsic fallback (CON-3306 priority 4) — used when no
/// frontmatter, no matching rule, and no `[parse] default` apply.
pub const DEFAULT_PARSER: &str = "commonmark";

/// The raw TOML form of a single `[[parse.rule]]` entry. Compiled globs
/// are kept on [`CompiledParseConfig`] so the TOML round-trip stays
/// lossless.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ParserRule {
    /// glob pattern matched against the page's vault-relative path.
    pub pattern: String,
    /// Registry id of the parser to use.
    pub parser: String,
}

/// [`ParseConfig`] with its globs pre-compiled for the selection
/// resolver. Build once at pipeline init via [`ParseConfig::compile`].
#[derive(Debug)]
pub struct CompiledParseConfig {
    default: Option<String>,
    rules: Vec<CompiledRule>,
}

#[derive(Debug)]
struct CompiledRule {
    matcher: GlobMatcher,
    parser: String,
}

impl ParseConfig {
    /// Deserialise from an in-memory TOML string. Accepts either a full
    /// vault config (with `[parse] …` nested) or a bare `[parse]` block
    /// by looking up the nested `parse` key first.
    pub fn from_toml_str(s: &str) -> Result<Self, ParseConfigError> {
        #[derive(Deserialize)]
        struct Wrapper {
            #[serde(default)]
            parse: Option<ParseConfig>,
        }

        let wrapper: Wrapper =
            toml::from_str(s).map_err(|e| ParseConfigError::Malformed(e.to_string()))?;
        Ok(wrapper.parse.unwrap_or_default())
    }

    /// Load `.ztl/config.toml` from the vault and extract its
    /// `[parse]` block. Missing file → default `ParseConfig`; malformed
    /// → [`ParseConfigError`].
    pub fn load_from_vault(vault_root: &Path) -> Result<Self, ParseConfigError> {
        let path = vault_root.join(".ztl").join("config.toml");
        match std::fs::read_to_string(&path) {
            Ok(s) => Self::from_toml_str(&s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(ParseConfigError::Io(e.to_string())),
        }
    }

    /// Pre-compile every `[[parse.rule]]` glob so the resolver hot path
    /// runs allocation-free per page.
    pub fn compile(self) -> Result<CompiledParseConfig, ParseConfigError> {
        let mut rules = Vec::with_capacity(self.rule.len());
        for r in self.rule {
            let glob = Glob::new(&r.pattern).map_err(|e| ParseConfigError::BadPattern {
                pattern: r.pattern.clone(),
                detail: e.to_string(),
            })?;
            rules.push(CompiledRule {
                matcher: glob.compile_matcher(),
                parser: r.parser,
            });
        }
        Ok(CompiledParseConfig {
            default: self.default,
            rules,
        })
    }
}

impl CompiledParseConfig {
    /// Empty compiled config — the resolver falls back through every
    /// priority level and ends on [`DEFAULT_PARSER`].
    pub fn empty() -> Self {
        Self {
            default: None,
            rules: Vec::new(),
        }
    }

    /// Vault-wide default, if any.
    pub fn default_parser(&self) -> Option<&str> {
        self.default.as_deref()
    }

    /// First matching `[[parse.rule]]`'s parser id, if any.
    pub fn match_rule(&self, page_path: &Path) -> Option<&str> {
        self.rules
            .iter()
            .find(|r| r.matcher.is_match(page_path))
            .map(|r| r.parser.as_str())
    }
}

/// Failure surface for loading / compiling a [`ParseConfig`].
#[derive(Debug, Clone)]
pub enum ParseConfigError {
    /// `.ztl/config.toml` couldn't be read for a reason other than
    /// `NotFound` (permission denied, I/O, …).
    Io(String),
    /// TOML wasn't syntactically valid or didn't match the schema.
    Malformed(String),
    /// A `[[parse.rule]]` pattern failed to compile as a glob.
    BadPattern { pattern: String, detail: String },
}

impl std::fmt::Display for ParseConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseConfigError::Io(e) => write!(f, "could not read .ztl/config.toml: {e}"),
            ParseConfigError::Malformed(e) => {
                write!(f, "malformed [parse] config in .ztl/config.toml: {e}")
            }
            ParseConfigError::BadPattern { pattern, detail } => {
                write!(f, "invalid [[parse.rule]] pattern '{pattern}': {detail}")
            }
        }
    }
}

impl std::error::Error for ParseConfigError {}

// ── Selection resolver (CON-3306) ───────────────────────────────────────────

/// Resolve a parser id for a single page per CON-3306 precedence.
///
/// - `frontmatter` is the parsed YAML frontmatter object, or `None` if
///   the page has no frontmatter. The resolver looks for a top-level
///   `parser` string key.
/// - `page_path` is the vault-relative page path, used to evaluate
///   `[[parse.rule]]` globs.
/// - `config` is the compiled `[parse]` block.
///
/// Returns a freshly-allocated [`String`] rather than a `&str` because
/// the two highest-priority sources (frontmatter + rule-match) may
/// return borrowed data whose lifetime is scoped tighter than the
/// caller needs.
pub fn select_parser_name(
    frontmatter: Option<&Frontmatter>,
    page_path: &Path,
    config: &CompiledParseConfig,
) -> String {
    if let Some(fm_parser) = frontmatter.and_then(extract_parser_from_frontmatter) {
        return fm_parser;
    }
    if let Some(rule_parser) = config.match_rule(page_path) {
        return rule_parser.to_string();
    }
    if let Some(default) = config.default_parser() {
        return default.to_string();
    }
    DEFAULT_PARSER.to_string()
}

/// Pull a top-level `parser: <name>` field out of a parsed frontmatter
/// object. Non-string values are ignored (treated as absent) rather
/// than erroring: the selection resolver is cheap and pure, and a
/// malformed frontmatter shape would already have surfaced elsewhere
/// in the pipeline's frontmatter validation.
pub fn extract_parser_from_frontmatter(frontmatter: &Frontmatter) -> Option<String> {
    frontmatter
        .get("parser")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

// ── Error type ──────────────────────────────────────────────────────────────

/// Parser-layer errors surfaced to the pre-parse stage. Map into a
/// REQ-3207 [`crate::hooks::failure_scoping::FailureRecord`] at the
/// call site — this module doesn't own failure-scoping's taxonomy.
#[derive(Debug, Clone)]
pub enum ParseError {
    /// The selected parser id isn't in the registry — e.g. a page's
    /// `parser: djot` when only commonmark + pandoc are registered.
    /// Caller skips the page and emits the diagnostic.
    UnknownParser { name: String, known: Vec<String> },
    /// The selected parser is registered but its runtime dependency
    /// isn't met (e.g. Pandoc binary missing). Same user-visible
    /// behaviour as `UnknownParser` — skip the page, diagnose — but
    /// the hint points at `ztl ecosystem check` rather than the
    /// registry listing.
    RuntimeUnavailable { parser: String, hint: String },
    /// The parser accepted the content but failed mid-parse (malformed
    /// input, adapter crash, …).
    Failed { parser: String, detail: String },
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::UnknownParser { name, known } => {
                write!(f, "unknown parser '{name}' (known: [{}])", known.join(", "))
            }
            ParseError::RuntimeUnavailable { parser, hint } => {
                write!(f, "parser '{parser}' is unavailable: {hint}")
            }
            ParseError::Failed { parser, detail } => {
                write!(f, "parser '{parser}' failed: {detail}")
            }
        }
    }
}

impl std::error::Error for ParseError {}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fm(value: serde_json::Value) -> Frontmatter {
        match value {
            serde_json::Value::Object(m) => m,
            _ => panic!("test helper expects an object"),
        }
    }

    // ── Registry ───────────────────────────────────────────────────────

    #[test]
    fn builtin_registry_has_commonmark_and_pandoc() {
        let reg = ParserRegistry::with_builtins();
        assert!(reg.is_registered("commonmark"));
        assert!(reg.is_registered("pandoc"));
        assert!(!reg.is_registered("djot"));
    }

    #[test]
    fn registry_require_returns_parser_for_known_name() {
        let reg = ParserRegistry::with_builtins();
        let p = reg.require("commonmark").unwrap();
        assert_eq!(p.id(), "commonmark");
    }

    #[test]
    fn registry_require_errors_for_unknown() {
        let reg = ParserRegistry::with_builtins();
        let err = reg.require("djot").err().expect("expected Err");
        match err {
            ParseError::UnknownParser { name, known } => {
                assert_eq!(name, "djot");
                assert!(known.contains(&"commonmark".to_string()));
                assert!(known.contains(&"pandoc".to_string()));
            }
            other => panic!("expected UnknownParser, got {other:?}"),
        }
    }

    #[test]
    fn registry_names_are_sorted() {
        let reg = ParserRegistry::with_builtins();
        let names = reg.names();
        assert_eq!(names, vec!["commonmark".to_string(), "pandoc".to_string()]);
    }

    #[test]
    fn commonmark_parses_trivial_document() {
        let reg = ParserRegistry::with_builtins();
        let p = reg.require("commonmark").unwrap();
        let doc = p.parse("# Hello\n\nA paragraph.\n").unwrap();
        assert_eq!(doc.ast_version, crate::hooks::ast::AST_VERSION);
        assert!(!doc.children.is_empty());
    }

    #[test]
    fn pandoc_stub_surfaces_runtime_unavailable() {
        let reg = ParserRegistry::with_builtins();
        let p = reg.require("pandoc").unwrap();
        let err = p.parse("# anything").unwrap_err();
        match err {
            ParseError::RuntimeUnavailable { parser, hint } => {
                assert_eq!(parser, "pandoc");
                assert!(hint.contains("ztl ecosystem check"));
            }
            other => panic!("expected RuntimeUnavailable, got {other:?}"),
        }
    }

    // ── Config parsing ─────────────────────────────────────────────────

    #[test]
    fn parse_config_round_trips_through_toml() {
        let src = r#"
[parse]
default = "pandoc"

[[parse.rule]]
pattern = "papers/**"
parser = "pandoc"

[[parse.rule]]
pattern = "notes/**"
parser = "commonmark"
"#;
        let cfg = ParseConfig::from_toml_str(src).unwrap();
        assert_eq!(cfg.default.as_deref(), Some("pandoc"));
        assert_eq!(cfg.rule.len(), 2);
        assert_eq!(cfg.rule[0].pattern, "papers/**");
        assert_eq!(cfg.rule[0].parser, "pandoc");
        assert_eq!(cfg.rule[1].pattern, "notes/**");
        assert_eq!(cfg.rule[1].parser, "commonmark");
    }

    #[test]
    fn parse_config_empty_string_is_default() {
        let cfg = ParseConfig::from_toml_str("").unwrap();
        assert!(cfg.default.is_none());
        assert!(cfg.rule.is_empty());
    }

    #[test]
    fn parse_config_rejects_unknown_fields() {
        let src = r#"
[parse]
default = "pandoc"
unknown_field = 42
"#;
        let err = ParseConfig::from_toml_str(src).unwrap_err();
        assert!(matches!(err, ParseConfigError::Malformed(_)));
    }

    #[test]
    fn bad_glob_surfaces_bad_pattern_error() {
        let src = r#"
[parse]
default = "commonmark"

[[parse.rule]]
pattern = "["
parser = "pandoc"
"#;
        let cfg = ParseConfig::from_toml_str(src).unwrap();
        let err = cfg.compile().unwrap_err();
        match err {
            ParseConfigError::BadPattern { pattern, .. } => {
                assert_eq!(pattern, "[");
            }
            other => panic!("expected BadPattern, got {other:?}"),
        }
    }

    // ── TEST-3306 matrix (CON-3306 precedence) ─────────────────────────

    // Row 1: No frontmatter, no rule, no default → commonmark
    #[test]
    fn resolves_to_ztl_default_commonmark_when_nothing_configured() {
        let config = CompiledParseConfig::empty();
        let name = select_parser_name(None, Path::new("index.md"), &config);
        assert_eq!(name, "commonmark");
    }

    // Row 2: [parse] default = "pandoc" → pandoc
    #[test]
    fn resolves_to_vault_default_when_no_rule_or_frontmatter() {
        let config = ParseConfig::from_toml_str(
            r#"
[parse]
default = "pandoc"
"#,
        )
        .unwrap()
        .compile()
        .unwrap();
        let name = select_parser_name(None, Path::new("index.md"), &config);
        assert_eq!(name, "pandoc");
    }

    // Row 3: [[parse.rule]] matches page → rule's parser
    #[test]
    fn resolves_to_first_matching_rule_overriding_default() {
        let config = ParseConfig::from_toml_str(
            r#"
[parse]
default = "commonmark"

[[parse.rule]]
pattern = "papers/**"
parser = "pandoc"
"#,
        )
        .unwrap()
        .compile()
        .unwrap();
        let name = select_parser_name(None, Path::new("papers/whitepaper.md"), &config);
        assert_eq!(name, "pandoc");

        // Sibling directory falls through to the vault default.
        let name = select_parser_name(None, Path::new("notes/journal.md"), &config);
        assert_eq!(name, "commonmark");
    }

    // Row 4: frontmatter `parser: commonmark` beats vault default `pandoc`
    #[test]
    fn frontmatter_beats_every_other_source() {
        let config = ParseConfig::from_toml_str(
            r#"
[parse]
default = "pandoc"

[[parse.rule]]
pattern = "papers/**"
parser = "pandoc"
"#,
        )
        .unwrap()
        .compile()
        .unwrap();
        let frontmatter = fm(json!({ "parser": "commonmark" }));
        let name = select_parser_name(
            Some(&frontmatter),
            Path::new("papers/whitepaper.md"),
            &config,
        );
        assert_eq!(name, "commonmark");
    }

    // Row 5: frontmatter `parser: djot` resolves to "djot" which the
    // registry rejects → UnknownParser error at lookup time.
    #[test]
    fn unknown_frontmatter_parser_surfaces_at_registry_lookup() {
        let config = CompiledParseConfig::empty();
        let frontmatter = fm(json!({ "parser": "djot" }));
        let name = select_parser_name(Some(&frontmatter), Path::new("index.md"), &config);
        assert_eq!(name, "djot");

        let reg = ParserRegistry::with_builtins();
        let err = reg.require(&name).err().expect("expected Err");
        assert!(matches!(err, ParseError::UnknownParser { .. }));
    }

    // ── Additional resolver edge cases ─────────────────────────────────

    #[test]
    fn non_string_frontmatter_parser_is_ignored() {
        let config = CompiledParseConfig::empty();
        // Integer parser: should be ignored and fall through to default.
        let frontmatter = fm(json!({ "parser": 42 }));
        let name = select_parser_name(Some(&frontmatter), Path::new("x.md"), &config);
        assert_eq!(name, "commonmark");
    }

    #[test]
    fn rules_evaluate_in_declaration_order() {
        // Two rules both match `papers/foo.md`; first one (pandoc) wins.
        let config = ParseConfig::from_toml_str(
            r#"
[parse]
default = "commonmark"

[[parse.rule]]
pattern = "papers/**"
parser = "pandoc"

[[parse.rule]]
pattern = "**/*.md"
parser = "commonmark"
"#,
        )
        .unwrap()
        .compile()
        .unwrap();
        let name = select_parser_name(None, Path::new("papers/x.md"), &config);
        assert_eq!(name, "pandoc");
        // Rule ordering reverses: commonmark-first wins globally.
        let config = ParseConfig::from_toml_str(
            r#"
[parse]
default = "commonmark"

[[parse.rule]]
pattern = "**/*.md"
parser = "commonmark"

[[parse.rule]]
pattern = "papers/**"
parser = "pandoc"
"#,
        )
        .unwrap()
        .compile()
        .unwrap();
        let name = select_parser_name(None, Path::new("papers/x.md"), &config);
        assert_eq!(name, "commonmark");
    }

    #[test]
    fn load_from_vault_returns_default_when_file_missing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let cfg = ParseConfig::load_from_vault(tmp.path()).unwrap();
        assert!(cfg.default.is_none());
        assert!(cfg.rule.is_empty());
    }

    #[test]
    fn load_from_vault_reads_parse_block() {
        let tmp = tempfile::TempDir::new().unwrap();
        let cfg_dir = tmp.path().join(".ztl");
        std::fs::create_dir_all(&cfg_dir).unwrap();
        std::fs::write(
            cfg_dir.join("config.toml"),
            r#"
[parse]
default = "pandoc"
"#,
        )
        .unwrap();
        let cfg = ParseConfig::load_from_vault(tmp.path()).unwrap();
        assert_eq!(cfg.default.as_deref(), Some("pandoc"));
    }

    #[test]
    fn extract_parser_from_frontmatter_returns_none_when_absent() {
        let frontmatter = fm(json!({ "title": "Hello" }));
        assert_eq!(extract_parser_from_frontmatter(&frontmatter), None);
    }

    #[test]
    fn compile_preserves_default_without_rules() {
        let cfg = ParseConfig::from_toml_str(
            r#"
[parse]
default = "pandoc"
"#,
        )
        .unwrap()
        .compile()
        .unwrap();
        assert_eq!(cfg.default_parser(), Some("pandoc"));
        assert_eq!(cfg.match_rule(Path::new("anything.md")), None);
    }
}
