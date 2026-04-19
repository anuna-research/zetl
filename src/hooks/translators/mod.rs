//! Typed AST-protocol dispatch surface (SPEC-032 REQ-3221 / CON-3221).
//!
//! This module owns the translator layer that converts a zetl-ext
//! canonical AST ([`crate::hooks::ast::Document`]) to and from the
//! foreign ecosystems a hook might declare via `ast_type` in its
//! manifest:
//!
//! - **`zetl-ext`** (default) — zetl's native AST. Identity translator.
//! - **`mdast-ext`** — mdast (remark) shape with marker conventions for
//!   zetl-native Wikilink / Embed / SPL nodes (SPEC-033 REQ-3308).
//! - **`pandoc-ext`** — pandoc-types shape with marker conventions for
//!   the same zetl concepts (SPEC-033 REQ-3307).
//!
//! ## Dispatch surface
//!
//! Callers (the transform-stage pipeline wire in `task-eco-registry`) use
//! [`TranslatorRegistry`] to look up a translator for the hook's declared
//! `ast_type`, then call [`dispatch_transform`]:
//!
//! ```text
//!   zetl-ext AST --zetl_to_foreign--> foreign JSON
//!                                     --(persistent protocol: run)-->
//!                                     foreign JSON'
//!                --foreign_to_zetl--> zetl-ext AST'
//!                --marker-strip scan--> [warnings]
//! ```
//!
//! The dispatch function is stage-agnostic at its signature — the caller
//! supplies the "run" closure that actually ships bytes to the hook
//! (today: [`crate::hooks::persistent::PersistentHook::run`]).
//!
//! ## Not-compiled ecosystems (SPEC-033 REQ-3313)
//!
//! A v1 binary registering only `zetl-ext` rejects `pandoc-ext` and
//! `mdast-ext` manifests at load time via
//! [`TranslatorRegistry::require`], which returns a typed
//! [`DispatchError::EcosystemNotCompiled`] carrying an actionable hint
//! (pointing at the `zetl ecosystem check` subcommand planned for
//! REQ-3310).
//!
//! ## Mixed pipelines (REQ-3221)
//!
//! Two hooks at the same stage may declare different `ast_type`s.
//! Between hooks the pipeline always holds a canonical zetl-ext
//! document; each translator invocation is a round trip (foreign → zetl
//! → foreign) so no hook ever sees another hook's AST in a format it
//! didn't request.
//!
//! ## Marker-strip detection (REQ-3221 / REQ-3224)
//!
//! After each foreign-ext hook invocation, zetl counts instances of
//! each node type listed in the hook's `contract.preserves` declaration
//! in the input and the output; a net decrease produces a
//! [`StripWarning`]. The pipeline continues — marker-strip is a warning
//! loop, not a failure. The baseline default for pandoc-ext and
//! mdast-ext adapters is `["Wikilink", "Embed", "SplBlock"]`, injected
//! by [`AstType::default_preserves`] when the manifest doesn't declare
//! its own.

pub mod mdast;
pub mod pandoc;
pub mod zetl_ext;

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::hooks::ast::{Block, Document, Inline};

/// The concrete AST ecosystems a hook manifest may declare.
///
/// `zetl-ext` is the canonical internal form and the manifest default
/// (REQ-3221). The other variants require their corresponding translator
/// to be registered — a v1 binary registering only `zetl-ext` rejects
/// manifests declaring `pandoc-ext` / `mdast-ext` at load time with a
/// typed [`DispatchError::EcosystemNotCompiled`] (REQ-3313).
///
/// Serialisation uses the wire spelling from the manifest grammar; the
/// short aliases `"mdast"` / `"pandoc-types"` are accepted on parse for
/// ergonomic toml authoring but emitted in canonical form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AstType {
    #[serde(rename = "zetl-ext")]
    ZetlExt,
    #[serde(rename = "mdast-ext", alias = "mdast")]
    MdastExt,
    #[serde(rename = "pandoc-ext", alias = "pandoc-types")]
    PandocExt,
}

