use std::path::Path;

use anyhow::Result;
use globset::GlobBuilder;
use serde::Serialize;

use crate::scanner::{body_text_ranges, scan_vault};
use crate::search_index::SearchIndex;

/// A single search match within a file.
#[derive(Debug, Serialize)]
pub struct SearchMatch {
    pub page: String,
    pub path: String,
    pub line: u32,
    pub column: u32,
    pub context: Option<String>,
    pub heading: Option<String>,
    pub heading_level: Option<u8>,
    pub score: f64,
}

/// Output envelope for the search command.
#[derive(Debug, Serialize)]
pub struct SearchOutput {
    pub query: String,
    pub total_matches: usize,
    /// Resolved anchor page name (present only when --near is used). REQ-013-009.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub near: Option<String>,
    /// Hop depth used (present only when --near is used). REQ-013-009.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth: Option<usize>,
    /// Number of pages in the neighbourhood (present only when --near is used). REQ-013-009.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub neighbourhood_size: Option<usize>,
    pub results: Vec<SearchMatch>,
}

/// A heading found within the body text of a Markdown file.
///
/// `byte_offset` is the byte position of the first `#` character.
/// `level` is in the range 1–6. `text` has the leading `#` markers and
/// mandatory whitespace separator stripped, with trailing whitespace removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHeading {
    pub byte_offset: usize,
    pub level: u8,
    pub text: String,
}

/// Detect ATX headings (`# …` through `###### …`) that fall within body text ranges.
///
/// Scans `content` line by line. Any line whose first characters match `^#{1,6}[\t ]`
/// is a candidate; it is included only when its starting byte offset lies within one
/// of the supplied `body_text_ranges`. This excludes headings inside frontmatter,
/// fenced code blocks, and HTML comments (ADR-013-003).
///
/// The returned slice is sorted ascending by `byte_offset`. Setext-style headings
/// are not detected (out of scope per ADR-013-003).
///
/// REQ-013-010.
pub fn detect_headings(content: &str, body_text_ranges: &[(usize, usize)]) -> Vec<FileHeading> {
    let mut headings = Vec::new();
    let mut byte_offset = 0usize;

    for line in content.split('\n') {
        let level = line.bytes().take_while(|&b| b == b'#').count();
        if level >= 1 && level <= 6 {
            let after_hashes = &line[level..];
            if after_hashes.starts_with(' ') || after_hashes.starts_with('\t') {
                if in_body_text(byte_offset, body_text_ranges) {
                    let text = after_hashes[1..].trim_end().to_string();
                    headings.push(FileHeading {
                        byte_offset,
                        level: level as u8,
                        text,
                    });
                }
            }
        }
        byte_offset += line.len() + 1; // +1 for the '\n' we split on
    }

    headings
}

/// Return the nearest heading at or before `byte_offset`.
///
/// Uses binary search on the sorted `headings` slice (produced by [`detect_headings`]).
/// Returns `(heading_text, heading_level)`, or `(None, None)` if no heading precedes
/// the given offset.
///
/// REQ-013-011.
pub fn find_heading_for_offset(
    headings: &[FileHeading],
    byte_offset: usize,
) -> (Option<String>, Option<u8>) {
    let idx = headings.partition_point(|h| h.byte_offset <= byte_offset);
    if idx == 0 {
        return (None, None);
    }
    let h = &headings[idx - 1];
    (Some(h.text.clone()), Some(h.level))
}

/// Configuration for a search operation.
pub struct SearchConfig<'a> {
    pub query: &'a str,
    pub context_chars: usize,
    pub limit: usize,
    pub case_sensitive: bool,
    pub path_filter: Option<&'a str>,
}

