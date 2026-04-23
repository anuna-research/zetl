//! Behavioural-contract enforcement (SPEC-032 REQ-3224 / CON-3224).
//!
//! Runtime validators for the `[contract]` manifest table
//! ([`crate::hooks::manifest::ContractDecl`]) and a fixture-driven
//! property-test harness that asks a hook to honour its declared claims.
//!
//! | Field             | Validator                       | Tier  |
//! | ----------------- | ------------------------------- | ----- |
//! | `preserves`       | [`validate_preserves`]          | 1     |
//! | `may_restructure` | [`validate_may_restructure`]    | 1     |
//! | `idempotent`      | [`validate_idempotence`]        | 1 CI  |
//! | `pure`            | surfaced in reports, not gated  | 2 v1.1 |
//! | `expansion_bound` | [`validate_expansion_bound`]    | 2 v1.1 (advisory) |
//!
//! Each validator is a pure function that returns zero or more
//! [`ContractViolation`]s. A violation converts into the shared
//! [`crate::hooks::diagnostic::HookDiagnostic`] (class
//! [`DiagnosticClass::ContractViolation`]) for terminal rendering, and
//! into a [`FailureRecord`] with `reason = "contract_violation"` for
//! on-disk `hook-diagnostics.json` persistence (CON-3207).
//!
//! ## Canonical-form equivalence
//!
//! [`canonicalise`] strips source-position annotations so two documents
//! that render identically are `==` after canonicalisation. This is the
//! NFR-3305 equivalence relation the `idempotent` validator uses —
//! f(f(x)) and f(x) are canonical-form-equivalent iff their rendered
//! outputs would match.
//!
//! ## Property-test harness
//!
//! [`PropertyTestCase`] + [`run_property_test`] let a test (or a user
//! invoking `ztl hook test --property`) take a hook, an input fixture,
//! and a [`ContractDecl`], then validate every declared claim in one
//! pass. Each run produces a [`PropertyTestReport`] summarising
//! violations — empty on success, populated when the hook's behaviour
//! diverged from its declarations.

use std::time::Duration;

use crate::hooks::ast::{Block, Document, Inline, ListItem, Position};
use crate::hooks::diagnostic::{DiagnosticClass, HookDiagnostic};
use crate::hooks::failure_scoping::{now_iso8601, FailureReason, FailureRecord};
use crate::hooks::manifest::ContractDecl;
use crate::hooks::pipeline::{HookError, Stage};
use crate::hooks::translators::count_node_types;

/// Sub-reason encoded in a `contract_violation` diagnostic (CON-3224).
///
/// The parent REQ-3207 category is always `"contract_violation"`; this
/// enum names *which* invariant failed so tooling and users can filter
/// (`preserves` vs `idempotent` vs `may_restructure`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractSubReason {
    /// `contract.preserves` — hook dropped declared node types.
    Preserves,
    /// `contract.idempotent` — f(f(x)) != f(x).
    Idempotent,
    /// `contract.may_restructure` — pre-parse hook rewrote block tree.
    MayRestructure,
    /// `contract.pure` — v1.1 sandbox detected I/O outside the
    /// whitelist. Reserved: emitted by no validator in v1.
    Pure,
    /// `contract.expansion_bound` — output length exceeded the declared
    /// multiplier. v1 advisory, v1.1 gated.
    ExpansionBound,
}

impl ContractSubReason {
    /// Stable machine-readable label. Embedded in the
    /// `reason = "contract_violation:<sub>"` detail and matched against
    /// by CI log filters.
    pub const fn as_str(self) -> &'static str {
        match self {
            ContractSubReason::Preserves => "preserves",
            ContractSubReason::Idempotent => "idempotent",
            ContractSubReason::MayRestructure => "may_restructure",
            ContractSubReason::Pure => "pure",
            ContractSubReason::ExpansionBound => "expansion_bound",
        }
    }
}

impl std::fmt::Display for ContractSubReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single contract-invariant violation detected for one hook
/// invocation on one page.
///
/// Renders into both:
/// - [`HookDiagnostic`] (CON-3225 five-part terminal output), and
/// - [`FailureRecord`] (CON-3207 `hook-diagnostics.json` schema,
///   with `reason = "contract_violation"` and the sub_reason embedded
///   in `detail`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractViolation {
    /// Which invariant failed.
    pub sub_reason: ContractSubReason,
    /// Pipeline stage the hook ran at.
    pub stage: Stage,
    /// Hook id (manifest `extension_id` or filename-derived).
    pub hook_id: String,
    /// Page slug under render when the violation was detected. Empty
    /// for CI double-runs against standalone fixtures.
    pub page_slug: String,
    /// One-line machine-readable summary (e.g. `"-4 Wikilink"`).
    pub detail: String,
    /// Observed-data lines surfaced in the CON-3225 diagnostic body.
    pub observed: Vec<String>,
}

impl ContractViolation {
    /// Convert to a [`FailureRecord`] — the on-disk `hook-diagnostics.json`
    /// schema. `duration` is the hook's wall-clock time at violation
    /// detection (0 when detection was offline, e.g. CI double-run).
    ///
    /// The categorised `reason` field is always `"contract_violation"`
    /// (CON-3207); the sub_reason is prefixed into the `detail` field so
    /// tooling can filter on either dimension without a schema bump.
    pub fn to_failure_record(&self, duration: Duration) -> FailureRecord {
        FailureRecord::new(
            self.hook_id.clone(),
            self.stage,
            self.page_slug.clone(),
            FailureReason::ContractViolation,
            format!("{}: {}", self.sub_reason, self.detail),
            duration,
            now_iso8601(),
        )
    }

