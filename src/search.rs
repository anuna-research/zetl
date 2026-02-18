use std::path::Path;

use anyhow::Result;
use ignore::WalkBuilder;
use regex::Regex;
use serde::Serialize;

use crate::scanner::{body_text_ranges, page_name_from_path};

/// A single search match within a file.
#[derive(Debug, Serialize)]
pub struct SearchMatch {
    pub page: String,
    pub path: String,
    pub line: u32,
    pub column: u32,
    pub context: Option<String>,
}

/// Output envelope for the search command.
#[derive(Debug, Serialize)]
pub struct SearchOutput {
    pub query: String,
    pub regex: bool,
    pub total_matches: usize,
    pub results: Vec<SearchMatch>,
}

/// Configuration for a search operation.
pub struct SearchConfig<'a> {
    pub query: &'a str,
    pub context_chars: usize,
    pub limit: usize,
    pub regex: bool,
    pub case_sensitive: bool,
    pub body_only: bool,
}

/// Search all Markdown files in `vault_root` for matches.
///
/// Walks the directory tree respecting ignore patterns (same as scan_vault),
/// reads each file, and matches the query against content. When `body_only`
/// is true, matches inside frontmatter, code blocks, inline code, and HTML
/// comments are skipped.
pub fn search_vault(vault_root: &Path, config: &SearchConfig) -> Result<SearchOutput> {
    let matcher = build_matcher(config)?;

    let mut all_matches: Vec<SearchMatch> = Vec::new();
    let mut total = 0usize;

    let mut builder = WalkBuilder::new(vault_root);
    builder
        .hidden(false)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(false);

    let zetlignore = vault_root.join(".zetlignore");
    if zetlignore.exists() {
        builder.add_ignore(&zetlignore);
    }

    let mut overrides = ignore::overrides::OverrideBuilder::new(vault_root);
    overrides.add("!.git/")?;
    overrides.add("!node_modules/")?;
    overrides.add("!.zetl/")?;
    builder.overrides(overrides.build()?);

    for entry in builder.build() {
        let entry = entry?;
        let path = entry.path();

        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str());
        if ext != Some("md") {
            continue;
        }

        let rel_path = path.strip_prefix(vault_root).unwrap_or(path);
        let page_name = page_name_from_path(rel_path);
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let body_ranges = if config.body_only {
            Some(body_text_ranges(&content))
        } else {
            None
        };

        let file_matches = find_matches_in_content(
            &content,
            &matcher,
            &page_name,
            &rel_path.to_string_lossy(),
            config.context_chars,
            body_ranges.as_deref(),
        );

        for m in file_matches {
            total += 1;
            if all_matches.len() < config.limit {
                all_matches.push(m);
            }
        }
    }

    // Sort by path, then line
    all_matches.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));

    Ok(SearchOutput {
        query: config.query.to_string(),
        regex: config.regex,
        total_matches: total,
        results: all_matches,
    })
}

/// Internal matcher abstraction — literal or regex.
enum Matcher {
    Literal { pattern: String, case_sensitive: bool },
    Regex(Regex),
}

fn build_matcher(config: &SearchConfig) -> Result<Matcher> {
    if config.regex {
        let pattern = if config.case_sensitive {
            config.query.to_string()
        } else {
            format!("(?i){}", config.query)
        };
        let re = Regex::new(&pattern)
            .map_err(|e| anyhow::anyhow!("Invalid regex: {e}"))?;
        Ok(Matcher::Regex(re))
    } else {
        Ok(Matcher::Literal {
            pattern: if config.case_sensitive {
                config.query.to_string()
            } else {
                config.query.to_lowercase()
            },
            case_sensitive: config.case_sensitive,
        })
    }
}

/// Check if a byte offset falls within any of the body-text ranges.
fn in_body_text(byte_offset: usize, body_ranges: &[(usize, usize)]) -> bool {
    body_ranges
        .iter()
        .any(|&(start, end)| byte_offset >= start && byte_offset < end)
}

