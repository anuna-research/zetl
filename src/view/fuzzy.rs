//! Fuzzy page-not-found selection prompt (REQ-073).
//!
//! When the user supplies a `<page>` argument that does not match any indexed
//! page title exactly, this module displays the top-5 SimHash-nearest titles
//! and waits for a single-key selection before entering the TUI.

use std::io::Write as _;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};

use crate::simhash::SimHashIndex;

/// Present a numbered fuzzy-suggestion prompt on stdout.
///
/// Builds a [`SimHashIndex`] over `pages`, ranks them by Hamming distance from
/// `query`, shows up to 5 suggestions, then reads a single keypress:
///
/// - `1`–`5`  → returns `Ok(Some(page_title))` for the chosen suggestion.
/// - `q` / `Esc` → returns `Ok(None)` (caller should `exit(0)`).
///
/// The prompt format follows REQ-073:
/// ```text
/// 1. Title One  2. Title Two  3. Title Three
/// Select [1-3] or q to quit:
/// ```
///
/// Raw mode is enabled only during the keypress read and is always restored
/// before returning, so the caller need not manage terminal state.
pub fn fuzzy_suggestion_prompt(query: &str, pages: &[(String, String)]) -> Result<Option<String>> {
    if pages.is_empty() {
        return Ok(None);
    }

    // Find top-5 closest page titles.  Threshold=64 means "all distances",
    // so we always get the nearest results even for very different strings.
    let index = SimHashIndex::build(pages);
    let suggestions = index.search(query, 64, 5);

    if suggestions.is_empty() {
        return Ok(None);
    }

    let count = suggestions.len();

    // Print numbered titles on a single line: "1. Title  2. Title  ..."
    let list: String = suggestions
        .iter()
        .enumerate()
        .map(|(i, s)| format!("{}. {}", i + 1, s.page))
        .collect::<Vec<_>>()
        .join("  ");

    println!("{list}");
    print!("Select [1-{count}] or q to quit: ");
    std::io::stdout().flush()?;

    // Enter raw mode to read a single keypress (handles Esc, q, 1–5).
    enable_raw_mode()?;
    let choice = loop {
        match event::read() {
            Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break None,
                KeyCode::Char(c) if c.is_ascii_digit() => {
                    let n = (c as usize).wrapping_sub('0' as usize);
                    if n >= 1 && n <= count {
                        break Some(suggestions[n - 1].page.clone());
                    }
                }
                _ => {}
            },
            Err(_) => break None,
            _ => {}
        }
    };
    disable_raw_mode()?;

    // Echo a newline so the shell prompt appears on a fresh line.
    // When the user chose 'q'/Esc we still want a clean terminal, but REQ-073
    // says "exit 0 with no output" — the newline here is invisible to callers.
    println!();

    Ok(choice)
}