    /// Render as a [`HookDiagnostic`] (CON-3225 five-part body).
    ///
    /// The summary line names the hook, page, and sub-reason. The
    /// context carries the declared invariant (e.g. `contract.preserves
    /// = [...]`). The observed block holds the concrete counts the
    /// validator collected. The `cause`/`hint` pair is chosen per
    /// sub_reason so identical failure shapes yield identical advice.
    pub fn to_diagnostic(&self, declaration: &str) -> HookDiagnostic {
        let summary = if self.page_slug.is_empty() {
            format!(
                "hook '{}' contract violation ({})",
                self.hook_id, self.sub_reason,
            )
        } else {
            format!(
                "hook '{}' contract violation on {} ({})",
                self.hook_id, self.page_slug, self.sub_reason,
            )
        };
        let mut d = HookDiagnostic::new(DiagnosticClass::ContractViolation, summary)
            .with_context(declaration.to_string());
        for line in &self.observed {
            d = d.with_observed(line.clone());
        }
        let (cause, hint) = advice_for(self.sub_reason);
        if let Some(c) = cause {
            d = d.with_cause(c);
        }
        if let Some(h) = hint {
            d = d.with_hint(h);
        }
        d
    }
}

fn advice_for(sub: ContractSubReason) -> (Option<&'static str>, Option<&'static str>) {
    match sub {
        ContractSubReason::Preserves => (
            Some(
                "the transform strips unrecognised Span classes or\n\
                 drops inline nodes whose type it doesn't match.",
            ),
            Some(
                "run `ztl ast diff <before.json> <after.json>` on the fixture\n\
                 input to locate the removed nodes; see:\n  \
                 https://ztl.codeberg.page/docs/hook-authoring/preservation",
            ),
        ),
        ContractSubReason::Idempotent => (
            Some(
                "the transform runs over its own output, wrapping\n\
                 already-rendered blocks a second time.",
            ),
            Some(
                "detect already-rendered output and short-circuit, OR declare\n\
                 contract.idempotent = false if the repeated output is intended.",
            ),
        ),
        ContractSubReason::MayRestructure => (
            Some(
                "a pre-parse hook rewrote the block tree shape. By default\n\
                 pre-parse hooks must preserve block structure (REQ-3222).",
            ),
            Some(
                "set contract.may_restructure = true if the rewrite is intended,\n\
                 or constrain the hook to inline/textual edits only.",
            ),
        ),
        ContractSubReason::Pure => (
            Some(
                "the hook performed I/O (network / filesystem / clock) outside\n\
                 the stage_input + context whitelist.",
            ),
            Some("drop contract.pure = true if the hook legitimately needs I/O."),
        ),
        ContractSubReason::ExpansionBound => (
            Some("output size exceeded the declared contract.expansion_bound multiplier."),
            Some("raise contract.expansion_bound, or constrain the hook's output."),
        ),
    }
}

// ── Canonicalisation ───────────────────────────────────────────────────────

/// Strip source positions so two documents that render identically
/// compare `==` (NFR-3305 canonical form).
///
/// Used by the idempotence validator to compare `f(x)` and `f(f(x))`
/// without tripping on positions set to wherever the synthetic nodes
/// came from. Everything else (text, children, attrs) is compared
/// verbatim.
pub fn canonicalise(doc: &Document) -> Document {
    let mut out = doc.clone();
    out.position = Position::origin();
    for block in &mut out.children {
        canonicalise_block(block);
    }
    out
}

fn canonicalise_block(b: &mut Block) {
    match b {
        Block::Heading(n) => {
            n.position = Position::origin();
            for c in &mut n.children {
                canonicalise_inline(c);
            }
        }
        Block::Paragraph(n) => {
            n.position = Position::origin();
            for c in &mut n.children {
                canonicalise_inline(c);
            }
        }
        Block::BlockQuote(n) => {
            n.position = Position::origin();
            for c in &mut n.children {
                canonicalise_block(c);
            }
        }
        Block::List(n) => {
            n.position = Position::origin();
            for item in &mut n.children {
                canonicalise_list_item(item);
            }
        }
        Block::CodeBlock(n) => n.position = Position::origin(),
        Block::ThematicBreak(n) => n.position = Position::origin(),
        Block::HtmlBlock(n) => n.position = Position::origin(),
        Block::SplBlock(n) => n.position = Position::origin(),
        Block::Embed(n) => n.position = Position::origin(),
    }
}

fn canonicalise_list_item(item: &mut ListItem) {
    item.position = Position::origin();
    for c in &mut item.children {
        canonicalise_block(c);
    }
}

fn canonicalise_inline(i: &mut Inline) {
    match i {
        Inline::Text(n) => n.position = Position::origin(),
        Inline::Emphasis(n) => {
            n.position = Position::origin();
            for c in &mut n.children {
                canonicalise_inline(c);
            }
        }
        Inline::Strong(n) => {
            n.position = Position::origin();
            for c in &mut n.children {
                canonicalise_inline(c);
            }
        }
        Inline::Code(n) => n.position = Position::origin(),
        Inline::Link(n) => {
            n.position = Position::origin();
            for c in &mut n.children {
                canonicalise_inline(c);
            }
        }
        Inline::Image(n) => {
            n.position = Position::origin();
            for c in &mut n.children {
                canonicalise_inline(c);
            }
        }
        Inline::LineBreak(n) => n.position = Position::origin(),
        Inline::SoftBreak(n) => n.position = Position::origin(),
        Inline::HtmlInline(n) => n.position = Position::origin(),
        Inline::Wikilink(n) => n.position = Position::origin(),
    }
}

// ── preserves validator ────────────────────────────────────────────────────