impl AstType {
    /// The canonical wire spelling emitted in diagnostics and the
    /// `zetl ecosystem check` output.
    pub const fn as_str(self) -> &'static str {
        match self {
            AstType::ZetlExt => "zetl-ext",
            AstType::MdastExt => "mdast-ext",
            AstType::PandocExt => "pandoc-ext",
        }
    }

    /// Default `contract.preserves` for this ecosystem when the manifest
    /// does not declare its own (REQ-3221 / REQ-3224).
    ///
    /// `zetl-ext` requires the hook author to opt in — a hook that
    /// doesn't touch wikilinks shouldn't pay marker-strip bookkeeping.
    /// Foreign adapters inherit the baseline `Wikilink / Embed /
    /// SplBlock` list because those nodes ride into the foreign AST as
    /// markers and are the most plausible casualty of a naive filter.
    pub fn default_preserves(self) -> &'static [&'static str] {
        match self {
            AstType::ZetlExt => &[],
            AstType::MdastExt | AstType::PandocExt => &["Wikilink", "Embed", "SplBlock"],
        }
    }
}

impl Default for AstType {
    fn default() -> Self {
        AstType::ZetlExt
    }
}

impl std::fmt::Display for AstType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for AstType {
    type Err = UnknownAstType;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "zetl-ext" => Ok(AstType::ZetlExt),
            "mdast-ext" | "mdast" => Ok(AstType::MdastExt),
            "pandoc-ext" | "pandoc-types" => Ok(AstType::PandocExt),
            other => Err(UnknownAstType(other.to_string())),
        }
    }
}

/// Returned by [`AstType::from_str`] when the manifest's `ast_type`
/// value isn't one of the three known ecosystem names.
///
/// This is distinct from [`DispatchError::EcosystemNotCompiled`] —
/// `UnknownAstType` means the value isn't even known to zetl as a
/// concept (a typo, a future ecosystem, an invented name), while
/// `EcosystemNotCompiled` means zetl recognises the name but this build
/// didn't link the translator in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownAstType(pub String);

impl std::fmt::Display for UnknownAstType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unknown ast_type '{}' (expected one of: zetl-ext, mdast-ext, pandoc-ext)",
            self.0
        )
    }
}

impl std::error::Error for UnknownAstType {}

/// Bidirectional translator between zetl-ext canonical AST and a foreign
/// AST ecosystem. Implementors are pure (REQ-3221 / CON-3221) — no I/O,
/// no global state, so they can be exercised as plain Rust functions in
/// unit tests.
pub trait Translator: Send + Sync {
    /// The `ast_type` this translator implements.
    fn ast_type(&self) -> AstType;

    /// Serialise a zetl-ext document into its foreign wire shape.
    fn zetl_to_foreign(&self, doc: &Document) -> Result<serde_json::Value, TranslationError>;

    /// Deserialise a foreign wire shape back into the zetl-ext canonical
    /// document. Lossy translations (foreign AST had attrs zetl doesn't
    /// track) are permitted per CON-3221 — the round-trip guarantee is
    /// semantic (renders identically), not byte-identical.
    fn foreign_to_zetl(&self, foreign: serde_json::Value) -> Result<Document, TranslationError>;
}

/// A translation-layer error — either serialisation failed or the
/// foreign shape couldn't be coaxed back into a zetl-ext document.
///
/// Kept as a single error type (rather than a nested enum per
/// ecosystem) because callers in the pipeline treat every translation
/// failure identically: REQ-3207 failure-scoped abort of this hook's
/// transform, page fragment reverted to the pre-hook AST.
#[derive(Debug, Clone)]
pub struct TranslationError {
    pub ast_type: AstType,
    pub direction: TranslationDirection,
    pub message: String,
}

impl TranslationError {
    pub fn to_foreign(ast_type: AstType, message: impl Into<String>) -> Self {
        Self {
            ast_type,
            direction: TranslationDirection::ZetlToForeign,
            message: message.into(),
        }
    }

    pub fn from_foreign(ast_type: AstType, message: impl Into<String>) -> Self {
        Self {
            ast_type,
            direction: TranslationDirection::ForeignToZetl,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for TranslationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} translation ({}) failed: {}",
            self.ast_type, self.direction, self.message
        )
    }
}

impl std::error::Error for TranslationError {}

/// Direction of a translation step, used in [`TranslationError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranslationDirection {
    ZetlToForeign,
    ForeignToZetl,
}

impl std::fmt::Display for TranslationDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TranslationDirection::ZetlToForeign => f.write_str("zetl→foreign"),
            TranslationDirection::ForeignToZetl => f.write_str("foreign→zetl"),
        }
    }
}

