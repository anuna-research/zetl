//! SPEC-049 CON-4904 — sound static HTML-context lint over the minijinja AST.
//!
//! A build-time check that no content-derived value (a content-settable prop, or the
//! sanitised slot) is interpolated into an unsafe HTML context in a content-invocable
//! component's template. It adopts the Go `html/template` context-autoescaper technique
//! over minijinja's `machinery::parse` AST (NFR-4903 pins the version + feature set).
//!
//! "Sound" means **no false negatives**: it never *misses* an unsafe placement. It
//! achieves this by **failing closed** — anything it cannot prove safe is rejected
//! (`content-context-unsafe` / `-indeterminate` / `-unsafe-emit` / `-template-unanalyzable`).
//! It will reject some safe templates (the accepted trade-off at a trust boundary).
//!
//! The AST `match`es are **exhaustive with no `_` arm** (NFR-4903(b)): a minijinja
//! upgrade that adds a `Stmt`/`Expr` variant will fail to *compile*, not silently pass.
//! The cfg gates (`multi_template`, `macros`, `loop_controls`) mirror the pinned feature
//! set so the arms match exactly what is compiled.

use super::manifest::{Manifest, PropType};
use super::sanitize::is_url_attr;
use super::{CResult, ComponentError};
use minijinja::machinery::{ast, parse, WhitespaceConfig};
use minijinja::syntax::SyntaxConfig;
use std::collections::BTreeMap;

/// The taint kind of an expression: which untrusted value (if any) it carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Taint {
    /// Not content-derived — safe to place anywhere.
    None,
    /// A content scalar prop (`string`/`bool`/`int`/`number`) — HTML-autoescape-safe in
    /// TEXT and double-quoted attribute only.
    Scalar,
    /// A `url`-typed content prop — additionally safe in a URL attribute (scheme was
    /// ingestion-validated, CON-4902).
    Url,
    /// The sanitised slot (`caller()`) — safe HTML in TEXT only.
    Slot,
}

impl Taint {
    fn join(self, other: Taint) -> Taint {
        // Combining any taint with another yields the most-restrictive non-None taint.
        // Slot is most restrictive, then Scalar, then Url, then None.
        use Taint::*;
        match (self, other) {
            (None, x) | (x, None) => x,
            (Slot, _) | (_, Slot) => Slot,
            (Scalar, _) | (_, Scalar) => Scalar,
            (Url, Url) => Url,
        }
    }
}

/// HTML tokeniser state (CON-4904(1)).
#[derive(Debug, Clone, PartialEq, Eq)]
enum St {
    Text,
    Comment,
    /// Inside `<script>` (JS) — value text is a JS context.
    RawJs,
    /// Inside `<style>` (CSS).
    RawCss,
    /// Inside `<title>`/`<textarea>`.
    Rcdata,
    TagOpen,
    EndTagOpen,
    TagName,
    BeforeAttrName,
    AttrName,
    AfterAttrName,
    BeforeAttrValue,
    AttrValueDq,
    AttrValueSq,
    AttrValueUq,
}

/// The classified context at an interpolation point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ctx {
    Text,
    Rcdata,
    AttrValueDq,
    AttrValueSq,
    AttrValueUq,
    /// A double-quoted URL attribute (`href="…"`).
    UrlAttrDq,
    Css,
    Js,
    Comment,
    /// Interpolating a tag/attribute *name* or otherwise unanalyzable position.
    Unanalyzable,
    /// The literal HTML did not resolve to a single well-defined context.
    Indeterminate,
}

/// Incremental HTML tokeniser over the template's literal `EmitRaw` spans.
#[derive(Debug, Clone, PartialEq, Eq)]
struct HtmlState {
    st: St,
    tag: String,
    attr: String,
    /// Set once the state machine has seen syntax it cannot model — every subsequent
    /// query returns `Indeterminate` (fail closed).
    poisoned: bool,
}

impl HtmlState {
    fn new() -> Self {
        HtmlState {
            st: St::Text,
            tag: String::new(),
            attr: String::new(),
            poisoned: false,
        }
    }