/// Validate `contract.preserves` after a single hook invocation.
///
/// Counts each declared node-type name in `before` and `after`; any type
/// whose count strictly decreased produces a
/// [`ContractSubReason::Preserves`] violation. Generalises REQ-3221's
/// baseline wikilink / embed / SPL marker-strip scan to an arbitrary
/// per-manifest list.
///
/// A name absent from both documents is silently ignored; a name
/// present only in the input and dropped to zero still produces a
/// violation (that's the whole point). Unknown node-type names don't
/// error here — they just never match — but the manifest parser
/// rejects unknown ast_types at load time, and CI lints the preserves
/// list against the AST schema at gate time.
pub fn validate_preserves(
    stage: Stage,
    hook_id: &str,
    page_slug: &str,
    preserves: &[String],
    before: &Document,
    after: &Document,
) -> Vec<ContractViolation> {
    if preserves.is_empty() {
        return Vec::new();
    }
    let before_counts = count_node_types(before, preserves);
    let after_counts = count_node_types(after, preserves);
    let mut violations = Vec::new();
    for name in preserves {
        let b = before_counts.get(name).copied().unwrap_or(0);
        let a = after_counts.get(name).copied().unwrap_or(0);
        if a < b {
            let dropped = b - a;
            violations.push(ContractViolation {
                sub_reason: ContractSubReason::Preserves,
                stage,
                hook_id: hook_id.to_string(),
                page_slug: page_slug.to_string(),
                detail: format!("-{dropped} {name}"),
                observed: vec![
                    format!("observed in input:  {b} {name}"),
                    format!("observed in output: {a} {name}"),
                    format!("net change: -{dropped} {name}"),
                ],
            });
        }
    }
    violations
}

// ── may_restructure validator ──────────────────────────────────────────────

/// Shape summary of a document's block tree — the REQ-3222 diffing
/// surface.
///
/// The invariant captured is "two documents have the same block-tree
/// shape if (a) their top-level block kinds are identical in order,
/// and (b) their maximum block-nesting depth is the same". A pre-parse
/// hook that rewrites block structure trips at least one of those
/// conditions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockShape {
    /// Kind tag of every top-level block, left-to-right.
    pub top_level_kinds: Vec<&'static str>,
    /// Deepest nesting depth of any block (1 = flat top-level block).
    pub max_depth: usize,
}

impl BlockShape {
    pub fn of(doc: &Document) -> Self {
        let top: Vec<&'static str> = doc.children.iter().map(Block::kind_str).collect();
        let max_depth = doc.children.iter().map(block_depth).max().unwrap_or(0);
        Self {
            top_level_kinds: top,
            max_depth,
        }
    }
}

fn block_depth(b: &Block) -> usize {
    match b {
        Block::Heading(_)
        | Block::Paragraph(_)
        | Block::CodeBlock(_)
        | Block::ThematicBreak(_)
        | Block::HtmlBlock(_)
        | Block::SplBlock(_)
        | Block::Embed(_) => 1,
        Block::BlockQuote(bq) => 1 + bq.children.iter().map(block_depth).max().unwrap_or(0),
        Block::List(l) => {
            let deepest_item = l
                .children
                .iter()
                .flat_map(|item| item.children.iter())
                .map(block_depth)
                .max()
                .unwrap_or(0);
            1 + deepest_item
        }
    }
}

/// Validate `contract.may_restructure = false` on a pre-parse hook.
///
/// Compares the block-tree shape of `before` and `after`. Violations:
///
/// - **Top-level length mismatch** — the hook added or removed top-level
///   blocks.
/// - **Top-level kind mismatch** — the hook replaced a block with a
///   different kind at the same index.
/// - **Depth delta greater than 1** — the hook added or removed more
///   than one layer of nesting. A single layer (e.g. wrapping paragraphs
///   in a blockquote) is allowed under REQ-3222's one-layer tolerance.
///
/// Returns `None` when shapes agree, [`Some`] when the hook
/// restructured the tree. The caller is expected to have parsed
/// `before` as the hook's input and `after` as the hook's output, using
/// the same parser + extensions (REQ-3222 explicitly requires identical
/// parse settings to rule out parser differences).
pub fn validate_may_restructure(
    stage: Stage,
    hook_id: &str,
    page_slug: &str,
    before: &Document,
    after: &Document,
) -> Option<ContractViolation> {
    let a = BlockShape::of(before);
    let b = BlockShape::of(after);

    let mut observed = Vec::new();
    if a.top_level_kinds.len() != b.top_level_kinds.len() {
        observed.push(format!(
            "top-level blocks: {} \u{2192} {}",
            a.top_level_kinds.len(),
            b.top_level_kinds.len()
        ));
    }
    for (i, (before_kind, after_kind)) in a
        .top_level_kinds
        .iter()
        .zip(b.top_level_kinds.iter())
        .enumerate()
    {
        if before_kind != after_kind {
            observed.push(format!("block[{i}]: {before_kind} \u{2192} {after_kind}"));
        }
    }
    let depth_delta = b.max_depth as isize - a.max_depth as isize;
    if depth_delta.abs() > 1 {
        observed.push(format!(
            "nesting depth: {} \u{2192} {} (delta {depth_delta})",
            a.max_depth, b.max_depth
        ));
    }

    if observed.is_empty() {
        None
    } else {
        let detail = format!(
            "block tree restructured ({} change{})",
            observed.len(),
            if observed.len() == 1 { "" } else { "s" },
        );
        Some(ContractViolation {
            sub_reason: ContractSubReason::MayRestructure,
            stage,
            hook_id: hook_id.to_string(),
            page_slug: page_slug.to_string(),
            detail,
            observed,
        })
    }
}

// ── idempotent validator ───────────────────────────────────────────────────

