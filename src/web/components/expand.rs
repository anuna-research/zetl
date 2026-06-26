//! SPEC-049 REQ-4906/4907/4908 — transform-stage directive expansion.
//!
//! Expands a recognised directive [`Node`] tree (CON-4901) into HTML, pinning the order
//! **recognise → sanitise-body-in-isolation → compose → render** (REQ-4906):
//!
//! 1. Expansion is **bottom-up, innermost-directive-first**, with a **per-provenance
//!    sanitisation barrier**: each directive's *own* untrusted body text is rendered to
//!    HTML and sanitised **in isolation** (CON-4902), while each already-expanded nested
//!    directive is a **trusted composed fragment** spliced in *without* re-sanitisation.
//!    So a nested component's legitimate `<form>`/`<button>` is never stripped, and every
//!    directive's own untrusted text is sanitised exactly once.
//! 2. The composed slot + recognised props (REQ-4904) drive the SPEC-048 component
//!    render through a **restricted, deny-by-default content context** (REQ-4907): only
//!    `props` + the sanitised slot — no `transclude`, no page-tier, no vault graph.
//! 3. Expansion is **depth-bounded** ([`MAX_CONTENT_DIRECTIVE_DEPTH`]); component-
//!    invocation cycles are caught by the SPEC-048 build-time cycle check. Component
//!    template output is **never re-scanned** for directives.
//!
//! A directive that names an unknown or non-content-invocable component (REQ-4903) fails
//! **closed**: its body is emitted **sanitised and inert** (no component), with a
//! diagnostic (REQ-4911) — never expanded, never raw HTML.
//!
//! The actual Markdown render and SPEC-048 component render are injected via
//! [`ContentRenderer`] so this module is unit-testable without the full engine; the wire
//! layer supplies the real implementations.

use super::directive::{Directive, DirectiveForm, Node};
use super::manifest::Manifest;
use super::sanitize::{sanitise_body, SanitiseReport};
use super::{content_props, CResult, ComponentError};
use minijinja::Value;
use std::collections::BTreeMap;

/// REQ-4908 / Q3 `[Provisional]` maximum content-directive nesting depth.
pub const MAX_CONTENT_DIRECTIVE_DEPTH: usize = 16;

/// A build-time, author-facing diagnostic tied to a content-Markdown source location
/// (REQ-4911 / OBS-4901). Decoupled from the hooks subsystem so the recogniser stays
/// pure; the build layer decides how to render it and whether it fails `--strict`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentDiagnostic {
    /// Stable SPEC-049 error code (`content-directive-unknown`, `content-prop-type`, …).
    pub code: String,
    /// Human message — never echoes the rejected payload verbatim (anti-oracle).
    pub message: String,
    /// 1-based source line in the author's Markdown.
    pub line: usize,
    /// True when the failure is fatal under `--strict` (depth/cycle), as opposed to a
    /// per-directive fail-closed-to-inert refusal.
    pub fatal: bool,
}

/// The result of expanding a content node tree.
#[derive(Debug, Default)]
pub struct Expansion {
    /// The composed HTML (Markdown-rendered text + expanded component HTML).
    pub html: String,
    /// Diagnostics raised during expansion (REQ-4911).
    pub diagnostics: Vec<ContentDiagnostic>,
    /// Aggregated sanitiser drop report across all bodies (OBS-4901).
    pub report: SanitiseReport,
}

impl Expansion {
    fn merge_report(&mut self, other: &SanitiseReport) {
        for (k, n) in &other.dropped {
            *self.report.dropped.entry(k.clone()).or_insert(0) += n;
        }
    }
}

/// Injected build-side capabilities the expander needs.
pub trait ContentRenderer {
    /// Render untrusted body Markdown to HTML (the same pipeline page content uses).
    fn render_markdown(&self, md: &str) -> String;

    /// Render a resolved, content-invocable component with recognised `props` and a
    /// pre-sanitised, trusted `slot_html`, through the **restricted content context**
    /// (REQ-4907) with strict-undefined on. Returns the rendered component HTML.
    fn render_component(
        &self,
        name: &str,
        props: &BTreeMap<String, Value>,
        slot_html: &str,
    ) -> CResult<String>;

    /// The manifest of a resolved component, or `None` if it does not exist.
    fn manifest(&self, name: &str) -> Option<&Manifest>;
}

/// Expand a content node tree into HTML. Top-level `Text` nodes are rendered as Markdown;
/// `Directive` nodes are expanded (REQ-4906) or emitted inert (REQ-4903) on failure.
pub fn expand_nodes(nodes: &[Node], r: &dyn ContentRenderer) -> Expansion {
    let mut out = Expansion::default();
    for node in nodes {
        match node {
            Node::Text(t) => out.html.push_str(&r.render_markdown(t)),
            Node::Directive(d) => expand_into(&mut out, d, r, 1),
        }
    }
    out
}