    /// Feed a literal text span, advancing the state machine.
    fn feed(&mut self, raw: &str) {
        let chars: Vec<char> = raw.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            match self.st {
                St::Text => {
                    if c == '<' {
                        // comment? `<!--`
                        if chars.get(i + 1) == Some(&'!')
                            && chars.get(i + 2) == Some(&'-')
                            && chars.get(i + 3) == Some(&'-')
                        {
                            self.st = St::Comment;
                            i += 4;
                            continue;
                        }
                        if chars.get(i + 1) == Some(&'/') {
                            self.st = St::EndTagOpen;
                            self.tag.clear();
                            i += 2;
                            continue;
                        }
                        self.st = St::TagOpen;
                        self.tag.clear();
                    }
                }
                St::Comment => {
                    if c == '-' && chars.get(i + 1) == Some(&'-') && chars.get(i + 2) == Some(&'>')
                    {
                        self.st = St::Text;
                        i += 3;
                        continue;
                    }
                }
                St::RawJs | St::RawCss | St::Rcdata => {
                    // exit on the matching close tag `</tag`
                    if c == '<' && chars.get(i + 1) == Some(&'/') {
                        let close: String = chars[i + 2..]
                            .iter()
                            .take_while(|c| c.is_ascii_alphanumeric())
                            .collect::<String>()
                            .to_ascii_lowercase();
                        if close == self.tag {
                            // An end tag never opens content — return to Text and FORGET
                            // the tag. The previous `st = TagName; tag = close` re-entered
                            // RawJs/RawCss via enter_content() (keyed on the still-set tag)
                            // and never left raw mode (BUG-4). EndTagOpen consumes the tag
                            // name then `>` → Text.
                            self.st = St::EndTagOpen;
                            self.tag.clear();
                            i += 2;
                            continue;
                        }
                    }
                }
                St::TagOpen => {
                    if c.is_ascii_alphabetic() {
                        self.st = St::TagName;
                        self.tag.push(c.to_ascii_lowercase());
                    } else {
                        // not a tag after all
                        self.st = St::Text;
                        continue;
                    }
                }
                St::EndTagOpen => {
                    if c == '>' {
                        self.st = St::Text;
                    }
                    // ignore the end-tag name
                }
                St::TagName => match c {
                    c if c.is_ascii_alphanumeric() || c == '-' => self.tag.push(c.to_ascii_lowercase()),
                    c if c.is_whitespace() => self.st = St::BeforeAttrName,
                    '>' => self.st = self.enter_content(),
                    '/' => {} // self-closing solidus
                    _ => self.poisoned = true,
                },
                St::BeforeAttrName => match c {
                    c if c.is_whitespace() => {}
                    '>' => self.st = self.enter_content(),
                    '/' => {}
                    _ => {
                        self.st = St::AttrName;
                        self.attr.clear();
                        self.attr.push(c.to_ascii_lowercase());
                    }
                },
                St::AttrName => match c {
                    c if c.is_ascii_alphanumeric() || c == '-' || c == ':' => {
                        self.attr.push(c.to_ascii_lowercase())
                    }
                    '=' => self.st = St::BeforeAttrValue,
                    c if c.is_whitespace() => self.st = St::AfterAttrName,
                    '>' => self.st = self.enter_content(),
                    _ => self.poisoned = true,
                },
                St::AfterAttrName => match c {
                    c if c.is_whitespace() => {}
                    '=' => self.st = St::BeforeAttrValue,
                    '>' => self.st = self.enter_content(),
                    '/' => {}
                    _ => {
                        self.st = St::AttrName;
                        self.attr.clear();
                        self.attr.push(c.to_ascii_lowercase());
                    }
                },
                St::BeforeAttrValue => match c {
                    c if c.is_whitespace() => {}
                    '"' => self.st = St::AttrValueDq,
                    '\'' => self.st = St::AttrValueSq,
                    '>' => self.st = self.enter_content(),
                    _ => {
                        self.st = St::AttrValueUq;
                        continue;
                    }
                },
                St::AttrValueDq => {
                    if c == '"' {
                        self.st = St::BeforeAttrName;
                    }
                }
                St::AttrValueSq => {
                    if c == '\'' {
                        self.st = St::BeforeAttrName;
                    }
                }
                St::AttrValueUq => match c {
                    c if c.is_whitespace() => self.st = St::BeforeAttrName,
                    '>' => self.st = self.enter_content(),
                    _ => {}
                },
            }
            i += 1;
        }
    }

    /// When a start tag closes with `>`, decide the content state for special elements.
    fn enter_content(&self) -> St {
        match self.tag.as_str() {
            "script" => St::RawJs,
            "style" => St::RawCss,
            "title" | "textarea" => St::Rcdata,
            _ => St::Text,
        }
    }

    /// The classified context at an interpolation in the current state.
    fn context(&self) -> Ctx {
        if self.poisoned {
            return Ctx::Indeterminate;
        }
        match self.st {
            St::Text => Ctx::Text,
            St::Rcdata => Ctx::Rcdata,
            St::Comment => Ctx::Comment,
            St::RawJs => Ctx::Js,
            St::RawCss => Ctx::Css,
            // interpolating a tag/attr name or in the middle of tag structure
            St::TagOpen | St::EndTagOpen | St::TagName | St::BeforeAttrName | St::AttrName
            | St::AfterAttrName => Ctx::Unanalyzable,
            // An interpolation right after `=` is the whole unquoted attribute value
            // (had it been quoted, the opening quote would already be in an EmitRaw).
            St::BeforeAttrValue => Ctx::AttrValueUq,
            St::AttrValueUq => Ctx::AttrValueUq,
            St::AttrValueDq => self.attr_value_ctx(true),
            St::AttrValueSq => self.attr_value_ctx(false),
        }
    }

    fn attr_value_ctx(&self, double_quoted: bool) -> Ctx {
        let a = self.attr.as_str();
        if a == "style" {
            return Ctx::Css;
        }
        if a.starts_with("on") && a.len() > 2 {
            return Ctx::Js;
        }
        // `srcdoc` (and `xlink:href`-style HTML/markup-valued attributes) are parsed as a
        // *document* by the browser, which entity-decodes the value — so an HTML-autoescaped
        // content string still becomes live markup there. Treat as an unsafe (HTML) context
        // so any tainted value in it is rejected (Codex P1). `Js` is in no safe-context set.
        if a == "srcdoc" {
            return Ctx::Js;
        }
        if is_url_attr(a) {
            // a single-quoted URL attr is not a sanctioned context for a url prop
            return if double_quoted { Ctx::UrlAttrDq } else { Ctx::AttrValueSq };
        }
        if double_quoted {
            Ctx::AttrValueDq
        } else {
            Ctx::AttrValueSq
        }
    }
}

