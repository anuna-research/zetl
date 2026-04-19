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
        _ => None,
    }
}

/// Canary runner: pulldown-cmark with the options `render_to_html` uses,
/// minus wikilink rewriting. Deliberately *not* re-exported from
/// `zetl::web::markdown` because that path also runs the line-anchor
/// injector and the wikilink regex, neither of which belongs in a plain
/// Markdown fixture.
pub fn baseline_markdown_runner(input: &str) -> String {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_GFM;
    let parser = Parser::new_ext(input, options);
    let mut out = String::new();
    html::push_html(&mut out, parser);
    out
}
