//! SPEC-049 CON-4901 — content-directive recogniser.
//!
//! Recognises the three generic-directive forms (after the remark-directive /
//! CommonMark generic-directive prior art, ADR-4904) in untrusted content Markdown:
//!
//! - **container** `:::name{attrs}` … body … `:::` (block, with a Markdown body),
//! - **leaf** `::name{attrs}` (block, no body),
//! - **inline** `:name[label]{attrs}` (in running text).
//!
//! Recognition is **fail-closed** (REQ-4901): a malformed directive is *not* a
//! directive — it is left as inert text, never emitted as raw HTML. Recognition runs in
//! the CommonMark block phase for `:::`/`::` and the inline phase for `:name[…]`, and
//! **never inside a fenced/indented code block or an inline code span** (CON-4901
//! recognition phase): directive syntax shown in a code sample is literal text.
//!
//! The recogniser produces a [`Node`] **tree** — container bodies hold their own text
//! plus already-recognised nested [`Directive`] children — so the transform stage
//! (REQ-4906) can expand bottom-up, innermost-first, sanitising each level's *own*
//! untrusted text in isolation. This module is the pure recogniser: no manifest, no
//! resolution, no rendering.

/// Which of the three CON-4901 forms a directive took.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectiveForm {
    Container,
    Leaf,
    Inline,
}

/// One recognised attribute from an `{attrs}` block (CON-4901). Order is preserved for
/// determinism (NFR-4901) and for `.class` accumulation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attr {
    /// `#id` shorthand — maps to a declared `id` prop (REQ-4902).
    Id(String),
    /// `.class` shorthand — appended to a declared `class` prop (REQ-4902).
    Class(String),
    /// `key=value` or `key="quoted value"`.
    KeyValue { key: String, value: String },
    /// A bare `key` flag — boolean true.
    Flag(String),
}

/// A recognised content directive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Directive {
    pub form: DirectiveForm,
    pub name: String,
    pub attrs: Vec<Attr>,
    /// Inline form only: the `[label]` text.
    pub label: Option<String>,
    /// Container form only: the body as a node tree (own text + nested directives).
    /// Empty for leaf / inline.
    pub children: Vec<Node>,
    /// 1-based source line of the directive's opening, for author diagnostics (REQ-4911).
    pub line: usize,
}

/// A node in the recognised content tree: literal Markdown text, or a directive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// Verbatim Markdown source (rendered + sanitised in isolation by the transform
    /// stage, REQ-4906).
    Text(String),
    Directive(Directive),
}

/// A directive `name`/attr `key` is a kebab identifier: `lower { lower|digit|- } (lower|digit)`
/// with no leading digit, no trailing `-`. A single lowercase letter is the minimum.
pub fn is_valid_directive_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    match bytes.first() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    if bytes.len() == 1 {
        return true;
    }
    // last char must be alnum (no trailing '-')
    let last = *bytes.last().unwrap();
    if !(last.is_ascii_lowercase() || last.is_ascii_digit()) {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

/// Error from attribute-block recognition; the caller treats any error as
/// "not a directive" (fail closed → inert text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectiveSyntaxError(pub String);

/// Parse the inner text of an `{attrs}` block (the part between `{` and `}`).
///
/// Grammar (CON-4901): whitespace-separated `#id` | `.class` | `key=value` |
/// `key="quoted"` | `flag`. A `bare` value admits `alnum -_.:/` (no spaces, quotes, or
/// `<`/`>`); a `quoted` value is single-line (a newline inside the quote is malformed).
/// The bare grammar deliberately admits `:`/`/` so a `javascript:` string can be
/// *carried* — scheme safety is REQ-4904's `url`-ptype ingestion job, never the grammar.
pub fn parse_attr_block(inner: &str) -> Result<Vec<Attr>, DirectiveSyntaxError> {
    let mut attrs = Vec::new();
    let mut chars = inner.char_indices().peekable();

    while let Some(&(_, c)) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        match c {
            '#' => {
                chars.next();
                let id = take_bare(&mut chars);
                if id.is_empty() {
                    return Err(DirectiveSyntaxError("empty #id".into()));
                }
                attrs.push(Attr::Id(id));
            }
            '.' => {
                chars.next();
                let class = take_bare(&mut chars);
                if class.is_empty() {
                    return Err(DirectiveSyntaxError("empty .class".into()));
                }
                attrs.push(Attr::Class(class));
            }
            _ => {
                // key, then optional `=value`
                let key = take_key(&mut chars);
                if key.is_empty() {
                    return Err(DirectiveSyntaxError(format!("unexpected char `{c}` in attrs")));
                }
                if !is_valid_directive_name(&key) {
                    return Err(DirectiveSyntaxError(format!("invalid attr key `{key}`")));
                }
                match chars.peek() {
                    Some(&(_, '=')) => {
                        chars.next(); // consume '='
                        let value = match chars.peek() {
                            Some(&(_, '"')) => {
                                chars.next(); // consume opening quote
                                take_quoted(&mut chars)?
                            }
                            _ => {
                                let v = take_bare(&mut chars);
                                if v.is_empty() {
                                    return Err(DirectiveSyntaxError(format!(
                                        "empty value for `{key}`"
                                    )));
                                }
                                v
                            }
                        };
                        attrs.push(Attr::KeyValue { key, value });
                    }
                    _ => attrs.push(Attr::Flag(key)),
                }
            }
        }
    }
    Ok(attrs)
}