/// Lint environment: content-prop taint kinds + locally-bound (set/with/for) taint.
struct Env<'m> {
    manifest: &'m Manifest,
    /// Names bound by `{% set %}`/`{% with %}`/`{% for %}` to a taint kind.
    locals: BTreeMap<String, Taint>,
}

impl<'m> Env<'m> {
    /// Taint of an attribute access `props.<name>` / scalar/url by ptype.
    fn prop_taint(&self, name: &str) -> Taint {
        if !self.manifest.is_content_settable_prop(name) {
            return Taint::None;
        }
        match self.manifest.props.get(name).map(|d| d.ty) {
            Some(PropType::Url) => Taint::Url,
            Some(_) => Taint::Scalar,
            None => Taint::None,
        }
    }
}

/// Result of expression taint analysis: its taint, plus whether a `safe`/`raw` escaper
/// bypass was applied to tainted data (→ build error).
struct ExprTaint {
    taint: Taint,
    unsafe_emit: bool,
}

impl ExprTaint {
    fn none() -> Self {
        ExprTaint { taint: Taint::None, unsafe_emit: false }
    }
}

fn err(code: &'static str, msg: impl Into<String>) -> ComponentError {
    ComponentError::new(code, msg.into())
}

/// Lint a content-invocable component's template source (CON-4904). `manifest` supplies
/// the content-settable prop set + types. Returns `Ok(())` if every content-derived value
/// reaches only a safe context, or a fail-closed build error.
pub fn lint_component(template_src: &str, manifest: &Manifest) -> CResult<()> {
    // (0) whitespace-trim precondition: forbid manual trim markers for content templates
    // (they mutate the literal spans the tokeniser reads — B-3).
    for marker in ["-%}", "{%-", "-}}", "{{-"] {
        if template_src.contains(marker) {
            return Err(err(
                "content-template-unanalyzable",
                format!("content-invocable template uses whitespace-trim marker `{marker}` (forbidden by CON-4904(0))"),
            ));
        }
    }

    let ast = parse(
        template_src,
        &manifest.name,
        SyntaxConfig,
        WhitespaceConfig::default(),
    )
    .map_err(|e| err("content-template-unanalyzable", format!("template parse error: {e}")))?;

    let mut env = Env {
        manifest,
        locals: BTreeMap::new(),
    };
    let mut state = HtmlState::new();
    // autoescape is enforced Html at render for content components; a non-Html region
    // over taint is rejected. `escape_html` tracks the current region.
    walk_stmt(&ast, &mut state, &mut env, true)?;
    Ok(())
}