/// Search all Markdown files in `vault_root` for matches via Tantivy.
///
/// Opens the Tantivy index at `.zetl/search/`, building it lazily if absent.
/// Queries the index against the body field, then re-scans each matched document
/// for precise line/column positions and heading context.
///
/// REQ-013-002, REQ-013-005, CON-013-001.
pub fn search_vault(vault_root: &Path, config: &SearchConfig) -> Result<SearchOutput> {
    if config.query.trim().is_empty() {
        anyhow::bail!("Empty search query");
    }

    // Open the Tantivy index, building it lazily if it doesn't exist.
    let index = match SearchIndex::open(vault_root)? {
        Some(idx) => idx,
        None => {
            eprintln!("Building search index (run `zetl index` to avoid this delay on future queries)");
            let files = scan_vault(vault_root, &[])?;
            SearchIndex::build(vault_root, &files)?
        }
    };

    // Build path filter glob if specified.
    let path_glob = if let Some(pattern) = config.path_filter {
        let pat = if pattern.ends_with('/') {
            format!("{pattern}**")
        } else {
            pattern.to_string()
        };
        let glob = GlobBuilder::new(&pat)
            .literal_separator(false)
            .build()
            .map_err(|e| anyhow::anyhow!("Invalid path glob: {e}"))?
            .compile_matcher();
        Some(glob)
    } else {
        None
    };

    // Query the index for top-scoring documents (limited to config.limit documents).
    let hits = index.query(config.query, config.limit)?;

    // Split query into individual terms for re-scanning.
    let terms: Vec<String> = config
        .query
        .split_whitespace()
        .map(|t| {
            if config.case_sensitive {
                t.to_string()
            } else {
                t.to_lowercase()
            }
        })
        .collect();

    let mut all_matches: Vec<SearchMatch> = Vec::new();

    for hit in hits {
        // Apply optional path filter.
        let rel_path = std::path::Path::new(&hit.path);
        if let Some(ref glob) = path_glob {
            if !glob.is_match(rel_path) {
                continue;
            }
        }

        // Read the original file from disk.
        let abs_path = vault_root.join(&hit.path);
        let content = match std::fs::read_to_string(&abs_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let body_ranges = body_text_ranges(&content);
        let line_starts: Vec<usize> = std::iter::once(0)
            .chain(content.match_indices('\n').map(|(i, _)| i + 1))
            .collect();

        let headings = detect_headings(&content, &body_ranges);

        let search_content = if config.case_sensitive {
            content.clone()
        } else {
            content.to_lowercase()
        };

        // Scan for each query term within body text only.
        for term in &terms {
            let mut start = 0usize;
            while let Some(pos) = search_content[start..].find(term.as_str()) {
                let byte_offset = start + pos;
                start = byte_offset + 1;

                if !in_body_text(byte_offset, &body_ranges) {
                    continue;
                }

                let (line, col) = byte_offset_to_line_col(&line_starts, byte_offset);
                let ctx =
                    extract_search_context(&content, byte_offset, term.len(), config.context_chars);
                let (heading, heading_level) = find_heading_for_offset(&headings, byte_offset);

                all_matches.push(SearchMatch {
                    page: hit.page_name.clone(),
                    path: hit.path.clone(),
                    line,
                    column: col,
                    context: ctx,
                    heading,
                    heading_level,
                    score: hit.score,
                });
            }
        }
    }

    // Sort by descending score, ties broken by path (ascending), then line (ascending).
    all_matches.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.path.cmp(&b.path))
            .then(a.line.cmp(&b.line))
    });

    let total = all_matches.len();
    all_matches.truncate(config.limit);
    Ok(SearchOutput {
        query: config.query.to_string(),
        total_matches: total,
        near: None,
        depth: None,
        neighbourhood_size: None,
        results: all_matches,
    })
}

/// Check if a byte offset falls within any of the body-text ranges.
fn in_body_text(byte_offset: usize, body_ranges: &[(usize, usize)]) -> bool {
    body_ranges
        .iter()
        .any(|&(start, end)| byte_offset >= start && byte_offset < end)
}

/// Convert a byte offset to (line, column), both 1-indexed.
fn byte_offset_to_line_col(line_starts: &[usize], byte_offset: usize) -> (u32, u32) {
    let line_idx = line_starts
        .partition_point(|&start| start <= byte_offset)
        .saturating_sub(1);
    let col = byte_offset - line_starts[line_idx];
    ((line_idx + 1) as u32, (col + 1) as u32)
}