type CharIter<'a> = std::iter::Peekable<std::str::CharIndices<'a>>;

/// A kebab key: `lower { lower|digit|- }` (greedy; validity checked by caller).
fn take_key(chars: &mut CharIter) -> String {
    let mut s = String::new();
    while let Some(&(_, c)) = chars.peek() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' {
            s.push(c);
            chars.next();
        } else {
            break;
        }
    }
    s
}

/// A bare value: `{ alnum | - | _ | . | : | / }`. Stops at whitespace, `}`, `"`, `<`,
/// `>`, or anything else.
fn take_bare(chars: &mut CharIter) -> String {
    let mut s = String::new();
    while let Some(&(_, c)) = chars.peek() {
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '/') {
            s.push(c);
            chars.next();
        } else {
            break;
        }
    }
    s
}

/// A quoted value: characters up to the closing `"`, with `\"` and `\\` escapes. A
/// newline before the closing quote is malformed (CON-4901: single-line).
fn take_quoted(chars: &mut CharIter) -> Result<String, DirectiveSyntaxError> {
    let mut s = String::new();
    while let Some((_, c)) = chars.next() {
        match c {
            '"' => return Ok(s),
            '\n' | '\r' => {
                return Err(DirectiveSyntaxError("newline inside quoted value".into()))
            }
            '\\' => match chars.next() {
                Some((_, '"')) => s.push('"'),
                Some((_, '\\')) => s.push('\\'),
                Some((_, other)) => {
                    s.push('\\');
                    s.push(other);
                }
                None => return Err(DirectiveSyntaxError("trailing backslash in quote".into())),
            },
            other => s.push(other),
        }
    }
    Err(DirectiveSyntaxError("unterminated quoted value".into()))
}

// ---------------------------------------------------------------------------
// Document scanner — block phase (`:::`/`::`) + inline phase (`:name[…]`), with
// fenced-code-block and inline-code-span exclusion (CON-4901 recognition phase).
// ---------------------------------------------------------------------------

/// Recognise every directive in a content-Markdown document, returning a node tree.
/// Text outside directives is preserved verbatim. Malformed directive openers are left
/// as text (fail closed). Container nesting follows the remark-directive colon-count
/// rule: a closing fence needs at least as many colons as its opener, so a `::::`
/// container may hold a `:::` one.
pub fn scan(src: &str) -> Vec<Node> {
    let lines: Vec<&str> = src.split_inclusive('\n').collect();
    let mut parser = Scanner {
        lines: &lines,
        idx: 0,
    };
    parser.parse_block(0, None)
}

struct Scanner<'a> {
    lines: &'a [&'a str],
    idx: usize,
}