/// Validate `contract.idempotent = true` via CI double-run.
///
/// Runs the supplied hook twice — once on `input`, once on the first
/// run's output — and compares the canonical forms. If
/// `canonicalise(f(f(x))) != canonicalise(f(x))`, reports a
/// [`ContractSubReason::Idempotent`] violation.
///
/// A run-time `HookError` short-circuits to `Err` — a hook that crashes
/// in the second run fails the CI gate but isn't a *contract* violation
/// per se; callers should log the error separately.
pub fn validate_idempotence<F>(
    stage: Stage,
    hook_id: &str,
    page_slug: &str,
    input: Document,
    mut run: F,
) -> Result<Option<ContractViolation>, HookError>
where
    F: FnMut(Document) -> Result<Document, HookError>,
{
    let once = run(input)?;
    let twice = run(once.clone())?;
    let canon_once = canonicalise(&once);
    let canon_twice = canonicalise(&twice);
    if canon_once == canon_twice {
        return Ok(None);
    }
    let observed = vec![
        format!("first run:  {} top-level block(s)", once.children.len()),
        format!("second run: {} top-level block(s)", twice.children.len()),
        "canonicalise(f(f(x))) != canonicalise(f(x))".to_string(),
    ];
    Ok(Some(ContractViolation {
        sub_reason: ContractSubReason::Idempotent,
        stage,
        hook_id: hook_id.to_string(),
        page_slug: page_slug.to_string(),
        detail: "f(f(x)) != f(x)".to_string(),
        observed,
    }))
}

// ── expansion_bound validator ──────────────────────────────────────────────

/// Validate `contract.expansion_bound` — tier-2, advisory in v1.
///
/// Compares `output_size / max(1, input_size)` against the declared
/// bound. A ratio strictly greater than `bound` produces a
/// [`ContractSubReason::ExpansionBound`] violation. Sizes are byte
/// counts — the caller decides whether to measure the serialised AST,
/// the rendered HTML, or both.
///
/// v1 treats this as advisory (log-only); v1.1 will gate on it per
/// CON-3224's enforcement table.
pub fn validate_expansion_bound(
    stage: Stage,
    hook_id: &str,
    page_slug: &str,
    bound: f32,
    input_size: usize,
    output_size: usize,
) -> Option<ContractViolation> {
    let denom = input_size.max(1) as f32;
    let ratio = output_size as f32 / denom;
    if ratio <= bound {
        return None;
    }
    Some(ContractViolation {
        sub_reason: ContractSubReason::ExpansionBound,
        stage,
        hook_id: hook_id.to_string(),
        page_slug: page_slug.to_string(),
        detail: format!("{ratio:.2}x > bound {bound:.2}x"),
        observed: vec![
            format!("input size:  {input_size} bytes"),
            format!("output size: {output_size} bytes"),
            format!("ratio: {ratio:.2}x (bound {bound:.2}x)"),
        ],
    })
}

// ── Property-test harness ──────────────────────────────────────────────────

/// A fixture-driven test case: a hook's contract declaration plus one
/// input document the harness will feed through the hook.
///
/// Separate from `tests/hook-fixtures/<name>/input.md` (the on-disk
/// `hook test` fixtures) because property tests run against
/// programmatically-built ASTs too — e.g. random-generated documents
/// in a proptest sweep, or the canonical fixtures the matrix CI
/// consumes.
#[derive(Debug, Clone)]
pub struct PropertyTestCase {
    /// Human name for the case — appears in [`PropertyTestReport`].
    pub name: String,
    /// The hook's declared contract. Tier-1 fields drive validator
    /// dispatch; tier-2 fields propagate to the report but don't gate
    /// v1 output.
    pub contract: ContractDecl,
    /// Stage at which the hook runs. Selects which validators apply:
    /// `may_restructure` is only checked for pre-parse hooks per
    /// REQ-3222.
    pub stage: Stage,
    /// Input document fed to the hook. For a pre-parse harness, callers
    /// should parse the raw markdown themselves and pass the parsed
    /// document on both sides of the `may_restructure` check.
    pub input: Document,
    /// Byte size of the *raw* stage input (markdown for pre-parse,
    /// serialised AST for transform, HTML for post-render). Used only by
    /// the `expansion_bound` validator.
    pub input_size: usize,
}

/// Summary returned by [`run_property_test`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PropertyTestReport {
    /// Case name — copied from [`PropertyTestCase::name`].
    pub case_name: String,
    /// Every violation collected across all validators that applied.
    /// Empty on success.
    pub violations: Vec<ContractViolation>,
}

impl PropertyTestReport {
    pub fn passed(&self) -> bool {
        self.violations.is_empty()
    }
}