/// Registry of concrete translators this binary has been compiled with.
///
/// Binaries decide at construction time which ecosystems to expose;
/// [`TranslatorRegistry::zetl_only`] matches the v1 default (ships
/// `zetl-ext` identity translator only) and rejects `pandoc-ext` /
/// `mdast-ext` manifests with the REQ-3313 error. [`Self::all_v1`]
/// registers every v1-known ecosystem and is what the zetl binary
/// actually constructs at startup.
pub struct TranslatorRegistry {
    translators: HashMap<AstType, Box<dyn Translator>>,
}

impl std::fmt::Debug for TranslatorRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TranslatorRegistry")
            .field("registered", &self.registered_types())
            .finish()
    }
}

impl TranslatorRegistry {
    /// Registry containing only the `zetl-ext` identity translator.
    /// Matches the v1 binary default (SPEC-033 REQ-3313) when no
    /// ecosystem feature flags are enabled at compile time.
    pub fn zetl_only() -> Self {
        let mut r = Self {
            translators: HashMap::new(),
        };
        r.register(Box::new(zetl_ext::ZetlExtTranslator));
        r
    }

    /// Registry containing every v1-known translator: `zetl-ext`,
    /// `mdast-ext`, `pandoc-ext`. The translators are minimal-fidelity
    /// and focus on round-tripping the common CommonMark nodes plus
    /// zetl marker nodes (Wikilink / Embed / SplBlock); deeper
    /// ecosystem-specific fidelity belongs to the per-ecosystem tasks
    /// (task-eco-pandoc / task-eco-mdbook / task-eco-remark).
    pub fn all_v1() -> Self {
        let mut r = Self::zetl_only();
        r.register(Box::new(mdast::MdastTranslator));
        r.register(Box::new(pandoc::PandocTranslator));
        r
    }

    /// Register a translator, replacing any prior entry for the same
    /// [`AstType`]. Tests use this to swap in fake translators; the
    /// binary calls it once per ecosystem at startup.
    pub fn register(&mut self, t: Box<dyn Translator>) {
        self.translators.insert(t.ast_type(), t);
    }

    /// Look up the translator for an [`AstType`]. Returns `None` when
    /// the ecosystem isn't compiled into this binary.
    pub fn get(&self, ast_type: AstType) -> Option<&dyn Translator> {
        self.translators.get(&ast_type).map(|b| b.as_ref())
    }

    /// Whether an ecosystem is registered in this binary.
    pub fn is_registered(&self, ast_type: AstType) -> bool {
        self.translators.contains_key(&ast_type)
    }

    /// All registered ast_types in canonical order for `zetl ecosystem
    /// check` / `zetl hook capabilities` reports.
    pub fn registered_types(&self) -> Vec<AstType> {
        let mut out: Vec<_> = self.translators.keys().copied().collect();
        out.sort_by_key(|t| t.as_str());
        out
    }

    /// Load-time capability check (SPEC-033 REQ-3313).
    ///
    /// Call on every hook manifest at pipeline-init to reject hooks
    /// whose declared `ast_type` isn't compiled into this binary.
    /// Returns the translator on success; on failure returns a typed
    /// [`DispatchError::EcosystemNotCompiled`] with the same error text
    /// `zetl ecosystem check` prints so users can triage both paths
    /// with one message.
    pub fn require(&self, ast_type: AstType, hook_id: &str) -> Result<&dyn Translator, DispatchError> {
        match self.get(ast_type) {
            Some(t) => Ok(t),
            None => Err(DispatchError::EcosystemNotCompiled {
                ast_type,
                hook_id: hook_id.to_string(),
                hint: ecosystem_not_compiled_hint(ast_type, self.registered_types()),
            }),
        }
    }
}

impl Default for TranslatorRegistry {
    fn default() -> Self {
        Self::all_v1()
    }
}

fn ecosystem_not_compiled_hint(ast_type: AstType, registered: Vec<AstType>) -> String {
    let registered_str = registered
        .iter()
        .map(|t| t.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "ast_type '{ast_type}' is not compiled into this zetl binary \
         (registered: [{registered_str}]). \
         Run `zetl ecosystem check` for the full capability report, \
         or rebuild zetl with the ecosystem feature enabled.",
    )
}