/// Snap a byte index to the nearest char boundary at or before it.
fn floor_char_boundary(s: &str, index: usize) -> usize {
    let mut i = index.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Snap a byte index to the nearest char boundary at or after it.
fn ceil_char_boundary(s: &str, index: usize) -> usize {
    let mut i = index.min(s.len());
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

fn extract_search_context(
    content: &str,
    byte_offset: usize,
    match_len: usize,
    context_chars: usize,
) -> Option<String> {
    if context_chars == 0 {
        return None;
    }

    // Find the line containing this match
    let line_start = content[..byte_offset].rfind('\n').map_or(0, |i| i + 1);
    let line_end = content[byte_offset..]
        .find('\n')
        .map_or(content.len(), |i| byte_offset + i);
    let line = &content[line_start..line_end];

    let pos_in_line = byte_offset - line_start;
    let match_end = pos_in_line + match_len;
    let ctx_start = floor_char_boundary(line, pos_in_line.saturating_sub(context_chars));
    let ctx_end = ceil_char_boundary(line, (match_end + context_chars).min(line.len()));

    Some(line[ctx_start..ctx_end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_byte_offset_to_line_col() {
        let content = "line1\nline2\nline3";
        let line_starts: Vec<usize> = std::iter::once(0)
            .chain(content.match_indices('\n').map(|(i, _)| i + 1))
            .collect();

        assert_eq!(byte_offset_to_line_col(&line_starts, 0), (1, 1)); // 'l' of line1
        assert_eq!(byte_offset_to_line_col(&line_starts, 6), (2, 1)); // 'l' of line2
        assert_eq!(byte_offset_to_line_col(&line_starts, 8), (2, 3)); // 'n' of line2
        assert_eq!(byte_offset_to_line_col(&line_starts, 12), (3, 1)); // 'l' of line3
    }

    #[test]
    fn test_extract_search_context_zero() {
        assert_eq!(extract_search_context("hello world", 0, 5, 0), None);
    }

    #[test]
    fn test_extract_search_context_with_chars() {
        let content = "The quick brown fox jumps";
        // "quick" starts at byte 4, length 5
        let ctx = extract_search_context(content, 4, 5, 3);
        assert_eq!(ctx, Some("he quick br".to_string()));
    }

    #[test]
    fn test_in_body_text() {
        let ranges = vec![(0, 10), (20, 30)];
        assert!(in_body_text(5, &ranges));
        assert!(!in_body_text(15, &ranges));
        assert!(in_body_text(25, &ranges));
    }

    // ── detect_headings ──────────────────────────────────────────────────────

    #[test]
    fn test_detect_headings_basic() {
        let content = "# Hello\n## World\n### Three\n";
        // All body text
        let ranges = vec![(0, content.len())];
        let headings = detect_headings(content, &ranges);
        assert_eq!(headings.len(), 3);
        assert_eq!(headings[0], FileHeading { byte_offset: 0, level: 1, text: "Hello".into() });
        assert_eq!(headings[1], FileHeading { byte_offset: 8, level: 2, text: "World".into() });
        assert_eq!(headings[2], FileHeading { byte_offset: 17, level: 3, text: "Three".into() });
    }

    #[test]
    fn test_detect_headings_excluded_by_ranges() {
        // "# Code\n" is inside a code block (not in body ranges)
        // "# Body\n" is in body text
        let content = "# Code\n# Body\n";
        // Only the second line is body text (byte 7..14)
        let ranges = vec![(7, 14)];
        let headings = detect_headings(content, &ranges);
        assert_eq!(headings.len(), 1);
        assert_eq!(headings[0].text, "Body");
        assert_eq!(headings[0].byte_offset, 7);
    }

    #[test]
    fn test_detect_headings_levels() {
        let content = "# h1\n## h2\n### h3\n#### h4\n##### h5\n###### h6\n####### not\n";
        let ranges = vec![(0, content.len())];
        let headings = detect_headings(content, &ranges);
        assert_eq!(headings.len(), 6);
        for (i, h) in headings.iter().enumerate() {
            assert_eq!(h.level, (i + 1) as u8);
        }
        // 7 hashes: not a heading
        assert!(headings.iter().all(|h| h.text != "not"));
    }

    #[test]
    fn test_detect_headings_no_space_not_heading() {
        // "##nospace" should not be detected (no space/tab after hashes)
        let content = "##nospace\n# valid\n";
        let ranges = vec![(0, content.len())];
        let headings = detect_headings(content, &ranges);
        assert_eq!(headings.len(), 1);
        assert_eq!(headings[0].text, "valid");
    }

    #[test]
    fn test_detect_headings_trailing_whitespace_stripped() {
        let content = "# Title   \n";
        let ranges = vec![(0, content.len())];
        let headings = detect_headings(content, &ranges);
        assert_eq!(headings[0].text, "Title");
    }

    #[test]
    fn test_detect_headings_empty_ranges() {
        let content = "# Hello\n";
        let headings = detect_headings(content, &[]);
        assert!(headings.is_empty());
    }

    // ── find_heading_for_offset ───────────────────────────────────────────────

    #[test]
    fn test_find_heading_for_offset_basic() {
        let content = "# Intro\n\nSome text.\n\n## Details\n\nMore text.\n";
        let ranges = vec![(0, content.len())];
        let headings = detect_headings(content, &ranges);
        // "More text." starts at byte offset 32 (after "## Details\n\n")
        let more_offset = content.find("More text.").unwrap();
        let (text, level) = find_heading_for_offset(&headings, more_offset);
        assert_eq!(text.as_deref(), Some("Details"));
        assert_eq!(level, Some(2));
    }

    #[test]
    fn test_find_heading_for_offset_before_first_heading() {
        let content = "preamble\n# Heading\n";
        let ranges = vec![(0, content.len())];
        let headings = detect_headings(content, &ranges);
        // offset 0 is before any heading
        let (text, level) = find_heading_for_offset(&headings, 0);
        assert_eq!(text, None);
        assert_eq!(level, None);
    }

    #[test]
    fn test_find_heading_for_offset_at_heading() {
        let content = "# Title\nbody\n";
        let ranges = vec![(0, content.len())];
        let headings = detect_headings(content, &ranges);
        // Offset exactly at the heading itself
        let (text, level) = find_heading_for_offset(&headings, 0);
        assert_eq!(text.as_deref(), Some("Title"));
        assert_eq!(level, Some(1));
    }

    #[test]
    fn test_find_heading_for_offset_empty() {
        let (text, level) = find_heading_for_offset(&[], 42);
        assert_eq!(text, None);
        assert_eq!(level, None);
    }

    #[test]
    fn test_find_heading_for_offset_returns_nearest() {
        let content = "# A\n## B\n### C\ntext\n";
        let ranges = vec![(0, content.len())];
        let headings = detect_headings(content, &ranges);
        // "text" starts after "### C\n"
        let text_offset = content.find("text").unwrap();
        let (text, level) = find_heading_for_offset(&headings, text_offset);
        assert_eq!(text.as_deref(), Some("C"));
        assert_eq!(level, Some(3));
    }
}