impl<'a> Scanner<'a> {
    /// Parse block-level nodes until EOF or (if `closing` is set) a container fence with
    /// `≥ closing` colons is hit. `min_colons` is the fence size of the enclosing
    /// container (0 at top level), used only to size nested closers.
    fn parse_block(&mut self, _min_colons: usize, closing: Option<usize>) -> Vec<Node> {
        let mut nodes: Vec<Node> = Vec::new();
        let mut text_buf = String::new();

        while self.idx < self.lines.len() {
            let line = self.lines[self.idx];
            let trimmed = line.trim_start();

            // Fenced code block: ``` or ~~~ — copy through verbatim, no directive
            // recognition inside (CON-4901).
            if let Some(fence) = code_fence_marker(trimmed) {
                self.consume_code_fence(line, fence, &mut text_buf);
                continue;
            }

            // Closing fence for an enclosing container?
            if let Some(min_close) = closing {
                if let Some(n) = colon_fence_only(trimmed) {
                    if n >= min_close {
                        self.idx += 1; // consume the closing fence
                        flush_text(&mut nodes, &mut text_buf);
                        return nodes;
                    }
                }
            }

            // Container opener `:::name{attrs}` (3+ colons, name follows).
            if let Some((colons, dir_line)) = self.try_container_opener(line) {
                flush_text(&mut nodes, &mut text_buf);
                self.idx += 1; // consume opener line
                let mut dir = dir_line;
                dir.children = self.parse_block(colons, Some(colons));
                nodes.push(Node::Directive(dir));
                continue;
            }

            // Leaf `::name{attrs}` (exactly 2 colons opener, whole line).
            if let Some(dir) = self.try_leaf(line) {
                flush_text(&mut nodes, &mut text_buf);
                self.idx += 1;
                nodes.push(Node::Directive(dir));
                continue;
            }

            // Otherwise a normal text line, appended verbatim. The inline form
            // (`:name[label]{attrs}`) is DEFERRED in v1 (CON-4901 Q2): expanding it
            // requires inline-level Markdown rendering within a paragraph, which the
            // block-segment expansion model cannot do without splitting the paragraph
            // into separate <p> blocks. Until that lands, `:name[…]` stays literal text.
            self.idx += 1;
            text_buf.push_str(line);
        }

        flush_text(&mut nodes, &mut text_buf);
        nodes
    }

    /// Consume a fenced code block verbatim into the text buffer (no recognition).
    fn consume_code_fence(&mut self, opener: &str, fence: (char, usize), buf: &mut String) {
        buf.push_str(opener);
        self.idx += 1;
        while self.idx < self.lines.len() {
            let line = self.lines[self.idx];
            buf.push_str(line);
            self.idx += 1;
            let trimmed = line.trim_start();
            if let Some(closing) = code_fence_marker(trimmed) {
                if closing.0 == fence.0 && closing.1 >= fence.1 {
                    break;
                }
            }
        }
    }

    /// Try to recognise the current line as a container opener `:::name{attrs}`.
    fn try_container_opener(&self, line: &str) -> Option<(usize, Directive)> {
        let trimmed = line.trim_start();
        let colons = leading_colons(trimmed);
        if colons < 3 {
            return None;
        }
        let rest = &trimmed[colons..];
        let (name, attrs, label, tail) = parse_opener_tail(rest)?;
        // A container opener must have nothing but whitespace after `{attrs}` (the body
        // begins on the next line). An inline form would carry `[label]`.
        if label.is_some() || !tail.trim().is_empty() {
            return None;
        }
        Some((
            colons,
            Directive {
                form: DirectiveForm::Container,
                name,
                attrs,
                label: None,
                children: Vec::new(),
                line: self.idx + 1,
            },
        ))
    }

    /// Try to recognise the current line as a leaf `::name{attrs}` (exactly 2 colons,
    /// whole line, no body).
    fn try_leaf(&self, line: &str) -> Option<Directive> {
        let trimmed = line.trim_start();
        if leading_colons(trimmed) != 2 {
            return None;
        }
        let rest = &trimmed[2..];
        let (name, attrs, label, tail) = parse_opener_tail(rest)?;
        if label.is_some() || !tail.trim().is_empty() {
            return None;
        }
        Some(Directive {
            form: DirectiveForm::Leaf,
            name,
            attrs,
            label: None,
            children: Vec::new(),
            line: self.idx + 1,
        })
    }
}

/// Count leading `:` characters.
fn leading_colons(s: &str) -> usize {
    s.bytes().take_while(|&b| b == b':').count()
}

