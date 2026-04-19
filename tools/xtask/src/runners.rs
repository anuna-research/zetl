//! Runner registry shared between `cargo xtask update-golden` and the
//! integration test `tests/ext_golden_html_integration.rs`.
//!
//! Each registered runner takes the fixture's `input.md` and returns the
//! HTML fragment the gate will compare against `expected.html`. The
//! contract is pure-fn — no IO, no side effects — so the xtask's writer
//! path and the test's asserter path are guaranteed to produce bit-for-bit
//! identical output.
//!
//! ## Adding a new fixture
//!
//! 1. Create `tests/extension-fixtures/<name>/input.md`.
//! 2. Add an arm in [`resolve`] mapping `<name>` to the appropriate runner.
//! 3. Run `cargo xtask update-golden <name>` to seed `expected.html`.
//! 4. Commit `input.md` + `expected.html` together.
//!
//! Canonical-extension fixtures that ship a real hook binary should route
//! through a runner that spawns the hook via `PersistentHook` and threads
//! `run_page` — follow the `callouts` / `tasks` / `admonition` plan items.

use pulldown_cmark::{html, Options, Parser};

pub type Runner = fn(&str) -> String;

/// Resolve a fixture name to its runner. Returns `None` when the name is
/// unknown — both the xtask and the test surface this as an error so a
/// fixture author sees a clear "no runner registered" diagnostic.
pub fn resolve(name: &str) -> Option<Runner> {
    match name {
        "baseline-markdown" => Some(baseline_markdown_runner),
        "admonition" => Some(admonition_runner),
        "tasks" => Some(tasks_runner),
        _ => None,
    }
}

/// Canary runner: pulldown-cmark with the options `render_to_html` uses,
/// minus wikilink rewriting. Deliberately *not* re-exported from
/// `zetl::web::markdown` because that path also runs the line-anchor
/// injector and the wikilink regex, neither of which belongs in a plain
/// Markdown fixture.
pub fn baseline_markdown_runner(input: &str) -> String {
    let parser = Parser::new_ext(input, gfm_options());
    let mut out = String::new();
    html::push_html(&mut out, parser);
    out
}

fn gfm_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_GFM
}

fn render_markdown(md: &str) -> String {
    let parser = Parser::new_ext(md, gfm_options());
    let mut out = String::new();
    html::push_html(&mut out, parser);
    out
}

/// Thin in-process stub for the canonical `admonition` extension
/// (SPEC-032 REQ-3212, TEST-3212c). Recognises:
///
/// - **Obsidian `ad-*` fenced blocks.** `` ```ad-<type> `` opens a block;
///   a leading `title: <text>` line sets the title, subsequent body lines
///   render as Markdown. Closed by the usual `` ``` ``.
/// - **Python-Markdown MkDocs admonitions.** `!!! <type> "<title>"` opens
///   the block; the body is lines indented by four spaces (or a tab).
///   Blank lines inside the block are preserved; the first non-indented
///   non-blank line ends it.
///
/// Output shape matches the `admonition` convention shared by
/// `mdbook-admonish` and Python-Markdown's `admonition` extension:
///
/// ```html
/// <div class="admonition <type>">
/// <p class="admonition-title"><Title></p>
/// <Markdown-rendered body>
/// </div>
/// ```
///
/// The default title is the type-name with its first letter uppercased
/// (`note` → `Note`). Real vaults run the transformation through an
/// ecosystem plugin; the stub only exists so the golden-HTML gate can
/// prove the theme CSS + template contract without pulling an external
/// binary into CI.
pub fn admonition_runner(input: &str) -> String {
    let lines: Vec<&str> = input.lines().collect();
    let mut out = String::new();
    let mut plain: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some(fence) = parse_obsidian_fence_open(line) {
            flush_plain(&mut plain, &mut out);
            let (next, title, body) = read_obsidian_block(&lines, i + 1, fence.inline_title);
            emit_admonition(&mut out, &fence.type_name, title.as_deref(), &body);
            i = next;
            continue;
        }
        if let Some((type_name, title)) = parse_pymd_open(line) {
            flush_plain(&mut plain, &mut out);
            let (next, body) = read_pymd_body(&lines, i + 1);
            emit_admonition(&mut out, &type_name, title.as_deref(), &body);
            i = next;
            continue;
        }
        plain.push(line);
        i += 1;
    }
    flush_plain(&mut plain, &mut out);
    out
}

fn flush_plain(plain: &mut Vec<&str>, out: &mut String) {
    if plain.is_empty() {
        return;
    }
    let joined = plain.join("\n");
    plain.clear();
    if joined.trim().is_empty() {
        return;
    }
    out.push_str(&render_markdown(&joined));
}

struct ObsidianFence {
    type_name: String,
    inline_title: Option<String>,
}