/// Walk a statement, threading the HTML tokeniser state. Returns the final state is
/// carried via `state` (mutable). `escape_html` is the current autoescape mode.
fn walk_stmt(
    stmt: &ast::Stmt,
    state: &mut HtmlState,
    env: &mut Env,
    escape_html: bool,
) -> CResult<()> {
    match stmt {
        ast::Stmt::Template(t) => {
            walk_body(&t.children, state, env, escape_html)?;
        }
        ast::Stmt::EmitRaw(r) => {
            state.feed(r.raw);
        }
        ast::Stmt::EmitExpr(e) => {
            let et = analyse_expr(&e.expr, env)?;
            check_emit(et, state, escape_html)?;
        }
        ast::Stmt::ForLoop(f) => {
            // taint of the loop variable follows the iterable's taint (conservative)
            let iter_t = analyse_expr(&f.iter, env)?;
            if iter_t.unsafe_emit {
                return Err(err("content-unsafe-emit", "`safe`/`raw` on tainted loop iterable"));
            }
            bind_target(&f.target, iter_t.taint, env);
            // body must be context-loop-invariant (CON-4904(3))
            let entry = state.clone();
            walk_body(&f.body, state, env, escape_html)?;
            if *state != entry {
                return Err(err(
                    "content-context-indeterminate",
                    "`for` body is not HTML-context-loop-invariant",
                ));
            }
            walk_body(&f.else_body, state, env, escape_html)?;
        }
        ast::Stmt::IfCond(c) => {
            let cond_t = analyse_expr(&c.expr, env)?;
            if cond_t.unsafe_emit {
                return Err(err("content-unsafe-emit", "`safe`/`raw` on tainted condition"));
            }
            // branches must merge to the same context (CON-4904(3))
            let entry = state.clone();
            let mut true_state = entry.clone();
            walk_body(&c.true_body, &mut true_state, env, escape_html)?;
            let mut false_state = entry.clone();
            walk_body(&c.false_body, &mut false_state, env, escape_html)?;
            if true_state != false_state {
                return Err(err(
                    "content-context-indeterminate",
                    "`if`/`else` branches end in different HTML contexts",
                ));
            }
            *state = true_state;
        }
        ast::Stmt::WithBlock(w) => {
            let saved = env.locals.clone();
            for (target, expr) in &w.assignments {
                let t = analyse_expr(expr, env)?;
                if t.unsafe_emit {
                    return Err(err("content-unsafe-emit", "`safe`/`raw` on tainted `with` value"));
                }
                bind_target(target, t.taint, env);
            }
            walk_body(&w.body, state, env, escape_html)?;
            env.locals = saved;
        }
        ast::Stmt::Set(s) => {
            let t = analyse_expr(&s.expr, env)?;
            if t.unsafe_emit {
                return Err(err("content-unsafe-emit", "`safe`/`raw` on tainted `set` value"));
            }
            bind_target(&s.target, t.taint, env);
        }
        ast::Stmt::SetBlock(s) => {
            // {% set x %}…{% endset %} (optionally filtered). If the captured body or the
            // filter carries taint into a non-text place we cannot model, fail closed.
            if let Some(filter) = &s.filter {
                let ft = analyse_expr(filter, env)?;
                if ft.unsafe_emit || ft.taint != Taint::None {
                    return Err(err(
                        "content-template-unanalyzable",
                        "filtered capture block over tainted data",
                    ));
                }
            }
            // The captured block becomes tainted if its body interpolates taint.
            let mut probe = HtmlState::new();
            let taint = body_capture_taint(&s.body, &mut probe, env)?;
            bind_target(&s.target, taint, env);
        }
        ast::Stmt::AutoEscape(a) => {
            // The enabled expr must be a const bool; non-Html over taint is unsafe.
            let enabled = const_bool(&a.enabled);
            match enabled {
                Some(true) => walk_body(&a.body, state, env, true)?,
                Some(false) => walk_body(&a.body, state, env, false)?,
                None => {
                    // Dynamic autoescape (e.g. `{% autoescape props.esc %}`): we cannot
                    // prove Html-escaping is on, and a content author could turn it off at
                    // render — so analyse the body as if escaping is DISABLED (fail
                    // closed). Any tainted scalar/url emit inside then trips
                    // content-context-unsafe via check_emit's escape_html guard (Codex P1).
                    walk_body(&a.body, state, env, false)?;
                }
            }
        }
        ast::Stmt::FilterBlock(f) => {
            // A filter applied to a body: `safe`/`raw` over taint → unsafe-emit; any
            // other taint through a filter we cannot model → unanalyzable.
            let mut probe = HtmlState::new();
            let taint = body_capture_taint(&f.body, &mut probe, env)?;
            if taint != Taint::None {
                if filter_is_raw(&f.filter) {
                    return Err(err("content-unsafe-emit", "`safe`/`raw` filter block over tainted body"));
                }
                return Err(err(
                    "content-template-unanalyzable",
                    "filter block over tainted body",
                ));
            }
            // no taint: treat the whole block as opaque text; poison context to be safe
            state.poisoned = true;
        }
        ast::Stmt::Macro(_m) => {
            // A content-invocable template that defines a local macro is outside the
            // CON-4904 analyzable subset: soundly tracking arg→param taint into the
            // macro's internal interpolation sinks requires full interprocedural
            // re-walking (the macro body has its own HTML context). Rather than analyse
            // the body with parameters forced untainted — which silently misses a tainted
            // argument reaching a JS/URL/CSS sink inside the macro (BUG-1, confirmed XSS)
            // — fail closed. Content components are leaf widgets; inline instead.
            return Err(err(
                "content-template-unanalyzable",
                "content-invocable template defines a `{% macro %}` (outside the CON-4904 analyzable subset)",
            ));
        }
        ast::Stmt::CallBlock(c) => {
            // {% call sub(...) %}body{% endcall %}. If any argument or the body carries
            // taint, we cannot resolve the callee's landing context here → fail closed.
            for arg in &c.call.args {
                if call_arg_taint(arg, env)? != Taint::None {
                    return Err(err(
                        "content-template-unanalyzable",
                        "tainted argument passed to a sub-component `call`",
                    ));
                }
            }
            let mut probe = HtmlState::new();
            let body_taint = body_capture_taint(&c.macro_decl.body, &mut probe, env)?;
            if body_taint != Taint::None {
                return Err(err(
                    "content-template-unanalyzable",
                    "tainted data in a `call` block body lands in an unresolved context",
                ));
            }
        }
        ast::Stmt::Do(d) => {
            for arg in &d.call.args {
                if call_arg_taint(arg, env)? != Taint::None {
                    return Err(err(
                        "content-template-unanalyzable",
                        "tainted argument in a `do` call",
                    ));
                }
            }
        }
        // ---- constructs outside the analyzable subset (CON-4904(5)) → fail closed ----
        // These variants exist because the pinned minijinja enables `multi_template`.
        // If the feature set changes, this match stops compiling (NFR-4903(b/c)).
        ast::Stmt::Block(_)
        | ast::Stmt::Import(_)
        | ast::Stmt::FromImport(_)
        | ast::Stmt::Extends(_)
        | ast::Stmt::Include(_) => {
            return Err(err(
                "content-template-unanalyzable",
                "content-invocable template uses template inheritance/import/include (outside the CON-4904 analyzable subset)",
            ));
        }
    }
    Ok(())
}