/// If `s` (trimmed) is *only* a run of `:` (a bare closing fence), return the count.
fn colon_fence_only(s: &str) -> Option<usize> {
    let n = leading_colons(s);
    if n >= 3 && s[n..].trim().is_empty() {
        Some(n)
    } else {
        None
    }
}

/// A ``` ` ``` or `~` code-fence marker: returns (char, run-length ≥ 3).
fn code_fence_marker(trimmed: &str) -> Option<(char, usize)> {
    for ch in ['`', '~'] {
        let n = trimmed.chars().take_while(|&c| c == ch).count();
        if n >= 3 {
            return Some((ch, n));
        }
    }
    None
}

/// Parse the part of an opener after the colons: `name [ "[" label "]" ] [ "{" attrs "}" ]`
/// plus the trailing remainder. Returns `(name, attrs, label, tail)` or `None` if the
/// name is invalid (→ not a directive).
fn parse_opener_tail(rest: &str) -> Option<(String, Vec<Attr>, Option<String>, String)> {
    // name
    let name_end = rest
        .find(|c: char| c == '[' || c == '{' || c.is_whitespace())
        .unwrap_or(rest.len());
    let name = &rest[..name_end];
    if !is_valid_directive_name(name) {
        return None;
    }
    let mut cursor = &rest[name_end..];

    // optional [label]
    let mut label = None;
    if let Some(stripped) = cursor.strip_prefix('[') {
        let close = stripped.find(']')?;
        label = Some(stripped[..close].to_string());
        cursor = &stripped[close + 1..];
    }

    // optional {attrs}
    let mut attrs = Vec::new();
    if let Some(stripped) = cursor.strip_prefix('{') {
        let close = stripped.find('}')?;
        let inner = &stripped[..close];
        attrs = parse_attr_block(inner).ok()?;
        cursor = &stripped[close + 1..];
    }

    Some((name.to_string(), attrs, label, cursor.to_string()))
}