fn parse_obsidian_fence_open(line: &str) -> Option<ObsidianFence> {
    let rest = line.trim_end().strip_prefix("```ad-")?;
    let (type_name, tail) = match rest.find(char::is_whitespace) {
        Some(pos) => (&rest[..pos], Some(rest[pos..].trim_start())),
        None => (rest, None),
    };
    if !is_admonition_type(type_name) {
        return None;
    }
    let inline_title = tail.and_then(|t| {
        t.strip_prefix("title:")
            .map(|v| v.trim().to_string())
            .filter(|s| !s.is_empty())
    });
    Some(ObsidianFence {
        type_name: type_name.to_string(),
        inline_title,
    })
}

fn read_obsidian_block(
    lines: &[&str],
    start: usize,
    inline_title: Option<String>,
) -> (usize, Option<String>, Vec<String>) {
    let mut title = inline_title;
    let mut j = start;
    // Consume the Obsidian header (title / collapse / icon / color + blank
    // separators) before the body proper. Stops at the first line that is
    // not a recognised k/v, not blank, and not the closing fence.
    while j < lines.len() {
        let ln = lines[j];
        if is_fence_close(ln) {
            break;
        }
        if title.is_none() {
            if let Some(v) = ln.strip_prefix("title:") {
                title = Some(v.trim().to_string());
                j += 1;
                continue;
            }
        }
        if is_obsidian_header_kv(ln) {
            j += 1;
            continue;
        }
        if ln.trim().is_empty() {
            j += 1;
            continue;
        }
        break;
    }
    let mut body: Vec<String> = Vec::new();
    while j < lines.len() {
        let ln = lines[j];
        if is_fence_close(ln) {
            j += 1;
            break;
        }
        body.push(ln.to_string());
        j += 1;
    }
    trim_trailing_blank(&mut body);
    (j, title, body)
}

fn is_fence_close(line: &str) -> bool {
    line.trim_start() == "```"
}

fn is_obsidian_header_kv(line: &str) -> bool {
    match line.split_once(':') {
        Some((k, _)) => matches!(k.trim(), "collapse" | "icon" | "color"),
        None => false,
    }
}

fn parse_pymd_open(line: &str) -> Option<(String, Option<String>)> {
    // `!!! type` or `!!! type "Title"` / `!!! type Title`. Reject
    // indented openings so `!!! foo` inside another admonition's body
    // stays treated as body text.
    if line.starts_with(' ') || line.starts_with('\t') {
        return None;
    }
    let rest = line.strip_prefix("!!!")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim_start();
    let (type_name, tail) = match rest.find(char::is_whitespace) {
        Some(pos) => (&rest[..pos], Some(rest[pos..].trim())),
        None => (rest, None),
    };
    if !is_admonition_type(type_name) {
        return None;
    }
    let title = tail.and_then(|t| {
        if t.is_empty() {
            None
        } else if let Some(inner) = t.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
            Some(inner.to_string())
        } else {
            Some(t.to_string())
        }
    });
    Some((type_name.to_string(), title))
}

fn read_pymd_body(lines: &[&str], start: usize) -> (usize, Vec<String>) {
    let mut body: Vec<String> = Vec::new();
    let mut j = start;
    while j < lines.len() {
        let ln = lines[j];
        if ln.trim().is_empty() {
            body.push(String::new());
            j += 1;
            continue;
        }
        if let Some(stripped) = strip_pymd_indent(ln) {
            body.push(stripped.to_string());
            j += 1;
            continue;
        }
        break;
    }
    trim_trailing_blank(&mut body);
    (j, body)
}

fn strip_pymd_indent(line: &str) -> Option<&str> {
    if let Some(s) = line.strip_prefix("    ") {
        return Some(s);
    }
    line.strip_prefix('\t')
}

fn trim_trailing_blank(body: &mut Vec<String>) {
    while body.last().map(|s| s.trim().is_empty()).unwrap_or(false) {
        body.pop();
    }
}