/// Errors raised during dispatch (a translate → run → translate-back
/// sequence). Each variant corresponds to a row in CON-3221's failure
/// table.
#[derive(Debug, Clone)]
pub enum DispatchError {
    /// Manifest declared an `ast_type` that this zetl binary wasn't
    /// compiled with a translator for (SPEC-033 REQ-3313). The hook is
    /// disabled at pipeline-init with an actionable hint.
    EcosystemNotCompiled {
        ast_type: AstType,
        hook_id: String,
        hint: String,
    },
    /// Translation either way failed (malformed foreign JSON, shape
    /// unrepresentable in zetl-ext, …). CON-3221 failure table row
    /// "Hook returns foreign AST that fails to translate back".
    Translation(TranslationError),
    /// The caller's run closure returned an error — e.g. the persistent
    /// protocol reported a typed `{"type":"error"}` or a timeout. This
    /// variant is opaque (carries a string) because dispatch doesn't
    /// own the run-path's error taxonomy; the caller reconstructs the
    /// full REQ-3207 diagnostic from its own state.
    HookError { reason: String, detail: String },
}

impl DispatchError {
    pub fn hook_error(reason: impl Into<String>, detail: impl Into<String>) -> Self {
        DispatchError::HookError {
            reason: reason.into(),
            detail: detail.into(),
        }
    }
}

impl From<TranslationError> for DispatchError {
    fn from(e: TranslationError) -> Self {
        DispatchError::Translation(e)
    }
}

impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DispatchError::EcosystemNotCompiled { ast_type, hook_id, hint } => {
                write!(
                    f,
                    "hook '{hook_id}' declared ast_type='{ast_type}': {hint}"
                )
            }
            DispatchError::Translation(e) => e.fmt(f),
            DispatchError::HookError { reason, detail } => {
                if detail.is_empty() {
                    write!(f, "hook error: {reason}")
                } else {
                    write!(f, "hook error: {reason} ({detail})")
                }
            }
        }
    }
}

impl std::error::Error for DispatchError {}

/// A single marker-strip warning — one entry per node type whose count
/// decreased across a foreign-ext hook invocation (REQ-3221).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StripWarning {
    pub hook_id: String,
    pub node_type: String,
    pub before: usize,
    pub after: usize,
}

impl StripWarning {
    /// Canonical log line shape (CON-3221 diagnostic):
    /// `"<hook> dropped <N> <NodeType>"`.
    pub fn log_line(&self) -> String {
        let dropped = self.before.saturating_sub(self.after);
        format!(
            "{} dropped {} {}",
            self.hook_id, dropped, self.node_type
        )
    }
}

/// Outcome of [`dispatch_transform`]: the post-hook zetl-ext document
/// plus any marker-strip warnings surfaced by the preserves scan.
#[derive(Debug)]
pub struct DispatchOutcome {
    pub output: Document,
    pub warnings: Vec<StripWarning>,
}

/// Dispatch a single transform-stage hook invocation.
///
/// Orchestrates the full translate → run → translate-back → scan loop
/// described at the module level:
///
/// 1. Translator is looked up from `registry` (REQ-3313 error if the
///    ecosystem isn't compiled).
/// 2. `input` is serialised via [`Translator::zetl_to_foreign`].
/// 3. The caller-supplied `run` closure sends the JSON to the hook and
///    returns the hook's response — this closure lives in the caller so
///    dispatch stays decoupled from the transport (persistent protocol,
///    in-process fake, …).
/// 4. The response JSON is deserialised via
///    [`Translator::foreign_to_zetl`] back to zetl-ext.
/// 5. The `preserves` list is scanned: for each name, if the post-hook
///    count is strictly less than the pre-hook count a [`StripWarning`]
///    is recorded.
///
/// The pipeline does NOT short-circuit on strip warnings — per
/// CON-3221 they are advisory only.
pub fn dispatch_transform<F>(
    registry: &TranslatorRegistry,
    ast_type: AstType,
    hook_id: &str,
    preserves: &[String],
    input: Document,
    run: F,
) -> Result<DispatchOutcome, DispatchError>
where
    F: FnOnce(serde_json::Value) -> Result<serde_json::Value, DispatchError>,
{
    let translator = registry.require(ast_type, hook_id)?;

    // Pre-count preserves-listed node types on the original document so
    // the post-hook comparison is against what went in, not what we
    // round-tripped. (A lossy translation would otherwise give a false
    // marker-strip warning on the first page.)
    let before_counts = count_node_types(&input, preserves);

    let foreign = translator.zetl_to_foreign(&input)?;
    let returned = run(foreign)?;
    let output = translator.foreign_to_zetl(returned)?;

    let after_counts = count_node_types(&output, preserves);

    let mut warnings = Vec::new();
    for name in preserves {
        let before = before_counts.get(name).copied().unwrap_or(0);
        let after = after_counts.get(name).copied().unwrap_or(0);
        if after < before {
            warnings.push(StripWarning {
                hook_id: hook_id.to_string(),
                node_type: name.clone(),
                before,
                after,
            });
        }
    }

    Ok(DispatchOutcome { output, warnings })
}