/// Find all matches in a single file's content.
fn find_matches_in_content(
    content: &str,
    matcher: &Matcher,
    page_name: &str,
    rel_path: &str,
    context_chars: usize,
    body_ranges: Option<&[(usize, usize)]>,
) -> Vec<SearchMatch> {
    let mut results = Vec::new();

    // Pre-compute line start byte offsets for O(1) line/column lookup
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(content.match_indices('\n').map(|(i, _)| i + 1))
        .collect();

    match matcher {
        Matcher::Literal { pattern, case_sensitive } => {
            let search_content = if *case_sensitive {
                content.to_string()
            } else {
                content.to_lowercase()
            };

            let mut start = 0;
            while let Some(pos) = search_content[start..].find(pattern.as_str()) {
                let byte_offset = start + pos;
                start = byte_offset + 1;

                if let Some(ranges) = body_ranges {
                    if !in_body_text(byte_offset, ranges) {
                        continue;
                    }
                }

                let (line, col) = byte_offset_to_line_col(&line_starts, byte_offset);
                let ctx = extract_search_context(content, byte_offset, pattern.len(), context_chars);
                results.push(SearchMatch {
                    page: page_name.to_string(),
                    path: rel_path.to_string(),
                    line,
                    column: col,
                    context: ctx,
                });
            }
        }
        Matcher::Regex(re) => {
            for mat in re.find_iter(content) {
                let byte_offset = mat.start();

                if let Some(ranges) = body_ranges {
                    if !in_body_text(byte_offset, ranges) {
                        continue;
                    }
                }

                let (line, col) = byte_offset_to_line_col(&line_starts, byte_offset);
                let ctx = extract_search_context(content, byte_offset, mat.len(), context_chars);
                results.push(SearchMatch {
                    page: page_name.to_string(),
                    path: rel_path.to_string(),
                    line,
                    column: col,
                    context: ctx,
                });
            }
        }
    }

    results
}

/// Convert a byte offset to (line, column), both 1-indexed.
fn byte_offset_to_line_col(line_starts: &[usize], byte_offset: usize) -> (u32, u32) {
    let line_idx = line_starts.partition_point(|&start| start <= byte_offset).saturating_sub(1);
    let col = byte_offset - line_starts[line_idx];
    ((line_idx + 1) as u32, (col + 1) as u32)
}

/// Extract context characters around a match.
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

    #[test]
    fn test_literal_case_insensitive() {
        let content = "Hello World\nhello again\nGoodbye";
        let matcher = Matcher::Literal {
            pattern: "hello".to_string(),
            case_sensitive: false,
        };
        let matches = find_matches_in_content(content, &matcher, "Test", "test.md", 0, None);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].line, 1);
        assert_eq!(matches[1].line, 2);
    }

    #[test]
    fn test_literal_case_sensitive() {
        let content = "Hello World\nhello again";
        let matcher = Matcher::Literal {
            pattern: "Hello".to_string(),
            case_sensitive: true,
        };
        let matches = find_matches_in_content(content, &matcher, "Test", "test.md", 0, None);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].line, 1);
    }

    #[test]
    fn test_body_text_exclusion() {
        let content = "body match\nexcluded match\nbody match again";
        // Only lines 1 and 3 are body text (bytes 0..11 and 26..44)
        let body_ranges = vec![(0, 11), (26, 44)];
        let matcher = Matcher::Literal {
            pattern: "match".to_string(),
            case_sensitive: false,
        };
        let matches =
            find_matches_in_content(content, &matcher, "Test", "test.md", 0, Some(&body_ranges));
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].line, 1);
        assert_eq!(matches[1].line, 3);
    }

    #[test]
    fn test_regex_matching() {
        let content = "note notes notation";
        let re = Regex::new(r"(?i)\bnotes?\b").unwrap();
        let matcher = Matcher::Regex(re);
        let matches = find_matches_in_content(content, &matcher, "Test", "test.md", 0, None);
        assert_eq!(matches.len(), 2); // "note" and "notes", not "notation"
    }

    #[test]
    fn test_column_numbers() {
        let content = "abc match xyz";
        let matcher = Matcher::Literal {
            pattern: "match".to_string(),
            case_sensitive: false,
        };
        let matches = find_matches_in_content(content, &matcher, "Test", "test.md", 0, None);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].line, 1);
        assert_eq!(matches[0].column, 5); // 1-indexed: 'match' starts at byte 4, col = 5
    }
}
