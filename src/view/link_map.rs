//! Wikilink parsing and anchor glyph injection for the main pane (REQ-064, ADR-017).
//!
//! This module provides:
//! - [`LinkEntry`] — a single document-order wikilink occurrence with its ordinal,
//!   target page title, dead-link flag, and source line number.
//! - [`LinkMap`] — the ordered list of all [`LinkEntry`] values for the current page;
//!   shared by the context pane, bridge column, and focus-mode tasks.
//! - [`parse_wikilinks_in_line`] — finds `[[wikilink]]` patterns in a single line.
//! - [`build_annotated_lines`] — converts raw Markdown lines into ratatui [`Line`]s
//!   with styled `[N]` anchor glyphs injected after each `[[wikilink]]`.

use std::collections::HashSet;

use ratatui::prelude::*;
use ratatui::style::Color;

use super::color::{ColorMode, LinkColors};

// ── LinkEntry ─────────────────────────────────────────────────────────────

/// A single wikilink occurrence tracked in document order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkEntry {
    /// 1-based ordinal assigned in document order across the whole page.
    pub ordinal: usize,
    /// Resolved page title (text inside `[[…]]`, before any `|`, `#`, or `^`).
    pub page_title: String,
    /// `true` when the target page is absent from the vault index.
    pub is_dead: bool,
    /// 0-based index into `content_lines` where this wikilink appears.
    pub line_number: usize,
}

// ── LinkMap ───────────────────────────────────────────────────────────────

/// Document-order list of all wikilink occurrences in the current page.
///
/// Shared by the context pane, bridge column, and focus-mode tasks.
pub type LinkMap = Vec<LinkEntry>;

// ── Wikilink parsing ──────────────────────────────────────────────────────

/// Parse `[[wikilink]]` occurrences in a single line.
///
/// Returns `(start, end, page_title)` for each match, where:
/// - `start` is the byte offset of the opening `[[` in `line`.
/// - `end` is the byte offset immediately after the closing `]]`.
/// - `page_title` is the normalized target page (text before `|`, `#`, or `^`).
///
/// Nested `[[` are not supported; the first `]]` after each `[[` closes the link.
pub fn parse_wikilinks_in_line(line: &str) -> Vec<(usize, usize, String)> {
    let mut results = Vec::new();
    let mut search_from = 0usize;

    while search_from < line.len() {
        // Find the next opening [[.
        match line[search_from..].find("[[") {
            None => break,
            Some(rel_open) => {
                let open_abs = search_from + rel_open;
                let inner_start = open_abs + 2;

                // Find the matching ]].
                match line[inner_start..].find("]]") {
                    None => break, // unclosed [[ — stop scanning this line
                    Some(rel_close) => {
                        let close_abs = inner_start + rel_close;
                        let inner = &line[inner_start..close_abs];
                        let page_title = normalize_page_title(inner);
                        let end = close_abs + 2; // byte offset past ]]
                        results.push((open_abs, end, page_title));
                        search_from = end;
                    }
                }
            }
        }
    }

    results
}

/// Normalize a wikilink's inner text to a plain page title.
///
/// Strips the display alias (after `|`), heading anchor (after `#`), and
/// block reference (after `^`), then trims whitespace.
fn normalize_page_title(inner: &str) -> String {
    let without_alias = inner.split('|').next().unwrap_or(inner);
    let without_heading = without_alias.split('#').next().unwrap_or(without_alias);
    let without_block = without_heading.split('^').next().unwrap_or(without_heading);
    without_block.trim().to_string()
}

// ── Annotated-line builder ────────────────────────────────────────────────