/// Count occurrences of each node-type name in `doc`. Walks the full
/// tree (blocks + inlines + list items). Node-type names follow the
/// zetl-ext schema's `"type"` tag — e.g. `"Wikilink"`, `"Embed"`,
/// `"SplBlock"`, `"Paragraph"`.
///
/// Names not present in the tree are absent from the map (not mapped
/// to 0) so callers can distinguish "unseen" from "count==0"; the
/// marker-strip scanner treats absent as 0.
pub fn count_node_types(doc: &Document, names: &[String]) -> BTreeMap<String, usize> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let interest: std::collections::HashSet<&str> =
        names.iter().map(|s| s.as_str()).collect();
    for block in &doc.children {
        walk_block(block, &interest, &mut counts);
    }
    counts
}

fn walk_block(
    block: &Block,
    interest: &std::collections::HashSet<&str>,
    counts: &mut BTreeMap<String, usize>,
) {
    let tag = block.kind_str();
    if interest.contains(tag) {
        *counts.entry(tag.to_string()).or_insert(0) += 1;
    }
    match block {
        Block::Heading(n) => {
            for c in &n.children {
                walk_inline(c, interest, counts);
            }
        }
        Block::Paragraph(n) => {
            for c in &n.children {
                walk_inline(c, interest, counts);
            }
        }
        Block::BlockQuote(n) => {
            for c in &n.children {
                walk_block(c, interest, counts);
            }
        }
        Block::List(n) => {
            for item in &n.children {
                for c in &item.children {
                    walk_block(c, interest, counts);
                }
            }
        }
        Block::CodeBlock(_)
        | Block::ThematicBreak(_)
        | Block::HtmlBlock(_)
        | Block::SplBlock(_)
        | Block::Embed(_) => {}
    }
}

