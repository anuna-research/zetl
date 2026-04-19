//! Shared golden-HTML harness for canonical extensions (SPEC-032 CON-3212).
//!
//! ## Fixture format
//!
//! One directory per fixture under a harness root (conventionally
//! `tests/extension-fixtures/`). Per CON-3212:
//!
//! ```text
//! <root>/<name>/
//!   input.md           # sample page exercising the extension's syntax
//!   expected.html      # expected post-stage HTML fragment
//!   selector-match.txt # (optional) vault filenames the selector matches
//!   vault/             # (optional) small vault used to validate
//!                      # selector-match.txt when the extension ships one
//! ```
//!
//! `expected.html` is compared byte-for-byte after normalisation (collapse
//! whitespace between tags, sort attribute order within a tag). The
//! normaliser is deliberately narrow: it handles the formatting drift that
//! would otherwise need each extension-runner to emit pretty-printed output,
//! and nothing more. Semantic equivalence (e.g. `<b>` vs `<strong>`) is not
//! in scope — that is the runner's job.
//!
//! ## Running and updating
//!
//! The harness is pure: it reads fixtures and renders diffs. Callers supply
//! the runner — typically, the integration test in
//! `tests/ext_golden_html_integration.rs` walks the fixture tree via
//! [`load_fixtures`] and feeds each input through the stage pipeline the
//! extension declares, then calls [`compare`] to gate the result.
//!
//! Regenerating expecteds is done through the workspace xtask:
//!
//! ```shell
//! cargo xtask update-golden               # regenerate all fixtures
//! cargo xtask update-golden baseline-md   # single fixture
//! ```
//!
//! The xtask reuses the same runner registry the test harness uses, so the
//! "golden" output and the "checked" output come from one source — there is
//! no way to update expecteds to a value the gate would not then accept.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// A discovered extension fixture rooted at [`Fixture::dir`].
///
/// The invariant is that `dir.join("input.md")` exists; everything else on
/// the struct is accessor sugar for files the runner may or may not
/// produce. Callers that want to know whether a particular optional file
/// was present should check the accessor's `Option` return.
#[derive(Debug, Clone)]
pub struct Fixture {
    /// Fixture directory name, e.g. `"baseline-markdown"`. Used in
    /// diagnostics and as the selector for `xtask update-golden <name>`.
    pub name: String,
    /// Absolute or repo-relative path to the fixture directory.
    pub dir: PathBuf,
}

impl Fixture {
    /// Absolute path to `input.md` — asserted present by [`load_fixtures`].
    pub fn input_md_path(&self) -> PathBuf {
        self.dir.join("input.md")
    }

    /// Absolute path to `expected.html`.
    pub fn expected_html_path(&self) -> PathBuf {
        self.dir.join("expected.html")
    }

    /// Absolute path to the optional `selector-match.txt`.
    pub fn selector_match_path(&self) -> PathBuf {
        self.dir.join("selector-match.txt")
    }

    /// Absolute path to the optional `vault/` directory.
    pub fn vault_dir(&self) -> PathBuf {
        self.dir.join("vault")
    }

    /// Read `input.md` and return its contents. Errors propagate the raw
    /// [`io::Error`] so callers get useful context from
    /// [`io::Error::kind`].
    pub fn read_input(&self) -> io::Result<String> {
        fs::read_to_string(self.input_md_path())
    }