fn is_admonition_type(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn emit_admonition(out: &mut String, type_name: &str, title: Option<&str>, body: &[String]) {
    let default_title = capitalise_ascii(type_name);
    let title_text = title.unwrap_or(&default_title);
    out.push_str(&format!("<div class=\"admonition {type_name}\">\n"));
    out.push_str(&format!("<p class=\"admonition-title\">{title_text}</p>\n"));
    let body_md = body.join("\n");
    if !body_md.is_empty() {
        out.push_str(&render_markdown(&body_md));
    }
    out.push_str("</div>\n");
}

fn capitalise_ascii(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// Thin in-process stub for the canonical `tasks` extension
/// (SPEC-032 REQ-3212). Recognises Obsidian task-list syntax:
///
/// ```markdown
/// - [ ] Open task
/// - [x] Completed task @due(2026-04-25)
/// ```
///
/// A contiguous run of lines matching `- [ ] ` / `- [x] ` / `- [X] ` is
/// one task list. The optional inline marker `@due(YYYY-MM-DD)` surfaces
/// a due date; the stub strips it from the rendered label and emits a
/// `<time>` element alongside.
///
/// Output shape mirrors what a real theme would render after reading the
/// template vars a persistent hook would emit (`page.ext.tasks.open`,
/// `.done`, `.due`):
///
/// ```html
/// <ul class="tasks">
/// <li class="task task-open"><input disabled type="checkbox"> Open task</li>
/// <li class="task task-done"><input checked disabled type="checkbox"> Completed task <time class="task-due" datetime="2026-04-25">2026-04-25</time></li>
/// </ul>
/// ```
///
/// Real vaults run the transformation through an ecosystem plugin
/// (Obsidian Tasks subset); the stub only exists so the golden-HTML gate
/// can prove the theme CSS + template contract without pulling an
/// external binary into CI.
pub fn tasks_runner(input: &str) -> String {
    let lines: Vec<&str> = input.lines().collect();
    let mut out = String::new();
    let mut plain: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if parse_task_line(lines[i]).is_some() {
            flush_plain(&mut plain, &mut out);
            let (next, items) = read_task_block(&lines, i);
            emit_task_list(&mut out, &items);
            i = next;
            continue;
        }
        plain.push(lines[i]);
        i += 1;
    }
    flush_plain(&mut plain, &mut out);
    out
}

struct TaskItem {
    done: bool,
    text: String,
    due: Option<String>,
}

fn parse_task_line(line: &str) -> Option<(bool, &str)> {
    let body = line
        .strip_prefix("- [ ] ")
        .map(|b| (false, b))
        .or_else(|| {
            line.strip_prefix("- [x] ")
                .or_else(|| line.strip_prefix("- [X] "))
                .map(|b| (true, b))
        })?;
    Some(body)
}

fn read_task_block(lines: &[&str], start: usize) -> (usize, Vec<TaskItem>) {
    let mut items: Vec<TaskItem> = Vec::new();
    let mut j = start;
    while j < lines.len() {
        let Some((done, body)) = parse_task_line(lines[j]) else {
            break;
        };
        let (text, due) = extract_due(body);
        items.push(TaskItem { done, text, due });
        j += 1;
    }
    (j, items)
}

fn extract_due(body: &str) -> (String, Option<String>) {
    let Some(at) = body.find("@due(") else {
        return (body.trim().to_string(), None);
    };
    let rest = &body[at + 5..];
    let Some(end) = rest.find(')') else {
        return (body.trim().to_string(), None);
    };
    let date = &rest[..end];
    if !is_iso_date(date) {
        return (body.trim().to_string(), None);
    }
    let before = body[..at].trim_end();
    let after = body[at + 5 + end + 1..].trim_start();
    let mut text = String::new();
    text.push_str(before);
    if !before.is_empty() && !after.is_empty() {
        text.push(' ');
    }
    text.push_str(after);
    (text.trim().to_string(), Some(date.to_string()))
}

fn is_iso_date(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[0..4].iter().all(|b| b.is_ascii_digit())
        && bytes[5..7].iter().all(|b| b.is_ascii_digit())
        && bytes[8..10].iter().all(|b| b.is_ascii_digit())
}

fn emit_task_list(out: &mut String, items: &[TaskItem]) {
    out.push_str("<ul class=\"tasks\">\n");
    for it in items {
        let class = if it.done {
            "task task-done"
        } else {
            "task task-open"
        };
        let checkbox = if it.done {
            "<input checked disabled type=\"checkbox\">"
        } else {
            "<input disabled type=\"checkbox\">"
        };
        out.push_str(&format!("<li class=\"{class}\">{checkbox} "));
        out.push_str(&render_inline(&it.text));
        if let Some(due) = &it.due {
            out.push_str(&format!(
                " <time class=\"task-due\" datetime=\"{due}\">{due}</time>"
            ));
        }
        out.push_str("</li>\n");
    }
    out.push_str("</ul>\n");
}

/// Render inline markdown (bold, emphasis, code, links, etc.) without the
/// surrounding `<p>…</p>` pulldown-cmark would otherwise wrap around a
/// single block. Used for the visible label of each task item.
fn render_inline(text: &str) -> String {
    let rendered = render_markdown(text);
    let trimmed = rendered.trim_end_matches('\n');
    match trimmed
        .strip_prefix("<p>")
        .and_then(|s| s.strip_suffix("</p>"))
    {
        Some(inner) => inner.to_string(),
        None => trimmed.to_string(),
    }
}