fn walk_inline(
    inline: &Inline,
    interest: &std::collections::HashSet<&str>,
    counts: &mut BTreeMap<String, usize>,
) {
    let tag = inline.kind_str();
    if interest.contains(tag) {
        *counts.entry(tag.to_string()).or_insert(0) += 1;
    }
    match inline {
        Inline::Emphasis(n) => {
            for c in &n.children {
                walk_inline(c, interest, counts);
            }
        }
        Inline::Strong(n) => {
            for c in &n.children {
                walk_inline(c, interest, counts);
            }
        }
        Inline::Link(n) => {
            for c in &n.children {
                walk_inline(c, interest, counts);
            }
        }
        Inline::Image(n) => {
            for c in &n.children {
                walk_inline(c, interest, counts);
            }
        }
        Inline::Text(_)
        | Inline::Code(_)
        | Inline::LineBreak(_)
        | Inline::SoftBreak(_)
        | Inline::HtmlInline(_)
        | Inline::Wikilink(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::ast::{
        Document, DocumentKind, Paragraph, Position, Text, Wikilink, AST_VERSION,
    };

    fn pos() -> Position {
        Position::origin()
    }

    fn doc_with(children: Vec<Block>) -> Document {
        Document {
            ast_version: AST_VERSION.to_string(),
            kind: DocumentKind::Document,
            position: pos(),
            frontmatter: None,
            children,
        }
    }

    fn para(children: Vec<Inline>) -> Block {
        Block::Paragraph(Paragraph {
            position: pos(),
            children,
        })
    }

    fn text(s: &str) -> Inline {
        Inline::Text(Text {
            position: pos(),
            text: s.to_string(),
        })
    }

    fn wikilink(target: &str) -> Inline {
        Inline::Wikilink(Wikilink {
            position: pos(),
            target: target.to_string(),
            alias: None,
            heading: None,
            block_id: None,
        })
    }

    // ── AstType parsing ────────────────────────────────────────────────

    #[test]
    fn ast_type_parse_accepts_canonical_names() {
        assert_eq!("zetl-ext".parse::<AstType>().unwrap(), AstType::ZetlExt);
        assert_eq!("mdast-ext".parse::<AstType>().unwrap(), AstType::MdastExt);
        assert_eq!("pandoc-ext".parse::<AstType>().unwrap(), AstType::PandocExt);
    }

    #[test]
    fn ast_type_parse_accepts_short_aliases() {
        assert_eq!("mdast".parse::<AstType>().unwrap(), AstType::MdastExt);
        assert_eq!("pandoc-types".parse::<AstType>().unwrap(), AstType::PandocExt);
    }

    #[test]
    fn ast_type_parse_rejects_unknown() {
        let err = "djot".parse::<AstType>().unwrap_err();
        assert_eq!(err.0, "djot");
        assert!(err.to_string().contains("unknown ast_type"));
    }

    #[test]
    fn ast_type_default_is_zetl_ext() {
        assert_eq!(AstType::default(), AstType::ZetlExt);
    }

    #[test]
    fn ast_type_roundtrips_through_serde() {
        // Manifests consume ast_type via serde with the canonical name.
        for t in [AstType::ZetlExt, AstType::MdastExt, AstType::PandocExt] {
            let v = serde_json::to_value(t).unwrap();
            assert_eq!(v, serde_json::Value::String(t.as_str().to_string()));
            let back: AstType = serde_json::from_value(v).unwrap();
            assert_eq!(back, t);
        }
    }

    #[test]
    fn ast_type_serde_accepts_short_aliases() {
        let mdast: AstType = serde_json::from_value(serde_json::json!("mdast")).unwrap();
        assert_eq!(mdast, AstType::MdastExt);
        let pandoc: AstType = serde_json::from_value(serde_json::json!("pandoc-types")).unwrap();
        assert_eq!(pandoc, AstType::PandocExt);
    }

    #[test]
    fn default_preserves_matches_spec_baseline() {
        assert!(AstType::ZetlExt.default_preserves().is_empty());
        let baseline = &["Wikilink", "Embed", "SplBlock"];
        assert_eq!(AstType::MdastExt.default_preserves(), baseline);
        assert_eq!(AstType::PandocExt.default_preserves(), baseline);
    }

    // ── Registry ───────────────────────────────────────────────────────

    #[test]
    fn zetl_only_registry_rejects_non_default_ecosystems() {
        let reg = TranslatorRegistry::zetl_only();
        assert!(reg.is_registered(AstType::ZetlExt));
        assert!(!reg.is_registered(AstType::MdastExt));
        assert!(!reg.is_registered(AstType::PandocExt));
        let err = match reg.require(AstType::MdastExt, "mdast-hook") {
            Ok(_) => panic!("expected EcosystemNotCompiled"),
            Err(e) => e,
        };
        match err {
            DispatchError::EcosystemNotCompiled { ast_type, hook_id, hint } => {
                assert_eq!(ast_type, AstType::MdastExt);
                assert_eq!(hook_id, "mdast-hook");
                // REQ-3313 requires an actionable hint.
                assert!(hint.contains("zetl ecosystem check"));
                assert!(hint.contains("zetl-ext"));
            }
            other => panic!("expected EcosystemNotCompiled, got {other:?}"),
        }
    }

    #[test]
    fn all_v1_registry_has_every_known_ecosystem() {
        let reg = TranslatorRegistry::all_v1();
        assert!(reg.is_registered(AstType::ZetlExt));
        assert!(reg.is_registered(AstType::MdastExt));
        assert!(reg.is_registered(AstType::PandocExt));
        assert_eq!(
            reg.registered_types(),
            vec![AstType::MdastExt, AstType::PandocExt, AstType::ZetlExt]
        );
    }

    // ── Dispatch: identity pass-through ────────────────────────────────

    #[test]
    fn zetl_ext_dispatch_identity_round_trips_a_document() {
        let reg = TranslatorRegistry::all_v1();
        let doc = doc_with(vec![para(vec![text("hello "), wikilink("Other")])]);

        // The hook closure returns its input unchanged; the zetl-ext
        // identity translator means the document we get back is byte-
        // equivalent to the input.
        let outcome = dispatch_transform(
            &reg,
            AstType::ZetlExt,
            "identity",
            &[],
            doc.clone(),
            |v| Ok(v),
        )
        .unwrap();

        assert_eq!(outcome.output, doc);
        assert!(outcome.warnings.is_empty());
    }

    // ── Dispatch: marker-strip detection (REQ-3221) ────────────────────

    #[test]
    fn strip_warning_surfaces_when_foreign_hook_drops_wikilinks() {
        let reg = TranslatorRegistry::all_v1();
        let input = doc_with(vec![para(vec![
            text("see "),
            wikilink("A"),
            text(", "),
            wikilink("B"),
        ])]);

        // A malicious/buggy hook that forgets wikilinks on round-trip.
        let run = |_foreign: serde_json::Value| -> Result<serde_json::Value, DispatchError> {
            // Return a document with only the text, dropping wikilinks.
            // Use mdast shape — this test exercises mdast dispatch.
            Ok(serde_json::json!({
                "type": "root",
                "position": {"start_line":1,"start_col":1,"end_line":1,"end_col":1},
                "zetl_ast_version": "1.0",
                "children": [
                    {
                        "type": "paragraph",
                        "position": {"start_line":1,"start_col":1,"end_line":1,"end_col":1},
                        "children": [
                            {"type":"text","value":"see , ","position":{"start_line":1,"start_col":1,"end_line":1,"end_col":1}}
                        ]
                    }
                ]
            }))
        };

        let preserves = vec!["Wikilink".to_string(), "Embed".to_string()];
        let outcome = dispatch_transform(
            &reg,
            AstType::MdastExt,
            "lossy-hook",
            &preserves,
            input,
            run,
        )
        .unwrap();

        assert_eq!(outcome.warnings.len(), 1);
        let w = &outcome.warnings[0];
        assert_eq!(w.hook_id, "lossy-hook");
        assert_eq!(w.node_type, "Wikilink");
        assert_eq!(w.before, 2);
        assert_eq!(w.after, 0);
        assert_eq!(w.log_line(), "lossy-hook dropped 2 Wikilink");
    }

    #[test]
    fn identity_passthrough_emits_no_strip_warnings_with_preserves() {
        // Even when the preserves list names Wikilink, an identity
        // hook should produce zero warnings — the count is unchanged.
        let reg = TranslatorRegistry::all_v1();
        let input = doc_with(vec![para(vec![text("hi"), wikilink("Target")])]);
        let preserves = vec!["Wikilink".to_string()];

        let outcome = dispatch_transform(
            &reg,
            AstType::ZetlExt,
            "identity",
            &preserves,
            input,
            |v| Ok(v),
        )
        .unwrap();

        assert!(outcome.warnings.is_empty());
    }

    // ── Dispatch: ecosystem-not-compiled error ─────────────────────────

    #[test]
    fn dispatch_errors_when_ecosystem_missing() {
        let reg = TranslatorRegistry::zetl_only();
        let doc = doc_with(vec![]);

        let err = dispatch_transform(
            &reg,
            AstType::PandocExt,
            "needs-pandoc",
            &[],
            doc,
            |v| Ok(v),
        )
        .unwrap_err();

        match err {
            DispatchError::EcosystemNotCompiled { ast_type, hook_id, hint } => {
                assert_eq!(ast_type, AstType::PandocExt);
                assert_eq!(hook_id, "needs-pandoc");
                assert!(hint.contains("pandoc-ext"));
            }
            other => panic!("expected EcosystemNotCompiled, got {other:?}"),
        }
    }

    // ── count_node_types helper ────────────────────────────────────────

    #[test]
    fn count_node_types_walks_nested_children() {
        let doc = doc_with(vec![
            para(vec![wikilink("A"), wikilink("B")]),
            Block::BlockQuote(crate::hooks::ast::BlockQuote {
                position: pos(),
                children: vec![para(vec![wikilink("C")])],
            }),
            Block::Embed(crate::hooks::ast::Embed {
                position: pos(),
                target: "X".into(),
                heading: None,
                block_id: None,
            }),
        ]);
        let names = vec!["Wikilink".to_string(), "Embed".to_string()];
        let counts = count_node_types(&doc, &names);
        assert_eq!(counts.get("Wikilink").copied().unwrap_or(0), 3);
        assert_eq!(counts.get("Embed").copied().unwrap_or(0), 1);
    }
}