/// Build styled ratatui [`Line`]s from raw Markdown content, injecting
/// colored `[N]` anchor glyphs after each `[[wikilink]]` (REQ-064).
///
/// Also constructs and returns the [`LinkMap`] in document order.
///
/// # Arguments
/// - `content_lines` — raw lines of the note (no trailing newline).
/// - `page_set` — set of all page titles present in the vault index.
///   Wikilinks whose target is absent from this set are considered dead links.
/// - `color_mode` — terminal color capability, used to pick glyph styles.
///
/// # Rendering rules (ADR-017, NFR-025)
/// - **Color mode** (Ansi8 / Color256 / TrueColor): `[N]` rendered in the
///   link's palette color; dead-link `[N]` rendered in `Color::Red`.
/// - **No-color mode**: `[N]` rendered in default foreground for live links;
///   `![N]` (with `!` prefix) for dead links.
pub fn build_annotated_lines(
    content_lines: &[String],
    page_set: &HashSet<String>,
    color_mode: ColorMode,
) -> (Vec<Line<'static>>, LinkMap) {
    let link_colors = LinkColors::new(color_mode);
    let is_no_color = color_mode == ColorMode::NoColor;
    let mut link_map: LinkMap = Vec::new();
    let mut ordinal = 0usize;
    let mut annotated: Vec<Line<'static>> = Vec::with_capacity(content_lines.len());

    for (line_idx, line) in content_lines.iter().enumerate() {
        let wikilinks = parse_wikilinks_in_line(line);

        if wikilinks.is_empty() {
            annotated.push(Line::raw(line.clone()));
            continue;
        }

        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut prev_end = 0usize;

        for (start, end, page_title) in wikilinks {
            ordinal += 1;
            let is_dead = !page_set.contains(&page_title);

            // Plain text before this [[wikilink]].
            if start > prev_end {
                spans.push(Span::raw(line[prev_end..start].to_owned()));
            }

            // The [[wikilink]] text itself (unstyled).
            spans.push(Span::raw(line[start..end].to_owned()));

            // Anchor glyph immediately after the closing ]].
            let glyph_span = if is_no_color {
                if is_dead {
                    Span::raw(format!("![{}]", ordinal))
                } else {
                    Span::raw(format!("[{}]", ordinal))
                }
            } else {
                let color = if is_dead {
                    // Dead links always render in red regardless of palette.
                    Color::Red
                } else {
                    link_colors.get(ordinal).color()
                };
                Span::styled(format!("[{}]", ordinal), Style::default().fg(color))
            };
            spans.push(glyph_span);

            // Record in link map.
            link_map.push(LinkEntry {
                ordinal,
                page_title,
                is_dead,
                line_number: line_idx,
            });

            prev_end = end;
        }

        // Remaining text after the last wikilink on this line.
        if prev_end < line.len() {
            spans.push(Span::raw(line[prev_end..].to_owned()));
        }

        annotated.push(Line::from(spans));
    }

    (annotated, link_map)
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_wikilinks_in_line ─────────────────────────────────────────

    #[test]
    fn parse_single_wikilink() {
        let line = "See [[Foo]] here.";
        let links = parse_wikilinks_in_line(line);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].0, 4); // start of [[
        assert_eq!(links[0].1, 11); // end after ]]
        assert_eq!(links[0].2, "Foo");
    }

    #[test]
    fn parse_two_wikilinks_on_one_line() {
        let line = "See [[Foo]] and [[Bar]].";
        let links = parse_wikilinks_in_line(line);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].2, "Foo");
        assert_eq!(links[1].2, "Bar");
    }

    #[test]
    fn parse_wikilink_with_alias() {
        let links = parse_wikilinks_in_line("[[Target|Display Text]]");
        assert_eq!(links[0].2, "Target");
    }

    #[test]
    fn parse_wikilink_with_heading() {
        let links = parse_wikilinks_in_line("[[Page#Section]]");
        assert_eq!(links[0].2, "Page");
    }

    #[test]
    fn parse_wikilink_with_block_ref() {
        let links = parse_wikilinks_in_line("[[Page^block-id]]");
        assert_eq!(links[0].2, "Page");
    }

    #[test]
    fn parse_no_wikilinks() {
        let links = parse_wikilinks_in_line("Plain text with no links.");
        assert!(links.is_empty());
    }

    #[test]
    fn parse_unclosed_wikilink_ignored() {
        let links = parse_wikilinks_in_line("[[unclosed");
        assert!(links.is_empty());
    }

    // ── build_annotated_lines ───────────────────────────────────────────

    fn make_page_set(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn annotated_live_link_colored() {
        let lines = vec!["See [[Foo]] and [[Bar]].".to_string()];
        let page_set = make_page_set(&["Foo", "Bar"]);
        let (annotated, link_map) = build_annotated_lines(&lines, &page_set, ColorMode::TrueColor);

        // Two links in link_map.
        assert_eq!(link_map.len(), 2);
        assert_eq!(link_map[0].ordinal, 1);
        assert_eq!(link_map[0].page_title, "Foo");
        assert!(!link_map[0].is_dead);
        assert_eq!(link_map[0].line_number, 0);
        assert_eq!(link_map[1].ordinal, 2);
        assert_eq!(link_map[1].page_title, "Bar");
        assert!(!link_map[1].is_dead);

        // Annotated line has spans including glyphs.
        let line = &annotated[0];
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "See [[Foo]][1] and [[Bar]][2].");
    }

    #[test]
    fn dead_link_no_color_gets_bang_prefix() {
        let lines = vec!["[[Dead]]".to_string()];
        let page_set = make_page_set(&[]); // no pages in index
        let (annotated, link_map) = build_annotated_lines(&lines, &page_set, ColorMode::NoColor);

        assert_eq!(link_map.len(), 1);
        assert!(link_map[0].is_dead);

        let text: String = annotated[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(text, "[[Dead]]![1]");
    }

    #[test]
    fn live_link_no_color_no_bang() {
        let lines = vec!["[[Alive]]".to_string()];
        let page_set = make_page_set(&["Alive"]);
        let (annotated, link_map) = build_annotated_lines(&lines, &page_set, ColorMode::NoColor);

        assert!(!link_map[0].is_dead);
        let text: String = annotated[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(text, "[[Alive]][1]");
    }

    #[test]
    fn dead_link_colored_renders_red() {
        let lines = vec!["[[Missing]]".to_string()];
        let page_set = make_page_set(&[]);
        let (_annotated, link_map) = build_annotated_lines(&lines, &page_set, ColorMode::Ansi8);

        assert!(link_map[0].is_dead);
        // Verify the glyph span has red foreground.
        let line = &_annotated[0];
        // The last span is the glyph.
        let glyph = line.spans.last().unwrap();
        assert_eq!(glyph.style.fg, Some(Color::Red));
    }

    #[test]
    fn line_without_wikilinks_is_raw() {
        let lines = vec!["Just plain text.".to_string()];
        let page_set = make_page_set(&[]);
        let (annotated, link_map) = build_annotated_lines(&lines, &page_set, ColorMode::Ansi8);

        assert!(link_map.is_empty());
        assert_eq!(annotated.len(), 1);
        let text: String = annotated[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(text, "Just plain text.");
    }

    #[test]
    fn ordinals_are_document_order_across_lines() {
        let lines = vec![
            "[[A]]".to_string(),
            "[[B]] and [[C]]".to_string(),
            "[[A]] again".to_string(), // same target, new occurrence
        ];
        let page_set = make_page_set(&["A", "B", "C"]);
        let (_annotated, link_map) = build_annotated_lines(&lines, &page_set, ColorMode::NoColor);

        assert_eq!(link_map.len(), 4);
        assert_eq!(link_map[0].ordinal, 1);
        assert_eq!(link_map[1].ordinal, 2);
        assert_eq!(link_map[2].ordinal, 3);
        assert_eq!(link_map[3].ordinal, 4);
        assert_eq!(link_map[3].page_title, "A"); // same page, new occurrence
    }

    #[test]
    fn link_map_records_correct_line_numbers() {
        let lines = vec![
            "no links".to_string(),
            "[[X]]".to_string(),
            "[[Y]]".to_string(),
        ];
        let page_set = make_page_set(&["X", "Y"]);
        let (_annotated, link_map) = build_annotated_lines(&lines, &page_set, ColorMode::NoColor);

        assert_eq!(link_map[0].line_number, 1);
        assert_eq!(link_map[1].line_number, 2);
    }
}