fn walk_body(
    body: &[ast::Stmt],
    state: &mut HtmlState,
    env: &mut Env,
    escape_html: bool,
) -> CResult<()> {
    for stmt in body {
        walk_stmt(stmt, state, env, escape_html)?;
    }
    Ok(())
}

/// Probe a captured/sub body for the strongest taint reaching an emit, without affecting
/// the outer HTML state. Used for set-block / filter / call-block bodies.
fn body_capture_taint(body: &[ast::Stmt], probe: &mut HtmlState, env: &mut Env) -> CResult<Taint> {
    let mut taint = Taint::None;
    for stmt in body {
        taint = taint.join(stmt_capture_taint(stmt, probe, env)?);
    }
    Ok(taint)
}

fn stmt_capture_taint(stmt: &ast::Stmt, probe: &mut HtmlState, env: &mut Env) -> CResult<Taint> {
    match stmt {
        ast::Stmt::EmitExpr(e) => {
            let et = analyse_expr(&e.expr, env)?;
            if et.unsafe_emit {
                return Err(err("content-unsafe-emit", "`safe`/`raw` on tainted captured value"));
            }
            Ok(et.taint)
        }
        ast::Stmt::EmitRaw(r) => {
            probe.feed(r.raw);
            Ok(Taint::None)
        }
        ast::Stmt::Template(t) => body_capture_taint(&t.children, probe, env),
        ast::Stmt::IfCond(c) => {
            let a = body_capture_taint(&c.true_body, probe, env)?;
            let b = body_capture_taint(&c.false_body, probe, env)?;
            Ok(a.join(b))
        }
        ast::Stmt::ForLoop(f) => body_capture_taint(&f.body, probe, env),
        ast::Stmt::WithBlock(w) => body_capture_taint(&w.body, probe, env),
        // anything else inside a capture with potential taint → be conservative
        ast::Stmt::Set(s) => analyse_expr(&s.expr, env).map(|t| t.taint),
        ast::Stmt::SetBlock(s) => body_capture_taint(&s.body, probe, env),
        ast::Stmt::AutoEscape(a) => body_capture_taint(&a.body, probe, env),
        ast::Stmt::FilterBlock(f) => body_capture_taint(&f.body, probe, env),
        ast::Stmt::Do(_) => Ok(Taint::None),
        ast::Stmt::Macro(_) => Ok(Taint::None),
        ast::Stmt::CallBlock(c) => body_capture_taint(&c.macro_decl.body, probe, env),
        ast::Stmt::Block(_)
        | ast::Stmt::Import(_)
        | ast::Stmt::FromImport(_)
        | ast::Stmt::Extends(_)
        | ast::Stmt::Include(_) => Err(err(
            "content-template-unanalyzable",
            "inheritance/import inside a captured body",
        )),
    }
}

/// Check a tainted emit against the current HTML context (CON-4904(2)).
fn check_emit(et: ExprTaint, state: &HtmlState, escape_html: bool) -> CResult<()> {
    if et.unsafe_emit {
        return Err(err(
            "content-unsafe-emit",
            "a content value reaches an HTML emit via `safe`/`raw` (escaper bypass)",
        ));
    }
    if et.taint == Taint::None {
        return Ok(());
    }
    // (0) autoescape must be Html over a tainted scalar/url emit; the slot is pre-escaped
    // SafeHtml so it does not rely on autoescape, but is still TEXT-only.
    if !escape_html && matches!(et.taint, Taint::Scalar | Taint::Url) {
        return Err(err(
            "content-context-unsafe",
            "a content prop is emitted in a non-Html-autoescaped region (CON-4904(0))",
        ));
    }
    let ctx = state.context();
    let allowed = match (et.taint, ctx) {
        // scalar: TEXT or double-quoted attribute
        (Taint::Scalar, Ctx::Text | Ctx::AttrValueDq) => true,
        // url: additionally a double-quoted URL attribute
        (Taint::Url, Ctx::Text | Ctx::AttrValueDq | Ctx::UrlAttrDq) => true,
        // slot (SafeHtml): TEXT only
        (Taint::Slot, Ctx::Text) => true,
        _ => false,
    };
    if allowed {
        return Ok(());
    }
    // classify the rejection
    let code = match ctx {
        Ctx::Unanalyzable => "content-template-unanalyzable",
        Ctx::Indeterminate => "content-context-indeterminate",
        _ => "content-context-unsafe",
    };
    Err(err(
        code,
        format!(
            "content {:?} value reaches {:?} context (not in its safe-context set)",
            et.taint, ctx
        ),
    ))
}