fn flush_text(nodes: &mut Vec<Node>, buf: &mut String) {
    if !buf.is_empty() {
        nodes.push(Node::Text(std::mem::take(buf)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(nodes: &[Node]) -> Vec<&Directive> {
        let mut out = Vec::new();
        for n in nodes {
            if let Node::Directive(d) = n {
                out.push(d);
            }
        }
        out
    }

    #[test]
    fn name_validation() {
        assert!(is_valid_directive_name("callout"));
        assert!(is_valid_directive_name("a"));
        assert!(is_valid_directive_name("nav-header"));
        assert!(is_valid_directive_name("a1"));
        assert!(!is_valid_directive_name("nav-"));
        assert!(!is_valid_directive_name("1nav"));
        assert!(!is_valid_directive_name("Nav"));
        assert!(!is_valid_directive_name(""));
    }

    #[test]
    fn parses_container() {
        let src = ":::callout{type=warning title=\"Heads up\"}\nbody **md**\n:::\n";
        let nodes = scan(src);
        let ds = dirs(&nodes);
        assert_eq!(ds.len(), 1);
        let d = ds[0];
        assert_eq!(d.form, DirectiveForm::Container);
        assert_eq!(d.name, "callout");
        assert_eq!(
            d.attrs,
            vec![
                Attr::KeyValue { key: "type".into(), value: "warning".into() },
                Attr::KeyValue { key: "title".into(), value: "Heads up".into() },
            ]
        );
        // body text recognised as a Text child
        assert!(matches!(d.children.first(), Some(Node::Text(t)) if t.contains("body **md**")));
    }

    #[test]
    fn parses_leaf() {
        let src = "before\n::hr{}\nafter\n";
        let ds = scan(src);
        let only = dirs(&ds);
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].form, DirectiveForm::Leaf);
        assert_eq!(only[0].name, "hr");
    }

    #[test]
    fn inline_form_is_deferred_to_literal_text() {
        // Inline directives are DEFERRED in v1 (Q2): they stay literal text so they cannot
        // break a paragraph into separate <p> blocks. Container/leaf are unaffected.
        let src = "see :abbr[HTML]{title=\"HyperText\"} here\n";
        let nodes = scan(src);
        assert!(dirs(&nodes).is_empty(), "inline directive must not be recognised in v1");
        assert!(matches!(nodes.first(), Some(Node::Text(t)) if t.contains(":abbr[HTML]")));
    }

    #[test]
    fn id_and_class_shorthand() {
        let attrs = parse_attr_block("#main .a .b key=val flag").unwrap();
        assert_eq!(
            attrs,
            vec![
                Attr::Id("main".into()),
                Attr::Class("a".into()),
                Attr::Class("b".into()),
                Attr::KeyValue { key: "key".into(), value: "val".into() },
                Attr::Flag("flag".into()),
            ]
        );
    }

    #[test]
    fn bare_value_carries_uri_string() {
        // The bare grammar admits `:`/`/` — scheme safety is REQ-4904's job, not the
        // grammar. A `javascript:` string is carried fine; ingestion validation rejects
        // it later.
        let attrs = parse_attr_block("href=https://example.com/x").unwrap();
        assert_eq!(
            attrs,
            vec![Attr::KeyValue { key: "href".into(), value: "https://example.com/x".into() }]
        );
        let attrs = parse_attr_block("href=javascript:alert").unwrap();
        assert_eq!(
            attrs,
            vec![Attr::KeyValue { key: "href".into(), value: "javascript:alert".into() }]
        );
    }

    #[test]
    fn malformed_unterminated_quote_is_error() {
        assert!(parse_attr_block("title=\"oops").is_err());
    }

    #[test]
    fn newline_in_quote_rejected() {
        let mut it = "\"a\nb\"".char_indices().peekable();
        it.next(); // consume opening quote
        assert!(take_quoted(&mut it).is_err());
    }

    #[test]
    fn lt_in_bare_is_malformed() {
        // `<` is neither a bare char nor a separator → malformed (fail closed → the
        // whole directive becomes inert text; no breakout into markup).
        assert!(parse_attr_block("x=a<b").is_err());
    }

    #[test]
    fn malformed_attrs_make_opener_not_a_directive() {
        // A container opener whose attrs are malformed is not recognised as a directive.
        let src = ":::callout{x=a<b}\nbody\n:::\n";
        let nodes = scan(src);
        assert!(dirs(&nodes).is_empty());
        assert!(matches!(nodes.first(), Some(Node::Text(t)) if t.contains(":::callout")));
    }

    #[test]
    fn directive_in_code_fence_is_literal() {
        let src = "```\n:::callout{}\nx\n:::\n```\n";
        let nodes = scan(src);
        assert!(dirs(&nodes).is_empty(), "no directive recognised inside a code fence");
    }

    #[test]
    fn inline_directive_in_code_span_is_literal() {
        let src = "use `:abbr[x]{}` syntax\n";
        let nodes = scan(src);
        assert!(dirs(&nodes).is_empty());
    }

    #[test]
    fn nested_container_by_colon_count() {
        let src = "::::outer{}\nintro\n:::inner{}\ndeep\n:::\ntrailing\n::::\n";
        let nodes = scan(src);
        let ds = dirs(&nodes);
        assert_eq!(ds.len(), 1);
        let outer = ds[0];
        assert_eq!(outer.name, "outer");
        // outer body contains text + a nested inner directive + trailing text
        let inner = dirs(&outer.children);
        assert_eq!(inner.len(), 1);
        assert_eq!(inner[0].name, "inner");
        assert!(matches!(inner[0].children.first(), Some(Node::Text(t)) if t.contains("deep")));
    }

    #[test]
    fn unterminated_container_runs_to_eof() {
        // No closing fence: body extends to EOF; still recognised as a container (the
        // remark-directive behaviour). Not malformed — just open.
        let src = ":::note{}\nbody\n";
        let nodes = scan(src);
        let ds = dirs(&nodes);
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].name, "note");
    }

    #[test]
    fn invalid_name_is_not_a_directive() {
        let src = ":::Bad{}\nbody\n:::\n";
        let nodes = scan(src);
        assert!(dirs(&nodes).is_empty());
        // left as text
        assert!(matches!(nodes.first(), Some(Node::Text(t)) if t.contains(":::Bad")));
    }

    #[test]
    fn plain_text_untouched() {
        let src = "just a paragraph\nwith two lines\n";
        let nodes = scan(src);
        assert_eq!(nodes.len(), 1);
        assert!(matches!(&nodes[0], Node::Text(t) if t == src));
    }
}