/// Expand one directive, appending HTML + diagnostics to `out`.
fn expand_into(out: &mut Expansion, d: &Directive, r: &dyn ContentRenderer, depth: usize) {
    if depth > MAX_CONTENT_DIRECTIVE_DEPTH {
        out.diagnostics.push(ContentDiagnostic {
            code: "content-directive-too-deep".into(),
            message: format!(
                "content-directive nesting exceeds the depth bound of {MAX_CONTENT_DIRECTIVE_DEPTH}"
            ),
            line: d.line,
            fatal: true,
        });
        return; // fail closed — drop, no recursion
    }

    // REQ-4903 default-deny: resolve to a content-invocable component, else inert.
    let invocable = matches!(r.manifest(&d.name), Some(m) if m.content_invocable);
    if !invocable {
        out.diagnostics.push(ContentDiagnostic {
            code: "content-directive-unknown".into(),
            message: format!(
                "`{}` is not a content-invocable component; directive emitted inert",
                d.name
            ),
            line: d.line,
            fatal: false,
        });
        emit_inert(out, d, r, depth);
        return;
    }

    // Compose the slot from the directive's own body (per-provenance barrier, REQ-4906).
    let slot_html = match d.form {
        DirectiveForm::Container => compose_slot(out, &d.children, r, depth),
        DirectiveForm::Inline => {
            // The inline `[label]` is the (text) body: render + sanitise in isolation.
            let label = d.label.as_deref().unwrap_or("");
            let (clean, rep) = sanitise_body(&r.render_markdown(label));
            out.merge_report(&rep);
            clean
        }
        DirectiveForm::Leaf => String::new(),
    };

    // Recognise props against the manifest (REQ-4904).
    let manifest = r
        .manifest(&d.name)
        .expect("invocable implies manifest present");
    let recognised = match content_props::recognise(&d.attrs, manifest) {
        Ok(rec) => rec,
        Err(e) => {
            out.diagnostics.push(diag_from(&e, d.line));
            // fail closed: emit the (already-sanitised) slot inert, no component
            out.html.push_str(&slot_html);
            return;
        }
    };

    // Compose + render the trusted component (REQ-4906 step 3).
    match r.render_component(&d.name, &recognised.props, &slot_html) {
        Ok(html) => out.html.push_str(&html),
        Err(e) => {
            out.diagnostics.push(diag_from(&e, d.line));
            out.html.push_str(&slot_html); // inert fallback
        }
    }
}

/// Compose a container's slot: own text children are rendered + sanitised in isolation;
/// nested directives are expanded to trusted fragments and spliced in unsanitised.
fn compose_slot(
    out: &mut Expansion,
    children: &[Node],
    r: &dyn ContentRenderer,
    depth: usize,
) -> String {
    let mut slot = String::new();
    for child in children {
        match child {
            Node::Text(t) => {
                // own untrusted body → render Markdown then sanitise in isolation
                let (clean, rep) = sanitise_body(&r.render_markdown(t));
                out.merge_report(&rep);
                slot.push_str(&clean);
            }
            Node::Directive(nested) => {
                // already-composed trusted fragment — NOT re-sanitised (the barrier)
                let mut sub = Expansion::default();
                expand_into(&mut sub, nested, r, depth + 1);
                out.diagnostics.extend(sub.diagnostics);
                out.merge_report(&sub.report);
                slot.push_str(&sub.html);
            }
        }
    }
    slot
}

/// Emit a failed/unknown directive's body **sanitised and inert** (no component), so the
/// author's content is preserved but never executed (HP2, REQ-4911).
fn emit_inert(out: &mut Expansion, d: &Directive, r: &dyn ContentRenderer, depth: usize) {
    match d.form {
        DirectiveForm::Container => {
            let slot = compose_slot(out, &d.children, r, depth);
            out.html.push_str(&slot);
        }
        DirectiveForm::Inline => {
            let label = d.label.as_deref().unwrap_or("");
            let (clean, rep) = sanitise_body(&r.render_markdown(label));
            out.merge_report(&rep);
            out.html.push_str(&clean);
        }
        DirectiveForm::Leaf => {}
    }
}