/// Analyse an expression's taint and escaper-bypass status.
fn analyse_expr(expr: &ast::Expr, env: &Env) -> CResult<ExprTaint> {
    match expr {
        ast::Expr::Var(v) => {
            // Bare use of the `props`/`caller` capability identifiers — i.e. NOT the
            // recognised `props.field` / `props["field"]` / `caller()` patterns (those
            // are handled in GetAttr/GetItem/Call before recursing here) — cannot be
            // tracked per-field through an alias, so a tainted value could be laundered
            // (`{% set p = props %}{{ p['msg'] }}`). Fail closed (BUG-7).
            if v.id == "props" || v.id == "caller" {
                return Err(err(
                    "content-template-unanalyzable",
                    format!("bare use of `{}` cannot be statically tracked", v.id),
                ));
            }
            Ok(ExprTaint {
                taint: env.locals.get(v.id).copied().unwrap_or(Taint::None),
                unsafe_emit: false,
            })
        }
        ast::Expr::Const(_) => Ok(ExprTaint::none()),
        ast::Expr::GetAttr(g) => {
            // props.<name>
            if let ast::Expr::Var(v) = &g.expr {
                if v.id == "props" {
                    return Ok(ExprTaint { taint: env.prop_taint(g.name), unsafe_emit: false });
                }
            }
            analyse_expr(&g.expr, env)
        }
        ast::Expr::GetItem(g) => {
            // props["name"] with a const string subscript
            if let ast::Expr::Var(v) = &g.expr {
                if v.id == "props" {
                    if let ast::Expr::Const(c) = &g.subscript_expr {
                        if let Some(name) = c.value.as_str() {
                            return Ok(ExprTaint {
                                taint: env.prop_taint(name),
                                unsafe_emit: false,
                            });
                        }
                    }
                    // dynamic subscript into props → conservatively tainted scalar
                    return Ok(ExprTaint { taint: Taint::Scalar, unsafe_emit: false });
                }
            }
            let base = analyse_expr(&g.expr, env)?;
            Ok(base)
        }
        ast::Expr::Filter(f) => {
            let inner = match &f.expr {
                Some(e) => analyse_expr(e, env)?,
                None => ExprTaint::none(),
            };
            let mut args_taint = inner.taint;
            for a in &f.args {
                args_taint = args_taint.join(call_arg_taint(a, env)?);
            }
            let bypass = matches!(f.name, "safe" | "raw") && args_taint != Taint::None;
            Ok(ExprTaint {
                taint: args_taint,
                unsafe_emit: inner.unsafe_emit || bypass,
            })
        }
        ast::Expr::BinOp(b) => {
            let l = analyse_expr(&b.left, env)?;
            let r = analyse_expr(&b.right, env)?;
            Ok(ExprTaint {
                taint: l.taint.join(r.taint),
                unsafe_emit: l.unsafe_emit || r.unsafe_emit,
            })
        }
        ast::Expr::UnaryOp(u) => analyse_expr(&u.expr, env),
        ast::Expr::IfExpr(i) => {
            let t = analyse_expr(&i.true_expr, env)?;
            let f = match &i.false_expr {
                Some(e) => analyse_expr(e, env)?,
                None => ExprTaint::none(),
            };
            // condition taint does not flow to the value, but a bypass in it still counts
            let cond = analyse_expr(&i.test_expr, env)?;
            Ok(ExprTaint {
                taint: t.taint.join(f.taint),
                unsafe_emit: t.unsafe_emit || f.unsafe_emit || cond.unsafe_emit,
            })
        }
        ast::Expr::Slice(s) => analyse_expr(&s.expr, env),
        ast::Expr::Test(t) => analyse_expr(&t.expr, env),
        ast::Expr::Call(c) => {
            // caller() → the slot (SafeHtml)
            if let ast::Expr::Var(v) = &c.expr {
                if v.id == "caller" {
                    return Ok(ExprTaint { taint: Taint::Slot, unsafe_emit: false });
                }
            }
            // Any other call: if a content-tainted value flows in as an argument or the
            // receiver, it enters a body the lint does not re-walk (e.g. a local macro
            // whose internal interpolation is the real sink). We cannot prove the
            // callee's interior context, so fail closed (BUG-1 — the macro arg→param
            // taint hole). A call with no tainted inputs yields an untainted result.
            let mut arg_taint = Taint::None;
            for a in &c.args {
                arg_taint = arg_taint.join(call_arg_taint(a, env)?);
            }
            let recv = analyse_expr(&c.expr, env)?;
            if arg_taint != Taint::None || recv.taint != Taint::None {
                return Err(err(
                    "content-template-unanalyzable",
                    "a content-tainted value is passed into a call whose body the lint cannot analyse",
                ));
            }
            Ok(ExprTaint { taint: Taint::None, unsafe_emit: recv.unsafe_emit })
        }
        ast::Expr::List(l) => {
            let mut taint = Taint::None;
            let mut bypass = false;
            for item in &l.items {
                let t = analyse_expr(item, env)?;
                taint = taint.join(t.taint);
                bypass |= t.unsafe_emit;
            }
            Ok(ExprTaint { taint, unsafe_emit: bypass })
        }
        ast::Expr::Map(m) => {
            let mut taint = Taint::None;
            let mut bypass = false;
            for e in m.keys.iter().chain(m.values.iter()) {
                let t = analyse_expr(e, env)?;
                taint = taint.join(t.taint);
                bypass |= t.unsafe_emit;
            }
            Ok(ExprTaint { taint, unsafe_emit: bypass })
        }
    }
}