/// Run every applicable validator for one case against a hook.
///
/// The caller supplies the hook as a closure: `Fn(Document) → Result
/// <Document, HookError>`. The harness runs it *at least once* (for the
/// `preserves` check), and a *second time* on the first run's output
/// if `contract.idempotent` is set. A third measurement of the
/// serialised first-run output drives the `expansion_bound` check.
///
/// The `pure` tier-2 field is never validated here — it's an advisory
/// label in v1, surfaced in report metadata but not enforced.
///
/// A `HookError` from either hook invocation short-circuits the
/// harness. The partial report up to that point is discarded, since a
/// crashed hook can't be scored against its declarations in a
/// meaningful way.
pub fn run_property_test<F>(
    case: &PropertyTestCase,
    hook_id: &str,
    page_slug: &str,
    mut run: F,
) -> Result<PropertyTestReport, HookError>
where
    F: FnMut(Document) -> Result<Document, HookError>,
{
    let mut report = PropertyTestReport {
        case_name: case.name.clone(),
        violations: Vec::new(),
    };

    let first = run(case.input.clone())?;

    // preserves — any stage, runs only when the list is non-empty.
    report.violations.extend(validate_preserves(
        case.stage,
        hook_id,
        page_slug,
        &case.contract.preserves,
        &case.input,
        &first,
    ));

    // may_restructure — pre-parse only (REQ-3222). Check when
    // `may_restructure` is false (the default); `true` opts out.
    if case.stage == Stage::PreParse && !case.contract.may_restructure {
        if let Some(v) =
            validate_may_restructure(case.stage, hook_id, page_slug, &case.input, &first)
        {
            report.violations.push(v);
        }
    }

    // idempotent — CI double-run. The closure is called a second time
    // on the first run's output.
    if case.contract.idempotent {
        let second = run(first.clone())?;
        let canon_first = canonicalise(&first);
        let canon_second = canonicalise(&second);
        if canon_first != canon_second {
            report.violations.push(ContractViolation {
                sub_reason: ContractSubReason::Idempotent,
                stage: case.stage,
                hook_id: hook_id.to_string(),
                page_slug: page_slug.to_string(),
                detail: "f(f(x)) != f(x)".to_string(),
                observed: vec![
                    format!("first run:  {} top-level block(s)", first.children.len()),
                    format!("second run: {} top-level block(s)", second.children.len()),
                    "canonicalise(f(f(x))) != canonicalise(f(x))".to_string(),
                ],
            });
        }
    }

    // expansion_bound — advisory in v1. Measured against the first
    // run's serialised AST size, which stands in for "output size" for
    // both pre-parse and transform stages.
    if let Some(bound) = case.contract.expansion_bound {
        let out_size = serde_json::to_vec(&first).map(|v| v.len()).unwrap_or(0);
        if let Some(v) = validate_expansion_bound(
            case.stage,
            hook_id,
            page_slug,
            bound,
            case.input_size,
            out_size,
        ) {
            report.violations.push(v);
        }
    }

    Ok(report)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::ast::{
        BlockQuote, Document, DocumentKind, Embed, Heading, List, ListItem, Paragraph, Position,
        Text, Wikilink, AST_VERSION,
    };

    fn pos() -> Position {
        Position::origin()
    }

    fn doc(children: Vec<Block>) -> Document {
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

    fn embed(target: &str) -> Block {
        Block::Embed(Embed {
            position: pos(),
            target: target.to_string(),
            heading: None,
            block_id: None,
        })
    }

    // ── ContractSubReason ──────────────────────────────────────────────

    #[test]
    fn sub_reason_labels_are_stable_strings() {
        assert_eq!(ContractSubReason::Preserves.as_str(), "preserves");
        assert_eq!(ContractSubReason::Idempotent.as_str(), "idempotent");
        assert_eq!(
            ContractSubReason::MayRestructure.as_str(),
            "may_restructure"
        );
        assert_eq!(ContractSubReason::Pure.as_str(), "pure");
        assert_eq!(
            ContractSubReason::ExpansionBound.as_str(),
            "expansion_bound"
        );
        assert_eq!(format!("{}", ContractSubReason::Preserves), "preserves");
    }

    // ── validate_preserves ─────────────────────────────────────────────
    // TEST-3224 preservation-diagnostic matrix (REQ-3224).

    #[test]
    fn preserves_empty_list_never_violates() {
        // "empty (no enforcement)" row of TEST-3224's matrix.
        let input = doc(vec![para(vec![wikilink("A")])]);
        let output = doc(vec![]); // hook stripped everything
        let v = validate_preserves(Stage::Transform, "hook", "page", &[], &input, &output);
        assert!(v.is_empty());
    }

    #[test]
    fn preserves_single_type_flags_drop() {
        // "single type (Wikilink)" row.
        let input = doc(vec![para(vec![wikilink("A"), text(" "), wikilink("B")])]);
        let output = doc(vec![para(vec![text("x")])]); // wikilinks dropped
        let v = validate_preserves(
            Stage::Transform,
            "lossy",
            "notes/one",
            &["Wikilink".into()],
            &input,
            &output,
        );
        assert_eq!(v.len(), 1);
        let viol = &v[0];
        assert_eq!(viol.sub_reason, ContractSubReason::Preserves);
        assert_eq!(viol.hook_id, "lossy");
        assert_eq!(viol.page_slug, "notes/one");
        assert_eq!(viol.detail, "-2 Wikilink");
        assert!(viol.observed.iter().any(|o| o.contains("2 Wikilink")));
        assert!(viol.observed.iter().any(|o| o.contains("0 Wikilink")));
    }

    #[test]
    fn preserves_multiple_types_flags_each_dropped() {
        // "multiple types (Wikilink + Embed + CodeBlock)" row.
        let input = doc(vec![para(vec![wikilink("A"), wikilink("B")]), embed("X")]);
        let output = doc(vec![para(vec![text("x")])]); // dropped both wl and embed
        let preserves = vec![
            "Wikilink".to_string(),
            "Embed".to_string(),
            "CodeBlock".to_string(),
        ];
        let v = validate_preserves(
            Stage::Transform,
            "hook",
            "page",
            &preserves,
            &input,
            &output,
        );
        // CodeBlock count is zero on both sides → no violation; the two
        // that dropped each produce one record.
        let subs: Vec<&str> = v.iter().map(|v| v.detail.as_str()).collect();
        assert!(subs.contains(&"-2 Wikilink"), "got: {subs:?}");
        assert!(subs.contains(&"-1 Embed"), "got: {subs:?}");
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn preserves_identity_produces_no_violation() {
        // A hook that echoes its input should never trip preservation.
        let input = doc(vec![para(vec![wikilink("A"), wikilink("B")]), embed("X")]);
        let output = input.clone();
        let preserves = vec!["Wikilink".into(), "Embed".into()];
        let v = validate_preserves(
            Stage::Transform,
            "identity",
            "page",
            &preserves,
            &input,
            &output,
        );
        assert!(v.is_empty());
    }

    #[test]
    fn preserves_ignores_unknown_type_names_quietly() {
        // An unknown type name matches zero nodes in both docs; no
        // violation. (Manifest parser rejects ast_type typos; the type
        // *names* inside preserves are free-form strings because the
        // CommonMark / ztl schema can grow.)
        let input = doc(vec![para(vec![wikilink("A")])]);
        let output = doc(vec![]);
        let preserves = vec!["NotAType".to_string()];
        let v = validate_preserves(
            Stage::Transform,
            "hook",
            "page",
            &preserves,
            &input,
            &output,
        );
        assert!(v.is_empty());
    }

    #[test]
    fn preserves_only_counts_strict_decrease() {
        // A hook that *added* wikilinks must not produce a violation —
        // the contract says "does not strip", not "is byte-identical".
        let input = doc(vec![para(vec![wikilink("A")])]);
        let output = doc(vec![para(vec![
            wikilink("A"),
            wikilink("B"),
            wikilink("C"),
        ])]);
        let preserves = vec!["Wikilink".to_string()];
        let v = validate_preserves(
            Stage::Transform,
            "enricher",
            "page",
            &preserves,
            &input,
            &output,
        );
        assert!(v.is_empty());
    }

    // ── validate_may_restructure ───────────────────────────────────────
    // TEST-3222 (REQ-3222 / REQ-3224).

    #[test]
    fn may_restructure_identity_shape_passes() {
        let a = doc(vec![para(vec![text("hi")])]);
        let b = doc(vec![para(vec![text("hi, world")])]); // text-only edit
        let v = validate_may_restructure(Stage::PreParse, "hook", "page", &a, &b);
        assert!(v.is_none());
    }

    #[test]
    fn may_restructure_detects_top_level_count_change() {
        let before = doc(vec![para(vec![text("a")]), para(vec![text("b")])]);
        let after = doc(vec![para(vec![text("a b")])]); // joined
        let v = validate_may_restructure(Stage::PreParse, "joiner", "page", &before, &after);
        let v = v.expect("expected a restructure violation");
        assert_eq!(v.sub_reason, ContractSubReason::MayRestructure);
        assert!(v.observed.iter().any(|o| o.contains("top-level blocks: 2")));
    }

    #[test]
    fn may_restructure_detects_kind_swap() {
        // Paragraph → Heading at same index — TEST-3222's scenario.
        let before = doc(vec![para(vec![text("title")])]);
        let after = doc(vec![Block::Heading(Heading {
            position: pos(),
            level: 1,
            children: vec![text("title")],
        })]);
        let v = validate_may_restructure(Stage::PreParse, "wrapper", "page", &before, &after);
        let v = v.expect("expected a restructure violation");
        assert!(v
            .observed
            .iter()
            .any(|o| o.contains("Paragraph") && o.contains("Heading")));
    }

    #[test]
    fn may_restructure_allows_one_layer_of_nesting() {
        // Wrapping every paragraph in a blockquote is one layer of new
        // depth — REQ-3222's "delta > 1" tolerance says that's fine.
        // But changing the top-level kind (Paragraph → BlockQuote) trips
        // the kind-swap rule, so this IS a violation. The "one layer
        // tolerance" applies when shape is otherwise preserved.
        let before = doc(vec![para(vec![text("a")])]);
        let after = doc(vec![Block::BlockQuote(BlockQuote {
            position: pos(),
            children: vec![para(vec![text("a")])],
        })]);
        let v = validate_may_restructure(Stage::PreParse, "quoter", "page", &before, &after);
        assert!(v.is_some()); // the kind swap trips it
    }

    #[test]
    fn may_restructure_detects_deep_nesting_delta() {
        // depth 1 → depth 4 (blockquote > list > item > para) — delta
        // 3, well above tolerance.
        let before = doc(vec![para(vec![text("x")])]);
        let after = doc(vec![Block::BlockQuote(BlockQuote {
            position: pos(),
            children: vec![Block::List(List {
                position: pos(),
                ordered: false,
                tight: true,
                start: None,
                children: vec![ListItem::new(
                    pos(),
                    vec![Block::BlockQuote(BlockQuote {
                        position: pos(),
                        children: vec![para(vec![text("x")])],
                    })],
                )],
            })],
        })]);
        let v = validate_may_restructure(Stage::PreParse, "deep", "page", &before, &after);
        let v = v.expect("expected a depth-delta violation");
        assert!(v.observed.iter().any(|o| o.contains("nesting depth")));
    }

    #[test]
    fn block_shape_counts_depth_for_nested_structures() {
        let d = doc(vec![Block::BlockQuote(BlockQuote {
            position: pos(),
            children: vec![para(vec![text("x")])],
        })]);
        let shape = BlockShape::of(&d);
        assert_eq!(shape.top_level_kinds, vec!["BlockQuote"]);
        assert_eq!(shape.max_depth, 2);
    }

    #[test]
    fn block_shape_flat_has_depth_one() {
        let d = doc(vec![para(vec![text("x")]), para(vec![text("y")])]);
        let shape = BlockShape::of(&d);
        assert_eq!(shape.max_depth, 1);
    }

    // ── canonicalise ───────────────────────────────────────────────────

    #[test]
    fn canonicalise_zeros_positions_recursively() {
        let d = Document {
            ast_version: AST_VERSION.to_string(),
            kind: DocumentKind::Document,
            position: Position::new(5, 5, 6, 10),
            frontmatter: None,
            children: vec![Block::BlockQuote(BlockQuote {
                position: Position::new(7, 1, 8, 4),
                children: vec![Block::Paragraph(Paragraph {
                    position: Position::new(7, 3, 7, 20),
                    children: vec![Inline::Text(Text {
                        position: Position::new(7, 5, 7, 18),
                        text: "hi".into(),
                    })],
                })],
            })],
        };
        let canon = canonicalise(&d);
        assert_eq!(canon.position, Position::origin());
        match &canon.children[0] {
            Block::BlockQuote(bq) => {
                assert_eq!(bq.position, Position::origin());
                match &bq.children[0] {
                    Block::Paragraph(p) => {
                        assert_eq!(p.position, Position::origin());
                        match &p.children[0] {
                            Inline::Text(t) => {
                                assert_eq!(t.position, Position::origin());
                                assert_eq!(t.text, "hi");
                            }
                            _ => panic!("expected text"),
                        }
                    }
                    _ => panic!("expected paragraph"),
                }
            }
            _ => panic!("expected blockquote"),
        }
    }

    #[test]
    fn canonicalise_equates_positionally_different_but_semantically_equal_docs() {
        let a = doc(vec![para(vec![text("same")])]);
        let mut b_pos = Position::origin();
        b_pos.start_line = 42;
        let b = Document {
            position: b_pos,
            children: vec![Block::Paragraph(Paragraph {
                position: Position::new(42, 1, 42, 4),
                children: vec![Inline::Text(Text {
                    position: Position::new(42, 1, 42, 4),
                    text: "same".into(),
                })],
            })],
            ..doc(vec![])
        };
        assert_ne!(a, b);
        assert_eq!(canonicalise(&a), canonicalise(&b));
    }

    // ── validate_idempotence ───────────────────────────────────────────
    // TEST-3224-idempotent (REQ-3224).

    #[test]
    fn idempotence_passes_for_an_identity_hook() {
        let input = doc(vec![para(vec![text("x")])]);
        let v =
            validate_idempotence(Stage::Transform, "identity", "page", input, |d| Ok(d)).unwrap();
        assert!(v.is_none());
    }

    #[test]
    fn idempotence_flags_hook_that_wraps_its_output() {
        // f(x) wraps every paragraph in a blockquote on each call; two
        // calls produce two layers, breaking idempotence.
        let input = doc(vec![para(vec![text("x")])]);
        let wrap = |d: Document| {
            let wrapped: Vec<Block> = d
                .children
                .into_iter()
                .map(|b| {
                    Block::BlockQuote(BlockQuote {
                        position: Position::origin(),
                        children: vec![b],
                    })
                })
                .collect();
            Ok(Document {
                children: wrapped,
                ..d
            })
        };
        let v = validate_idempotence(Stage::Transform, "wrapper", "page", input, wrap)
            .unwrap()
            .expect("expected an idempotence violation");
        assert_eq!(v.sub_reason, ContractSubReason::Idempotent);
        assert_eq!(v.detail, "f(f(x)) != f(x)");
        // First run: 1 blockquote. Second: still 1 blockquote at top
        // level — but depth doubled. Observed message must report the
        // shape of the difference, not just "mismatch".
        assert!(v.observed.iter().any(|o| o.contains("canonicalise")));
    }

    #[test]
    fn idempotence_treats_position_only_differences_as_equivalent() {
        // A hook whose only change is source position must not trip
        // the idempotence check — NFR-3305 canonical form.
        let input = doc(vec![para(vec![text("x")])]);
        let pos_shifter = |mut d: Document| {
            d.position = Position::new(99, 99, 99, 99);
            Ok(d)
        };
        let v =
            validate_idempotence(Stage::Transform, "shifter", "page", input, pos_shifter).unwrap();
        assert!(v.is_none());
    }

    // ── validate_expansion_bound ───────────────────────────────────────

    #[test]
    fn expansion_bound_inside_budget_returns_none() {
        let v = validate_expansion_bound(Stage::PostRender, "h", "p", 3.0, 1000, 1500);
        assert!(v.is_none());
    }

    #[test]
    fn expansion_bound_over_budget_produces_violation() {
        let v = validate_expansion_bound(Stage::PostRender, "h", "p", 2.0, 1000, 3000);
        let v = v.expect("expected an expansion-bound violation");
        assert_eq!(v.sub_reason, ContractSubReason::ExpansionBound);
        assert!(v.observed.iter().any(|o| o.contains("ratio: 3.00x")));
    }

    #[test]
    fn expansion_bound_handles_zero_input_size_gracefully() {
        // Division-by-zero edge: if input size is 0 we still compute a
        // meaningful ratio by flooring denom at 1.
        let v = validate_expansion_bound(Stage::Transform, "h", "p", 1.0, 0, 5);
        assert!(v.is_some());
    }

    // ── to_failure_record / to_diagnostic ──────────────────────────────

    #[test]
    fn violation_to_failure_record_uses_contract_violation_reason() {
        let viol = ContractViolation {
            sub_reason: ContractSubReason::Preserves,
            stage: Stage::Transform,
            hook_id: "callouts".into(),
            page_slug: "projects/q2".into(),
            detail: "-4 Wikilink".into(),
            observed: vec![],
        };
        let rec = viol.to_failure_record(Duration::from_millis(12));
        assert_eq!(rec.hook, "callouts");
        assert_eq!(rec.stage, "transform");
        assert_eq!(rec.page_slug, "projects/q2");
        assert_eq!(rec.reason, "contract_violation");
        assert_eq!(rec.detail, "preserves: -4 Wikilink");
        assert_eq!(rec.duration_ms, 12);
    }

    #[test]
    fn violation_to_diagnostic_matches_con_3225_structure() {
        let viol = ContractViolation {
            sub_reason: ContractSubReason::Preserves,
            stage: Stage::Transform,
            hook_id: "callouts".into(),
            page_slug: "projects/q2-review.md".into(),
            detail: "-4 Wikilink".into(),
            observed: vec![
                "observed in input:  12 Wikilink, 3 Embed".into(),
                "observed in output:  8 Wikilink, 3 Embed".into(),
                "net change: -4 Wikilink".into(),
            ],
        };
        let d = viol.to_diagnostic(r#"contract.preserves = ["Wikilink", "Embed"]"#);
        assert_eq!(d.class, DiagnosticClass::ContractViolation);
        let rendered = d.to_string();
        assert!(rendered.starts_with("[ztl] hook 'callouts'"));
        assert!(rendered.contains("projects/q2-review.md"));
        assert!(rendered.contains("preserves"));
        assert!(rendered.contains(r#"contract.preserves = ["Wikilink", "Embed"]"#));
        assert!(rendered.contains("Likely cause:"));
        assert!(rendered.contains("Hint:"));
    }

    #[test]
    fn violation_diagnostic_without_page_slug_drops_the_on_clause() {
        let viol = ContractViolation {
            sub_reason: ContractSubReason::Idempotent,
            stage: Stage::Transform,
            hook_id: "tasks".into(),
            page_slug: String::new(),
            detail: "f(f(x)) != f(x)".into(),
            observed: vec![],
        };
        let d = viol.to_diagnostic("contract.idempotent = true");
        let rendered = d.to_string();
        assert!(rendered.contains("hook 'tasks' contract violation (idempotent)"));
        assert!(!rendered.contains(" on :"));
    }

    // ── run_property_test ──────────────────────────────────────────────

    fn make_contract(
        preserves: Vec<&str>,
        idempotent: bool,
        may_restructure: bool,
    ) -> ContractDecl {
        ContractDecl {
            preserves: preserves.into_iter().map(String::from).collect(),
            idempotent,
            may_restructure,
            pure: false,
            expansion_bound: None,
        }
    }

    #[test]
    fn property_test_identity_hook_passes_every_tier1_check() {
        let case = PropertyTestCase {
            name: "identity-passes".into(),
            contract: make_contract(vec!["Wikilink", "Embed"], true, false),
            stage: Stage::Transform,
            input: doc(vec![para(vec![wikilink("A")]), embed("X")]),
            input_size: 0,
        };
        let report = run_property_test(&case, "identity", "page", |d| Ok(d)).unwrap();
        assert!(report.passed(), "expected empty report, got {:?}", report);
        assert_eq!(report.case_name, "identity-passes");
    }

    #[test]
    fn property_test_catches_preserves_and_idempotence_in_one_pass() {
        // A hook that (a) strips wikilinks AND (b) is not idempotent —
        // the report should carry both violations.
        let case = PropertyTestCase {
            name: "lossy-wrapper".into(),
            contract: make_contract(vec!["Wikilink"], true, false),
            stage: Stage::Transform,
            input: doc(vec![para(vec![wikilink("A"), text(" "), wikilink("B")])]),
            input_size: 0,
        };
        let report = run_property_test(&case, "bad", "p", |d: Document| {
            // First call: drop wikilinks; second call: wrap result in a
            // new paragraph (breaks idempotence).
            let stripped: Vec<Block> = d
                .children
                .into_iter()
                .map(|b| match b {
                    Block::Paragraph(p) => Block::Paragraph(Paragraph {
                        position: Position::origin(),
                        children: p
                            .children
                            .into_iter()
                            .filter(|c| !matches!(c, Inline::Wikilink(_)))
                            .collect(),
                    }),
                    other => other,
                })
                .collect();
            Ok(Document {
                children: vec![para(vec![text("wrapped")])]
                    .into_iter()
                    .chain(stripped)
                    .collect(),
                ..d
            })
        })
        .unwrap();
        assert!(!report.passed());
        let subs: Vec<ContractSubReason> = report.violations.iter().map(|v| v.sub_reason).collect();
        assert!(subs.contains(&ContractSubReason::Preserves));
        assert!(subs.contains(&ContractSubReason::Idempotent));
    }

    #[test]
    fn property_test_skips_may_restructure_on_non_pre_parse_stages() {
        // A transform hook that swaps kinds shouldn't trip
        // may_restructure — that check is pre-parse-only per REQ-3222.
        let case = PropertyTestCase {
            name: "transform-restructure".into(),
            contract: make_contract(vec![], false, false),
            stage: Stage::Transform,
            input: doc(vec![para(vec![text("x")])]),
            input_size: 0,
        };
        let report = run_property_test(&case, "h", "p", |_| {
            Ok(doc(vec![Block::Heading(Heading {
                position: pos(),
                level: 1,
                children: vec![text("x")],
            })]))
        })
        .unwrap();
        assert!(report.passed());
    }

    #[test]
    fn property_test_checks_may_restructure_on_pre_parse_stage() {
        let case = PropertyTestCase {
            name: "restructuring-preparse".into(),
            contract: make_contract(vec![], false, false),
            stage: Stage::PreParse,
            input: doc(vec![para(vec![text("x")])]),
            input_size: 0,
        };
        let report = run_property_test(&case, "h", "p", |_| {
            Ok(doc(vec![Block::Heading(Heading {
                position: pos(),
                level: 1,
                children: vec![text("x")],
            })]))
        })
        .unwrap();
        assert_eq!(report.violations.len(), 1);
        assert_eq!(
            report.violations[0].sub_reason,
            ContractSubReason::MayRestructure
        );
    }

    #[test]
    fn property_test_honours_may_restructure_opt_out() {
        // may_restructure = true → no violation even when the tree is
        // rewritten.
        let case = PropertyTestCase {
            name: "allowed-restructure".into(),
            contract: make_contract(vec![], false, true),
            stage: Stage::PreParse,
            input: doc(vec![para(vec![text("x")])]),
            input_size: 0,
        };
        let report = run_property_test(&case, "h", "p", |_| {
            Ok(doc(vec![Block::Heading(Heading {
                position: pos(),
                level: 1,
                children: vec![text("x")],
            })]))
        })
        .unwrap();
        assert!(report.passed());
    }

    #[test]
    fn property_test_surfaces_expansion_bound_violation() {
        let case = PropertyTestCase {
            name: "big-output".into(),
            contract: ContractDecl {
                expansion_bound: Some(1.2),
                ..make_contract(vec![], false, false)
            },
            stage: Stage::Transform,
            input: doc(vec![para(vec![text("tiny")])]),
            input_size: 10,
        };
        // Output serialises to much more than 12 bytes; the bound is
        // 1.2x so this trips.
        let report = run_property_test(&case, "h", "p", |d| Ok(d)).unwrap();
        let subs: Vec<_> = report.violations.iter().map(|v| v.sub_reason).collect();
        assert!(subs.contains(&ContractSubReason::ExpansionBound));
    }

    #[test]
    fn property_test_short_circuits_on_hook_error() {
        let case = PropertyTestCase {
            name: "fails".into(),
            contract: make_contract(vec![], true, false),
            stage: Stage::Transform,
            input: doc(vec![]),
            input_size: 0,
        };
        let err = run_property_test(&case, "h", "p", |_| {
            Err(HookError::new(Stage::Transform, "h", "boom"))
        })
        .unwrap_err();
        assert_eq!(err.reason, "boom");
    }
}