    /// Read `expected.html` if it exists. Returns `Ok(None)` when the file
    /// is absent — a new fixture that hasn't had its golden written yet
    /// falls into this bucket and the comparator treats it as drift.
    pub fn read_expected(&self) -> io::Result<Option<String>> {
        match fs::read_to_string(self.expected_html_path()) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Read and parse `selector-match.txt`. Lines are trimmed; blank lines
    /// and `#`-prefixed comment lines are discarded. The result is **not**
    /// sorted — callers that want order-independence should sort before
    /// comparing.
    pub fn read_selector_match(&self) -> io::Result<Option<Vec<String>>> {
        match fs::read_to_string(self.selector_match_path()) {
            Ok(s) => Ok(Some(parse_selector_match(&s))),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Overwrite `expected.html` with `html`. Used by
    /// `cargo xtask update-golden` to re-seed the gate from the runner's
    /// current output. The write is unconditional — callers that want a
    /// "no-op when unchanged" semantic should diff first.
    pub fn write_expected(&self, html: &str) -> io::Result<()> {
        fs::write(self.expected_html_path(), html)
    }
}

fn parse_selector_match(raw: &str) -> Vec<String> {
    raw.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Walk `root` for subdirectories that contain an `input.md`. Sorts the
/// result by directory name so diagnostics and test-output ordering stay
/// stable across platforms and filesystems.
///
/// Non-directories (stray READMEs, `.keep` files) are ignored silently —
/// the harness is a convention-over-configuration layer and should not
/// trip on auxiliary files a fixture author drops in the root.
pub fn load_fixtures(root: &Path) -> io::Result<Vec<Fixture>> {
    let mut out = Vec::new();
    if !root.exists() {
        return Ok(out);
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if !path.join("input.md").is_file() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string)
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        out.push(Fixture { name, dir: path });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Outcome of comparing an expected HTML fragment to a runner's actual
/// output, pre-computed with a human-readable unified diff so callers that
/// assert on `!matched` have something useful to print.
#[derive(Debug, Clone)]
pub struct Comparison {
    /// `true` iff the normalised `expected` equals the normalised `actual`.
    pub matched: bool,
    /// Normalised expected text the comparator used. Cached so `Display`
    /// consumers don't need to re-normalise.
    pub expected_normalised: String,
    /// Normalised actual text the comparator used.
    pub actual_normalised: String,
    /// Unified diff of normalised expected ↔ actual. Empty when `matched`.
    pub diff: String,
}

/// Normalise then diff-compare two HTML fragments.
///
/// The normaliser collapses all runs of ASCII whitespace outside of tag
/// bodies to a single space, strips leading/trailing whitespace on each
/// line, and sorts attributes within every tag so reorderings don't trip
/// the gate. This is exactly the post-normalisation specified in
/// TEST-3212a/b/c.
pub fn compare(expected: &str, actual: &str) -> Comparison {
    let expected_n = normalise_html(expected);
    let actual_n = normalise_html(actual);
    let matched = expected_n == actual_n;
    let diff = if matched {
        String::new()
    } else {
        render_diff("expected", &expected_n, "actual", &actual_n)
    };
    Comparison {
        matched,
        expected_normalised: expected_n,
        actual_normalised: actual_n,
        diff,
    }
}

/// Canonicalise an HTML fragment for byte-for-byte comparison.
///
/// Operations, in order:
///
/// 1. Sort attributes within every opening tag alphabetically by name so
///    `<a class="x" id="y">` and `<a id="y" class="x">` compare equal.
/// 2. Collapse every run of ASCII whitespace outside `<pre>` / `<code>`
///    bodies to a single space. Newlines are not semantically
///    significant in HTML between block elements, so `"<p>A</p>\n<p>B</p>"`
///    and `"<p>A</p> <p>B</p>"` normalise equal.
/// 3. Insert linebreaks between adjacent tags so the diff output stays
///    one element per line. This is cosmetic — it doesn't affect the
///    comparison, only its presentation.
/// 4. Trim leading/trailing whitespace off the result.
///
/// `<pre>` / `<code>` preservation matters because their whitespace is
/// significant — collapsing it would make two genuinely different
/// code-block fixtures compare equal.
pub fn normalise_html(html: &str) -> String {
    let sorted = sort_attributes(html);
    let collapsed = collapse_whitespace_outside_preformatted(&sorted);
    // Break after a `>` that is followed by a `<` so tag runs render one
    // per line in the diff. The pure-whitespace gap between them was
    // collapsed to a single space in step 2 and is dropped here.
    let mut out = String::with_capacity(collapsed.len());
    let bytes = collapsed.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        out.push(b as char);
        if b == b'>' {
            // Look past a single optional space inserted by collapse and
            // decide whether the next non-space byte is a `<`; if so,
            // replace the gap with a newline.
            let mut j = i + 1;
            if j < bytes.len() && bytes[j] == b' ' {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'<' {
                out.push('\n');
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out.trim().to_string()
}

fn sort_attributes(html: &str) -> String {
    let bytes = html.as_bytes();
    let mut out = String::with_capacity(html.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let Some(close) = find_tag_end(bytes, i) else {
                out.push(bytes[i] as char);
                i += 1;
                continue;
            };
            let tag = &html[i..=close];
            out.push_str(&sort_attributes_in_tag(tag));
            i = close + 1;
        } else {
            // Byte-wise copy keeps UTF-8 intact because we only branch on
            // ASCII `<`.
            let end = bytes[i..]
                .iter()
                .position(|&b| b == b'<')
                .map(|p| i + p)
                .unwrap_or(bytes.len());
            out.push_str(&html[i..end]);
            i = end;
        }
    }
    out
}

/// Locate the byte index of `>` that closes the opening `<` at `start`.
///
/// Honours quoted attribute values so a `>` inside `class="a>b"` doesn't
/// trick us into closing early.
fn find_tag_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start + 1;
    let mut quote: Option<u8> = None;
    while i < bytes.len() {
        let b = bytes[i];
        match (quote, b) {
            (Some(q), c) if c == q => quote = None,
            (None, b'"') | (None, b'\'') => quote = Some(b),
            (None, b'>') => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

fn sort_attributes_in_tag(tag: &str) -> String {
    // Preserve comments, CDATA, and closing tags verbatim — none of them
    // have attributes worth sorting and blindly tokenising them would
    // corrupt the output.
    if tag.starts_with("<!--") || tag.starts_with("<![") || tag.starts_with("</") {
        return tag.to_string();
    }
    let inner = &tag[1..tag.len() - 1];
    let (name, rest) = split_tag_name(inner);
    // Self-closing slash ("<br/>" variants) sticks to the end of the tag
    // after sorting; we peel it off here so sort_attrs doesn't treat it as
    // a bareword attribute named "/".
    let (rest, self_close) = if let Some(stripped) = rest.trim_end().strip_suffix('/') {
        (stripped, true)
    } else {
        (rest, false)
    };
    let attrs = parse_attributes(rest);
    if attrs.is_empty() {
        // No-attribute tags still need the self-close normalised — leave
        // whitespace as the caller emitted it otherwise.
        return if self_close {
            format!("<{name}/>")
        } else {
            format!("<{name}>")
        };
    }
    let mut sorted = attrs;
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let attr_str = sorted
        .into_iter()
        .map(|(k, v)| match v {
            Some(v) => format!("{k}={v}"),
            None => k,
        })
        .collect::<Vec<_>>()
        .join(" ");
    if self_close {
        format!("<{name} {attr_str}/>")
    } else {
        format!("<{name} {attr_str}>")
    }
}

fn split_tag_name(inner: &str) -> (String, &str) {
    let end = inner
        .find(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/')
        .unwrap_or(inner.len());
    (inner[..end].to_string(), &inner[end..])
}

/// Parse `key=value` / `key='value'` / `key="value"` / bareword pairs out
/// of a tag-attribute byte run. Values keep their surrounding quotes so
/// the sorter's output round-trips verbatim.
fn parse_attributes(src: &str) -> Vec<(String, Option<String>)> {
    let mut out: Vec<(String, Option<String>)> = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let key_start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'=' {
            i += 1;
        }
        let key = src[key_start..i].to_string();
        if key.is_empty() {
            break;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            out.push((key, None));
            continue;
        }
        i += 1; // consume '='
        if i >= bytes.len() {
            out.push((key, Some(String::new())));
            break;
        }
        let quote = bytes[i];
        if quote == b'"' || quote == b'\'' {
            let value_start = i;
            i += 1;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
            if i < bytes.len() {
                i += 1; // consume closing quote
            }
            out.push((key, Some(src[value_start..i].to_string())));
        } else {
            let value_start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            out.push((key, Some(src[value_start..i].to_string())));
        }
    }
    out
}

fn collapse_whitespace_outside_preformatted(html: &str) -> String {
    let bytes = html.as_bytes();
    let mut out = String::with_capacity(html.len());
    let mut i = 0;
    while i < bytes.len() {
        // Detect a pre / code opening and copy its body verbatim through
        // the matching closer. Nested `<pre>` is not a concern in our
        // fixtures — if it ever becomes one, this loop can be upgraded to
        // a tag stack.
        if starts_with_tag(bytes, i, b"<pre") || starts_with_tag(bytes, i, b"<code") {
            let tag_name: &[u8] = if starts_with_tag(bytes, i, b"<pre") {
                b"</pre>"
            } else {
                b"</code>"
            };
            if let Some(close) = find_close_tag(bytes, i, tag_name) {
                out.push_str(&html[i..close + tag_name.len()]);
                i = close + tag_name.len();
                continue;
            }
        }
        if bytes[i].is_ascii_whitespace() {
            // Collapse every run of ASCII whitespace — newlines
            // included — to a single space. Block vs inline boundaries
            // are irrelevant to the comparison; the post-pass in
            // [`normalise_html`] re-inserts newlines between adjacent
            // tags purely for diff readability.
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            out.push(' ');
            continue;
        }
        // Non-space byte: preserve until the next whitespace or tag break.
        let end = bytes[i..]
            .iter()
            .position(|b| b.is_ascii_whitespace())
            .map(|p| i + p)
            .unwrap_or(bytes.len());
        out.push_str(&html[i..end]);
        i = end;
    }
    out
}

fn starts_with_tag(bytes: &[u8], at: usize, tag: &[u8]) -> bool {
    if !bytes[at..].starts_with(tag) {
        return false;
    }
    // Next byte must be whitespace, `>` or `/` so `<prefix>` variants
    // like `<prefixed>` don't match `<pre`.
    matches!(bytes.get(at + tag.len()), Some(b) if b.is_ascii_whitespace() || *b == b'>' || *b == b'/')
}

fn find_close_tag(bytes: &[u8], from: usize, close: &[u8]) -> Option<usize> {
    let mut i = from;
    while i + close.len() <= bytes.len() {
        if bytes[i..].starts_with(close) {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Render a minimal unified diff between two multi-line strings.
///
/// Output format:
///
/// ```text
/// --- <left_label>
/// +++ <right_label>
/// - <line-only-in-left>
/// + <line-only-in-right>
///   <context line>
/// ```
///
/// We deliberately don't emit `@@` hunk headers — the fragments this
/// harness compares are small enough that full-file diffs are easier to
/// read than minimal hunks, and a pure-dep algorithm keeps the harness
/// free of an LCS crate.
pub fn render_diff(left_label: &str, left: &str, right_label: &str, right: &str) -> String {
    let left_lines: Vec<&str> = left.lines().collect();
    let right_lines: Vec<&str> = right.lines().collect();
    let script = edit_script(&left_lines, &right_lines);
    let mut out = String::new();
    out.push_str(&format!("--- {left_label}\n"));
    out.push_str(&format!("+++ {right_label}\n"));
    for edit in script {
        match edit {
            Edit::Same(line) => {
                out.push_str("  ");
                out.push_str(line);
                out.push('\n');
            }
            Edit::Delete(line) => {
                out.push_str("- ");
                out.push_str(line);
                out.push('\n');
            }
            Edit::Insert(line) => {
                out.push_str("+ ");
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    out
}

#[derive(Debug)]
enum Edit<'a> {
    Same(&'a str),
    Delete(&'a str),
    Insert(&'a str),
}

/// Quadratic Myers-style LCS. Bounded by `O(|a| * |b|)` time + memory,
/// which is fine for HTML fragments under a few hundred lines — all our
/// fixtures live well below that. Anything pathological should trip the
/// integration test's size-cap sanity check long before this becomes the
/// bottleneck.
fn edit_script<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<Edit<'a>> {
    let n = a.len();
    let m = b.len();
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for (i, line_a) in a.iter().enumerate() {
        for (j, line_b) in b.iter().enumerate() {
            dp[i + 1][j + 1] = if line_a == line_b {
                dp[i][j] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut out = Vec::new();
    let mut i = n;
    let mut j = m;
    while i > 0 && j > 0 {
        if a[i - 1] == b[j - 1] {
            out.push(Edit::Same(a[i - 1]));
            i -= 1;
            j -= 1;
        } else if dp[i - 1][j] >= dp[i][j - 1] {
            out.push(Edit::Delete(a[i - 1]));
            i -= 1;
        } else {
            out.push(Edit::Insert(b[j - 1]));
            j -= 1;
        }
    }
    while i > 0 {
        out.push(Edit::Delete(a[i - 1]));
        i -= 1;
    }
    while j > 0 {
        out.push(Edit::Insert(b[j - 1]));
        j -= 1;
    }
    out.reverse();
    out
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn load_fixtures_skips_dirs_without_input_md() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("a")).unwrap();
        fs::write(root.join("a/input.md"), "hi").unwrap();
        fs::create_dir_all(root.join("b")).unwrap(); // no input.md
        fs::write(root.join("README.md"), "ignore").unwrap();
        let fixtures = load_fixtures(root).unwrap();
        assert_eq!(fixtures.len(), 1);
        assert_eq!(fixtures[0].name, "a");
    }

    #[test]
    fn load_fixtures_sorts_by_name() {
        let tmp = TempDir::new().unwrap();
        for name in &["zeta", "alpha", "mid"] {
            fs::create_dir_all(tmp.path().join(name)).unwrap();
            fs::write(tmp.path().join(name).join("input.md"), "").unwrap();
        }
        let names: Vec<_> = load_fixtures(tmp.path())
            .unwrap()
            .into_iter()
            .map(|f| f.name)
            .collect();
        assert_eq!(names, vec!["alpha", "mid", "zeta"]);
    }

    #[test]
    fn read_expected_returns_none_when_missing() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("input.md"), "").unwrap();
        let fx = Fixture {
            name: "x".into(),
            dir: tmp.path().to_path_buf(),
        };
        assert!(fx.read_expected().unwrap().is_none());
    }

    #[test]
    fn selector_match_strips_comments_and_blanks() {
        let raw = "\n# comment\nfoo.md\n\nbar/baz.md\n  # indented comment\n";
        assert_eq!(
            parse_selector_match(raw),
            vec!["foo.md".to_string(), "bar/baz.md".to_string()]
        );
    }

    #[test]
    fn normalise_sorts_attributes() {
        let a = "<a class=\"x\" id=\"y\">Hi</a>";
        let b = "<a id=\"y\" class=\"x\">Hi</a>";
        assert_eq!(normalise_html(a), normalise_html(b));
    }

    #[test]
    fn normalise_collapses_whitespace_outside_pre() {
        let a = "<p>one   two\n three</p>";
        let b = "<p>one two three</p>";
        // The collapse step leaves the `\n` as a line boundary, which then
        // gets trimmed; result is a single line either way.
        assert_eq!(normalise_html(a), normalise_html(b));
    }

    #[test]
    fn normalise_preserves_pre_body_whitespace() {
        let a = "<pre>  two\n  spaces</pre>";
        let b = "<pre>two spaces</pre>";
        assert_ne!(normalise_html(a), normalise_html(b));
    }

    #[test]
    fn normalise_preserves_code_block_body() {
        let a = "<code>  a  </code>";
        let b = "<code>a</code>";
        assert_ne!(normalise_html(a), normalise_html(b));
    }

    #[test]
    fn compare_matched_returns_empty_diff() {
        let c = compare("<p>hi</p>", "<p>hi</p>");
        assert!(c.matched);
        assert!(c.diff.is_empty());
    }

    #[test]
    fn compare_mismatched_produces_diff() {
        let c = compare("<p>one</p>\n<p>two</p>", "<p>one</p>\n<p>three</p>");
        assert!(!c.matched);
        assert!(c.diff.contains("- <p>two</p>"));
        assert!(c.diff.contains("+ <p>three</p>"));
    }

    #[test]
    fn write_expected_is_round_trippable() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("input.md"), "").unwrap();
        let fx = Fixture {
            name: "x".into(),
            dir: tmp.path().to_path_buf(),
        };
        fx.write_expected("<p>out</p>\n").unwrap();
        assert_eq!(fx.read_expected().unwrap(), Some("<p>out</p>\n".into()));
    }

    #[test]
    fn sort_attributes_ignores_closing_tags() {
        assert_eq!(sort_attributes("</a>"), "</a>");
        assert_eq!(sort_attributes("<!-- hi -->"), "<!-- hi -->");
    }

    #[test]
    fn sort_attributes_self_closing() {
        let n = normalise_html("<br id=\"b\" class=\"a\"/>");
        assert_eq!(n, "<br class=\"a\" id=\"b\"/>");
    }
}