fn diag_from(e: &ComponentError, line: usize) -> ContentDiagnostic {
    ContentDiagnostic {
        code: e.code.to_string(),
        message: e.message.clone(),
        line,
        fatal: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::components::directive::scan;
    use crate::web::components::manifest::parse_manifest;

    /// A mock renderer: Markdown is wrapped in `<p>…</p>`; the component renders as
    /// `<div data-z="name">props|slot</div>` so tests can see props + slot composition.
    struct Mock {
        manifests: BTreeMap<String, Manifest>,
    }

    impl Mock {
        fn new(srcs: &[(&str, &str)]) -> Self {
            let mut manifests = BTreeMap::new();
            for (dir, src) in srcs {
                manifests.insert(dir.to_string(), parse_manifest(src, dir).unwrap());
            }
            Mock { manifests }
        }
    }

    impl ContentRenderer for Mock {
        fn render_markdown(&self, md: &str) -> String {
            // trivial: keep text, but include a fake raw-HTML script to test sanitising
            format!("<p>{}</p>", md.trim())
        }
        fn render_component(
            &self,
            name: &str,
            props: &BTreeMap<String, Value>,
            slot_html: &str,
        ) -> CResult<String> {
            let propstr: Vec<String> = props.iter().map(|(k, v)| format!("{k}={v}")).collect();
            Ok(format!(
                "<div data-z=\"{name}\" data-props=\"{}\">{slot_html}</div>",
                propstr.join(",")
            ))
        }
        fn manifest(&self, name: &str) -> Option<&Manifest> {
            self.manifests.get(name)
        }
    }

    const CALLOUT: &str = r#"
        name = "callout"
        content_invocable = true
        content_props = ["tone"]
        [props]
        tone = { type = "string", default = "info", enum = ["info", "warning"] }
    "#;

    #[test]
    fn expands_simple_container() {
        let mock = Mock::new(&[("callout", CALLOUT)]);
        let nodes = scan(":::callout{tone=warning}\nbody text\n:::\n");
        let out = expand_nodes(&nodes, &mock);
        assert!(out.html.contains("data-z=\"callout\""));
        assert!(out.html.contains("tone=warning"));
        assert!(out.html.contains("body text"));
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn unknown_component_inert_with_diagnostic() {
        let mock = Mock::new(&[("callout", CALLOUT)]);
        let nodes = scan(":::raw-html{}\nhello world\n:::\n");
        let out = expand_nodes(&nodes, &mock);
        assert!(!out.html.contains("data-z"));
        assert!(
            out.html.contains("hello world"),
            "body preserved inert: {}",
            out.html
        );
        assert_eq!(out.diagnostics[0].code, "content-directive-unknown");
    }

    #[test]
    fn non_invocable_component_inert() {
        // a component that exists but is not content_invocable
        let trusted = r#"
            name = "secret"
            [props]
        "#;
        let mock = Mock::new(&[("secret", trusted)]);
        let nodes = scan(":::secret{}\nx\n:::\n");
        let out = expand_nodes(&nodes, &mock);
        assert!(!out.html.contains("data-z"));
        assert_eq!(out.diagnostics[0].code, "content-directive-unknown");
    }

    #[test]
    fn prop_error_fails_closed_inert() {
        let mock = Mock::new(&[("callout", CALLOUT)]);
        let nodes = scan(":::callout{tone=nonsense}\nbody\n:::\n");
        let out = expand_nodes(&nodes, &mock);
        assert!(!out.html.contains("data-z"), "no component on prop error");
        assert!(out.html.contains("body"));
        assert_eq!(out.diagnostics[0].code, "content-prop-enum");
    }

    #[test]
    fn nested_directive_trusted_fragment_not_resanitised() {
        // outer wraps inner; inner's component output (with a <button>) must survive the
        // outer body sanitiser (per-provenance barrier).
        let wrapper = r#"
            name = "wrapper"
            content_invocable = true
            content_props = []
            [props]
        "#;
        let widget = r#"
            name = "widget"
            content_invocable = true
            content_props = []
            [props]
        "#;
        let mut mock = Mock::new(&[("wrapper", wrapper), ("widget", widget)]);
        // make widget render a <button> (forbidden in the body sanitiser allowlist)
        struct ButtonMock(Mock);
        impl ContentRenderer for ButtonMock {
            fn render_markdown(&self, md: &str) -> String {
                self.0.render_markdown(md)
            }
            fn render_component(
                &self,
                name: &str,
                _props: &BTreeMap<String, Value>,
                slot: &str,
            ) -> CResult<String> {
                if name == "widget" {
                    Ok("<button>click</button>".into())
                } else {
                    Ok(format!("<section>{slot}</section>"))
                }
            }
            fn manifest(&self, name: &str) -> Option<&Manifest> {
                self.0.manifest(name)
            }
        }
        let bm = ButtonMock(std::mem::replace(&mut mock, Mock::new(&[])));
        let nodes = scan("::::wrapper{}\nintro\n:::widget{}\n:::\n::::\n");
        let out = expand_nodes(&nodes, &bm);
        assert!(
            out.html.contains("<button>click</button>"),
            "nested trusted output survives: {}",
            out.html
        );
        assert!(out.html.contains("<section>"));
    }

    #[test]
    fn depth_bound_fails_closed() {
        // Build a directive nested past the bound.
        let comp = r#"
            name = "n"
            content_invocable = true
            content_props = []
            [props]
        "#;
        let mock = Mock::new(&[("n", comp)]);
        let mut src = String::new();
        let levels = MAX_CONTENT_DIRECTIVE_DEPTH + 2;
        for i in 0..levels {
            let colons = ":".repeat(3 + (levels - i));
            src.push_str(&format!("{colons}n{{}}\n"));
        }
        src.push_str("deep\n");
        for i in 0..levels {
            let colons = ":".repeat(3 + i + 1);
            src.push_str(&format!("{colons}\n"));
        }
        let nodes = scan(&src);
        let out = expand_nodes(&nodes, &mock);
        assert!(
            out.diagnostics
                .iter()
                .any(|d| d.code == "content-directive-too-deep" && d.fatal),
            "depth bound triggers a fatal diagnostic: {:?}",
            out.diagnostics
        );
    }
}