fn call_arg_taint(arg: &ast::CallArg, env: &Env) -> CResult<Taint> {
    let e = match arg {
        ast::CallArg::Pos(e)
        | ast::CallArg::Kwarg(_, e)
        | ast::CallArg::PosSplat(e)
        | ast::CallArg::KwargSplat(e) => e,
    };
    Ok(analyse_expr(e, env)?.taint)
}

/// Bind an assignment target to a taint kind. Only simple `Var` targets are tracked;
/// anything else is ignored (its later use, if tainted, is caught at the emit).
fn bind_target(target: &ast::Expr, taint: Taint, env: &mut Env) {
    if let ast::Expr::Var(v) = target {
        env.locals.insert(v.id.to_string(), taint);
    }
}

fn const_bool(expr: &ast::Expr) -> Option<bool> {
    if let ast::Expr::Const(c) = expr {
        return c.value.as_str().is_none().then(|| c.value.is_true());
    }
    None
}

fn filter_is_raw(filter: &ast::Expr) -> bool {
    if let ast::Expr::Filter(f) = filter {
        return matches!(f.name, "safe" | "raw");
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::components::manifest::parse_manifest;

    fn manifest() -> Manifest {
        parse_manifest(
            r#"
            name = "c"
            content_invocable = true
            content_props = ["msg", "link", "n"]
            [props]
            msg = { type = "string", default = "" }
            link = { type = "url", required = false }
            n = { type = "int", default = 0 }
        "#,
            "c",
        )
        .unwrap()
    }

    fn lint(tmpl: &str) -> CResult<()> {
        lint_component(tmpl, &manifest())
    }

    #[test]
    fn scalar_in_text_ok() {
        assert!(lint("<p>{{ props.msg }}</p>").is_ok());
    }

    #[test]
    fn scalar_in_dq_attr_ok() {
        assert!(lint(r#"<div title="{{ props.msg }}">x</div>"#).is_ok());
    }

    #[test]
    fn scalar_in_sq_attr_unsafe() {
        let e = lint(r#"<div title='{{ props.msg }}'>x</div>"#).unwrap_err();
        assert_eq!(e.code, "content-context-unsafe");
    }

    #[test]
    fn scalar_in_style_attr_css_unsafe() {
        let e = lint(r#"<div style="color:{{ props.msg }}">x</div>"#).unwrap_err();
        assert_eq!(e.code, "content-context-unsafe");
    }

    // ---- adversarial-review regressions (BUG-1/4/7) ----

    #[test]
    fn bug1_macro_definition_rejected() {
        // A local macro whose body sinks a param into a JS context would launder taint;
        // defining a macro at all is now fail-closed.
        let e = lint(r#"{% macro emit(x) %}<script>var a={{ x }}</script>{% endmacro %}<p>{{ props.msg }}</p>"#)
            .unwrap_err();
        assert_eq!(e.code, "content-template-unanalyzable");
    }

    #[test]
    fn bug1_tainted_arg_to_call_rejected() {
        // Passing a content prop into any non-`caller` call hides the real sink.
        let e = lint(r#"<p>{{ emit(props.msg) }}</p>"#).unwrap_err();
        assert_eq!(e.code, "content-template-unanalyzable");
    }

    #[test]
    fn bug7_props_alias_laundering_rejected() {
        // `{% set p = props %}{{ p['msg'] }}` must not launder a tainted prop into href.
        let e = lint(r#"{% set p = props %}<a href="{{ p['msg'] }}">y</a>"#).unwrap_err();
        assert_eq!(e.code, "content-template-unanalyzable");
    }

    #[test]
    fn bug4_text_after_script_is_text_context() {
        // After </script> the tokeniser must return to TEXT; a scalar prop there is safe.
        assert!(lint(r#"<script>var x=1;</script><p>{{ props.msg }}</p>"#).is_ok());
        // and a scalar after </style> is likewise TEXT, not CSS
        assert!(lint(r#"<style>.a{color:red}</style><p>{{ props.msg }}</p>"#).is_ok());
    }

    #[test]
    fn scalar_in_script_js_unsafe() {
        let e = lint(r#"<script>var x = {{ props.msg }}</script>"#).unwrap_err();
        assert_eq!(e.code, "content-context-unsafe");
    }

    #[test]
    fn scalar_in_srcdoc_unsafe() {
        // srcdoc is parsed as an HTML document (entity-decoded), so autoescape doesn't
        // neutralise markup there (Codex P1).
        let e = lint(r#"<iframe srcdoc="{{ props.msg }}"></iframe>"#).unwrap_err();
        assert_eq!(e.code, "content-context-unsafe");
    }

    #[test]
    fn scalar_in_onclick_js_unsafe() {
        let e = lint(r#"<button onclick="{{ props.msg }}">x</button>"#).unwrap_err();
        assert_eq!(e.code, "content-context-unsafe");
    }

    #[test]
    fn scalar_in_unquoted_attr_unsafe() {
        let e = lint(r#"<div data-x={{ props.msg }}>x</div>"#).unwrap_err();
        assert_eq!(e.code, "content-context-unsafe");
    }

    #[test]
    fn non_url_prop_in_href_unsafe() {
        let e = lint(r#"<a href="{{ props.msg }}">x</a>"#).unwrap_err();
        assert_eq!(e.code, "content-context-unsafe");
    }

    #[test]
    fn url_prop_in_href_ok() {
        assert!(lint(r#"<a href="{{ props.link }}">x</a>"#).is_ok());
    }

    #[test]
    fn url_prop_in_text_ok() {
        assert!(lint(r#"<p>{{ props.link }}</p>"#).is_ok());
    }

    #[test]
    fn slot_in_text_ok() {
        assert!(lint(r#"<div>{{ caller() }}</div>"#).is_ok());
    }

    #[test]
    fn slot_in_attr_unsafe() {
        let e = lint(r#"<a href="{{ caller() }}">x</a>"#).unwrap_err();
        assert_eq!(e.code, "content-context-unsafe");
    }

    #[test]
    fn safe_filter_on_tainted_is_unsafe_emit() {
        let e = lint(r#"<p>{{ props.msg | safe }}</p>"#).unwrap_err();
        assert_eq!(e.code, "content-unsafe-emit");
    }

    #[test]
    fn set_propagates_taint() {
        let e = lint(r#"{% set u = props.msg %}<a href="{{ u }}">x</a>"#).unwrap_err();
        assert_eq!(e.code, "content-context-unsafe");
    }

    #[test]
    fn concat_with_tainted_is_tainted() {
        let e = lint(r#"<div style="x:{{ "a" ~ props.msg }}">y</div>"#).unwrap_err();
        assert_eq!(e.code, "content-context-unsafe");
    }

    #[test]
    fn untainted_in_any_context_ok() {
        // a non-content var (not in content_props) is untainted → fine in JS/CSS/attr
        assert!(lint(r#"<script>var x = {{ other }}</script>"#).is_ok());
    }

    #[test]
    fn autoescape_false_over_taint_unsafe() {
        let e = lint(r#"{% autoescape false %}<p>{{ props.msg }}</p>{% endautoescape %}"#)
            .unwrap_err();
        assert_eq!(e.code, "content-context-unsafe");
    }

    #[test]
    fn dynamic_autoescape_over_taint_unsafe() {
        // A non-const autoescape expr could be turned off at render; the body is analysed
        // as if escaping is disabled, so a tainted scalar in text is rejected (Codex P1).
        let e = lint(r#"{% autoescape props.n %}<p>{{ props.msg }}</p>{% endautoescape %}"#)
            .unwrap_err();
        assert_eq!(e.code, "content-context-unsafe");
    }

    #[test]
    fn whitespace_trim_marker_rejected() {
        let e = lint("<p>{{- props.msg }}</p>").unwrap_err();
        assert_eq!(e.code, "content-template-unanalyzable");
    }

    #[test]
    fn extends_rejected() {
        let e = lint(r#"{% extends "base.html" %}"#).unwrap_err();
        assert_eq!(e.code, "content-template-unanalyzable");
    }

    #[test]
    fn if_branches_consistent_context_ok() {
        assert!(lint(r#"<p>{% if props.n %}{{ props.msg }}{% else %}none{% endif %}</p>"#).is_ok());
    }

    #[test]
    fn tainted_arg_to_subcall_unanalyzable() {
        let e = lint(r#"{% call sub(x=props.msg) %}{% endcall %}"#).unwrap_err();
        assert_eq!(e.code, "content-template-unanalyzable");
    }
}
