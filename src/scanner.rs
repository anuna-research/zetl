use crate::types::{Diagnostic, DiagnosticLevel, ParsedFile, WikiLink};
use anyhow::Result;
use ignore::WalkBuilder;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use regex::Regex;
use std::path::{Path, PathBuf};

/// Scan a vault directory and parse all markdown files.
///
/// Walks the directory tree using the `ignore` crate (respects .gitignore-style patterns).
/// Applies default ignores for `.git`, `node_modules`, and `.zetl`, plus any custom patterns
/// from a `.zetlignore` file at the vault root.
pub fn scan_vault(root: &Path, ignore_patterns: &[String]) -> Result<Vec<ParsedFile>> {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false) // don't skip hidden files by default (user may have .files as notes)
        .git_ignore(true) // respect .gitignore
        .git_global(false)
        .git_exclude(false);

    // Add .zetlignore if it exists
    let zetlignore = root.join(".zetlignore");
    if zetlignore.exists() {
        builder.add_ignore(&zetlignore);
    }

    // Add custom ignore patterns via an in-memory override
    let mut overrides = ignore::overrides::OverrideBuilder::new(root);
    // Default ignores
    overrides.add("!.git/")?;
    overrides.add("!node_modules/")?;
    overrides.add("!.zetl/")?;
    for pattern in ignore_patterns {
        overrides.add(&format!("!{}", pattern))?;
    }
    builder.overrides(overrides.build()?);

    let mut parsed_files = Vec::new();

    for entry in builder.build() {
        let entry = entry?;
        let path = entry.path();

        // Only process .md files
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str());
        if ext != Some("md") {
            continue;
        }

        let rel_path = path.strip_prefix(root).unwrap_or(path).to_path_buf();
        let page_name = page_name_from_path(&rel_path);
        let content = std::fs::read_to_string(path)?;
        let mtime = std::fs::metadata(path)?.modified()?;

        let mut parsed = parse_file(&rel_path, &content, &page_name);
        parsed.mtime = mtime;
        parsed_files.push(parsed);
    }

    Ok(parsed_files)
}

/// Parse wikilinks from markdown content, respecting code blocks and comments.
///
/// Composes `body_text_ranges`, `extract_wikilinks`, and `validate_syntax` to produce
/// a complete `ParsedFile` with only links from body text and all syntax diagnostics.
pub fn parse_file(path: &Path, content: &str, page_name: &str) -> ParsedFile {
    // Get body text ranges (excludes code blocks, comments, frontmatter)
    let ranges = body_text_ranges(content);

    // Extract wikilinks only from body text regions
    let all_links = extract_wikilinks(content);
    let links: Vec<WikiLink> = all_links
        .into_iter()
        .filter(|link| {
            // Compute byte offset from line/column (1-indexed)
            let byte_offset = line_col_to_byte_offset(content, link.line, link.column);
            ranges
                .iter()
                .any(|&(start, end)| byte_offset >= start && byte_offset < end)
        })
        .collect();

    // Validate syntax
    let diagnostics = validate_syntax(path, content);

    ParsedFile {
        path: path.to_path_buf(),
        page_name: page_name.to_string(),
        links,
        diagnostics,
        mtime: std::time::SystemTime::UNIX_EPOCH, // caller sets real mtime
    }
}

/// Convert 1-indexed line and column to a byte offset in the content string.
fn line_col_to_byte_offset(content: &str, line: u32, column: u32) -> usize {
    let mut current_line = 1u32;
    let mut line_start = 0usize;
    for (i, ch) in content.char_indices() {
        if current_line == line {
            line_start = i;
            break;
        }
        if ch == '\n' {
            current_line += 1;
        }
    }
    if current_line < line {
        // line is past the end of content
        return content.len();
    }
    // column is 1-indexed, advance by (column-1) characters from line_start
    let mut col = 1u32;
    for (i, _) in content[line_start..].char_indices() {
        if col == column {
            return line_start + i;
        }
        col += 1;
    }
    content.len()
}

/// Extract wikilinks from raw text.
///
/// Finds all `!?[[...]]` patterns and parses the inner content to extract
/// the page name, heading, block reference, alias, and position information.
/// Code block filtering is NOT handled here (that is a separate concern).
pub fn extract_wikilinks(content: &str) -> Vec<WikiLink> {
    // Match optional ! followed by [[ ... ]]
    // The inner content must not contain [ or ] characters.
    let re = Regex::new(r"(!?)\[\[([^\[\]]+)\]\]").expect("invalid wikilink regex");

    let mut links = Vec::new();

    // Pre-compute line start byte offsets for efficient line/column lookup.
    // Each entry is the byte offset where that line begins (0-indexed lines here,
    // but we will report 1-indexed).
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(content.match_indices('\n').map(|(i, _)| i + 1))
        .collect();

    for cap in re.captures_iter(content) {
        let full_match = cap.get(0).unwrap();
        let embed_str = &cap[1];
        let inner = &cap[2]; // everything between [[ and ]]

        let is_embed = embed_str == "!";

        // The position we report is the start of the `[[` (or `![[` if embed).
        let match_start = full_match.start();

        // Compute 1-indexed line and column from byte offset.
        let line_idx = line_starts.partition_point(|&offset| offset <= match_start) - 1;
        let line = (line_idx + 1) as u32;
        let column = (match_start - line_starts[line_idx] + 1) as u32;

        // Split on the FIRST `|` to separate target from alias.
        let (target_part, alias) = match inner.find('|') {
            Some(pos) => {
                let alias_text = inner[pos + 1..].to_string();
                (&inner[..pos], Some(alias_text))
            }
            None => (inner, None),
        };

        // raw_target is the target portion (everything before alias split, i.e. the
        // part that identifies the page/heading/block).
        let raw_target = target_part.to_string();

        // Now parse target_part for heading (#) or block reference (^).
        // A block ref takes precedence if ^ comes before # (per the grammar,
        // heading-ref and block-ref are alternatives, not both).
        // We split on whichever delimiter comes first.
        let hash_pos = target_part.find('#');
        let caret_pos = target_part.find('^');

        let (target_page, heading, block_ref) = match (hash_pos, caret_pos) {
            (Some(h), Some(c)) if h < c => {
                // # comes first: treat as heading reference
                let page = target_part[..h].trim().to_string();
                let heading_text = target_part[h + 1..].trim().to_string();
                (page, Some(heading_text), None)
            }
            (Some(_h), Some(c)) => {
                // ^ comes first: treat as block reference
                let page = target_part[..c].trim().to_string();
                let block_text = target_part[c + 1..].trim().to_string();
                (page, None, Some(block_text))
            }
            (Some(h), None) => {
                // Only heading
                let page = target_part[..h].trim().to_string();
                let heading_text = target_part[h + 1..].trim().to_string();
                (page, Some(heading_text), None)
            }
            (None, Some(c)) => {
                // Only block ref
                let page = target_part[..c].trim().to_string();
                let block_text = target_part[c + 1..].trim().to_string();
                (page, None, Some(block_text))
            }
            (None, None) => {
                // Plain page reference
                (target_part.trim().to_string(), None, None)
            }
        };

        links.push(WikiLink {
            target_page,
            raw_target,
            heading,
            block_ref,
            alias,
            is_embed,
            line,
            column,
        });
    }

    links
}

/// Detect malformed wikilink syntax
///
/// Scans content line-by-line, skipping fenced code blocks and inline code
/// spans, and reports:
/// - Unclosed wikilinks (`[[` without matching `]]`)
/// - Empty wikilinks (`[[]]`)
/// - Nested brackets (`[[` inside an already-open wikilink)
pub fn validate_syntax(path: &Path, content: &str) -> Vec<Diagnostic> {
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    // Track fenced code block state (``` or ~~~)
    let mut in_fenced_code_block = false;

    for (line_idx, line) in content.lines().enumerate() {
        let line_number = (line_idx + 1) as u32;

        // Check for fenced code block toggles (``` or ~~~)
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fenced_code_block = !in_fenced_code_block;
            continue;
        }

        if in_fenced_code_block {
            continue;
        }

        // Process the line character by character, skipping inline code spans.
        // We work on bytes for indexing but use char boundaries carefully.
        let chars: Vec<char> = line.chars().collect();
        let len = chars.len();
        let mut i = 0;

        while i < len {
            // Skip inline code spans: ` ... `
            if chars[i] == '`' {
                // Find the closing backtick
                i += 1;
                while i < len && chars[i] != '`' {
                    i += 1;
                }
                if i < len {
                    i += 1; // skip closing backtick
                }
                continue;
            }

            // Look for `[[`
            if i + 1 < len && chars[i] == '[' && chars[i + 1] == '[' {
                let open_col = (i + 1) as u32; // 1-indexed column
                let open_start = i;
                i += 2; // skip past `[[`

                // Check for empty wikilink: `[[]]`
                if i + 1 < len && chars[i] == ']' && chars[i + 1] == ']' {
                    diagnostics.push(Diagnostic {
                        level: DiagnosticLevel::Error,
                        message: "Empty wikilink: '[[]]'".to_string(),
                        file: path.to_path_buf(),
                        line: line_number,
                        column: open_col,
                    });
                    i += 2; // skip past `]]`
                    continue;
                }

                // Scan for closing `]]`, watching for nested `[[`
                let mut found_close = false;
                let mut found_nested = false;

                while i < len {
                    // Check for nested `[[`
                    if i + 1 < len && chars[i] == '[' && chars[i + 1] == '[' {
                        found_nested = true;
                        diagnostics.push(Diagnostic {
                            level: DiagnosticLevel::Error,
                            message: "Nested brackets in wikilink".to_string(),
                            file: path.to_path_buf(),
                            line: line_number,
                            column: open_col,
                        });
                        // Skip to after the nested closing `]]` or end of line
                        // to avoid cascading errors. Find the outermost `]]`.
                        let mut depth = 2; // two open `[[` sequences
                        i += 2;
                        while i < len && depth > 0 {
                            if i + 1 < len && chars[i] == ']' && chars[i + 1] == ']' {
                                depth -= 1;
                                i += 2;
                            } else if i + 1 < len && chars[i] == '[' && chars[i + 1] == '[' {
                                depth += 1;
                                i += 2;
                            } else {
                                i += 1;
                            }
                        }
                        found_close = true; // we consumed everything related to this
                        break;
                    }

                    // Check for closing `]]`
                    if i + 1 < len && chars[i] == ']' && chars[i + 1] == ']' {
                        found_close = true;
                        i += 2; // skip past `]]`
                        break;
                    }

                    i += 1;
                }

                if !found_close && !found_nested {
                    // Unclosed wikilink - collect the text we saw for the message
                    let snippet: String = chars[open_start..len.min(open_start + 40)]
                        .iter()
                        .collect();
                    let display = if snippet.len() < (len - open_start) {
                        format!("{}...", snippet)
                    } else {
                        snippet
                    };
                    diagnostics.push(Diagnostic {
                        level: DiagnosticLevel::Error,
                        message: format!("Unclosed wikilink: '{}'", display),
                        file: path.to_path_buf(),
                        line: line_number,
                        column: open_col,
                    });
                }

                continue;
            }

            i += 1;
        }
    }

    diagnostics
}

/// Normalize a page name for comparison: lowercase, replace hyphens/underscores with spaces
pub fn normalize_page_name(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c == '-' || c == '_' { ' ' } else { c })
        .collect()
}

/// Resolve a page name against the file index.
///
/// Resolution follows SPEC-001 section 3.2:
/// 1. Exact case-insensitive match on page names
/// 2. Normalized match (spaces, hyphens, underscores equivalent)
/// 3. Path-qualified match if raw contains `/`
/// 4. Returns None if multiple matches found (ambiguous)
pub fn resolve_page_name(raw: &str, file_index: &[(String, PathBuf)]) -> Option<String> {
    // Step 1: Exact case-insensitive match on page names
    let raw_lower = raw.to_lowercase();
    let exact_matches: Vec<&str> = file_index
        .iter()
        .filter(|(page_name, _)| page_name.to_lowercase() == raw_lower)
        .map(|(page_name, _)| page_name.as_str())
        .collect();

    match exact_matches.len() {
        1 => return Some(exact_matches[0].to_string()),
        n if n > 1 => return None, // Ambiguous
        _ => {}
    }

    // Step 2: Normalized match (spaces, hyphens, underscores treated as equivalent)
    let raw_normalized = normalize_page_name(raw);
    let normalized_matches: Vec<&str> = file_index
        .iter()
        .filter(|(page_name, _)| normalize_page_name(page_name) == raw_normalized)
        .map(|(page_name, _)| page_name.as_str())
        .collect();

    match normalized_matches.len() {
        1 => return Some(normalized_matches[0].to_string()),
        n if n > 1 => return None, // Ambiguous
        _ => {}
    }

    // Step 3: Path-qualified match (if raw contains `/`)
    if raw.contains('/') {
        let raw_path_lower = raw.to_lowercase();
        let path_matches: Vec<&str> = file_index
            .iter()
            .filter(|(_, file_path)| {
                // Strip .md extension from the file path and compare case-insensitively
                let path_str = file_path.to_string_lossy();
                let path_sans_ext = path_str.strip_suffix(".md").unwrap_or(&path_str);
                path_sans_ext.to_lowercase() == raw_path_lower
            })
            .map(|(page_name, _)| page_name.as_str())
            .collect();

        match path_matches.len() {
            1 => return Some(path_matches[0].to_string()),
            n if n > 1 => return None, // Ambiguous
            _ => {}
        }

        // Also try normalized path matching
        let raw_path_normalized = normalize_page_name(raw);
        let normalized_path_matches: Vec<&str> = file_index
            .iter()
            .filter(|(_, file_path)| {
                let path_str = file_path.to_string_lossy();
                let path_sans_ext = path_str.strip_suffix(".md").unwrap_or(&path_str);
                normalize_page_name(path_sans_ext) == raw_path_normalized
            })
            .map(|(page_name, _)| page_name.as_str())
            .collect();

        match normalized_path_matches.len() {
            1 => return Some(normalized_path_matches[0].to_string()),
            _ if normalized_path_matches.len() > 1 => return None,
            _ => {}
        }
    }

    // No match found
    None
}

/// Derive page name from file path (strip .md, use filename)
pub fn page_name_from_path(path: &Path) -> String {
    path.file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

/// Given markdown content, return a list of (start_byte, end_byte) ranges that are body text
/// (i.e., NOT inside fenced code blocks, inline code, HTML comments, or YAML frontmatter).
///
/// Uses `pulldown-cmark` to parse the markdown and identify excluded regions, then returns
/// the complement (i.e., all byte ranges that are NOT excluded).
pub fn body_text_ranges(content: &str) -> Vec<(usize, usize)> {
    if content.is_empty() {
        return vec![];
    }

    let mut excluded: Vec<(usize, usize)> = Vec::new();

    // Enable YAML-style metadata blocks so pulldown-cmark can identify frontmatter.
    let mut options = Options::empty();
    options.insert(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);

    let parser = Parser::new_ext(content, options);

    // Track whether we are currently inside a code block or metadata block.
    let mut in_code_block = false;
    let mut code_block_start: usize = 0;
    let mut in_metadata_block = false;
    let mut metadata_block_start: usize = 0;

    for (event, range) in parser.into_offset_iter() {
        match event {
            // Fenced / indented code blocks: exclude the entire range from Start to End.
            Event::Start(Tag::CodeBlock(_)) => {
                in_code_block = true;
                code_block_start = range.start;
            }
            Event::End(TagEnd::CodeBlock) => {
                if in_code_block {
                    // The End event's range covers the full code block (same as Start).
                    // Use range.end which is the byte after the closing fence line.
                    let end = range.end;
                    // Also skip trailing newline after the closing fence if present.
                    let actual_end = if content.as_bytes().get(end) == Some(&b'\n') {
                        end + 1
                    } else {
                        end
                    };
                    excluded.push((code_block_start, actual_end));
                    in_code_block = false;
                }
            }

            // YAML frontmatter: exclude the entire metadata block including delimiters.
            Event::Start(Tag::MetadataBlock(_)) => {
                in_metadata_block = true;
                metadata_block_start = range.start;
            }
            Event::End(TagEnd::MetadataBlock(_)) => {
                if in_metadata_block {
                    let end = range.end;
                    // Skip trailing newline after closing ---
                    let actual_end = if content.as_bytes().get(end) == Some(&b'\n') {
                        end + 1
                    } else {
                        end
                    };
                    excluded.push((metadata_block_start, actual_end));
                    in_metadata_block = false;
                }
            }

            // Inline code: exclude the range (includes the backticks).
            Event::Code(_) => {
                excluded.push((range.start, range.end));
            }

            // HTML blocks: check if they contain comment markers.
            Event::Html(text) => {
                if text.contains("<!--") {
                    excluded.push((range.start, range.end));
                }
            }

            // Inline HTML: check for comment markers too.
            Event::InlineHtml(text) => {
                if text.contains("<!--") || text.contains("-->") {
                    excluded.push((range.start, range.end));
                }
            }

            _ => {}
        }
    }

    // Also scan for inline HTML comments that pulldown-cmark might report across
    // multiple InlineHtml events (opening `<!--` and closing `-->`). We do a
    // manual pass to catch `<!-- ... -->` that spans within a single line but
    // might not be caught as an HtmlBlock.
    let mut search_start = 0;
    while let Some(comment_open) = content[search_start..].find("<!--") {
        let abs_open = search_start + comment_open;
        if let Some(comment_close) = content[abs_open..].find("-->") {
            let abs_close = abs_open + comment_close + 3; // 3 for "-->"
            excluded.push((abs_open, abs_close));
            search_start = abs_close;
        } else {
            // Unclosed comment: exclude from open to end of content
            excluded.push((abs_open, content.len()));
            break;
        }
    }

    // Sort excluded ranges by start position and merge overlapping ones.
    excluded.sort_by_key(|&(start, _)| start);
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in excluded {
        if let Some(last) = merged.last_mut() {
            if start <= last.1 {
                // Overlapping or adjacent: extend the current merged range.
                last.1 = last.1.max(end);
                continue;
            }
        }
        merged.push((start, end));
    }

    // Compute the complement: ranges NOT in any excluded region.
    let mut body_ranges: Vec<(usize, usize)> = Vec::new();
    let mut cursor = 0;
    for (ex_start, ex_end) in &merged {
        if cursor < *ex_start {
            body_ranges.push((cursor, *ex_start));
        }
        cursor = *ex_end;
    }
    if cursor < content.len() {
        body_ranges.push((cursor, content.len()));
    }

    body_ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: given body_text_ranges output and the original content, concatenate
    /// the body text slices into a single String for easy assertion.
    fn body_text(content: &str) -> String {
        body_text_ranges(content)
            .iter()
            .map(|&(start, end)| &content[start..end])
            .collect::<Vec<_>>()
            .join("")
    }

    #[test]
    fn normal_text_returns_full_range() {
        let content = "Hello world\n";
        let ranges = body_text_ranges(content);
        assert_eq!(ranges, vec![(0, content.len())]);
    }

    #[test]
    fn empty_content_returns_empty() {
        let ranges = body_text_ranges("");
        assert!(ranges.is_empty());
    }

    #[test]
    fn fenced_code_block_excluded() {
        let content = "Before\n\n```rust\nlet x = 1;\n```\n\nAfter\n";
        let text = body_text(content);
        assert!(
            !text.contains("let x = 1;"),
            "Code block content should be excluded, got: {:?}",
            text
        );
        assert!(
            !text.contains("```"),
            "Code fence markers should be excluded, got: {:?}",
            text
        );
        assert!(text.contains("Before"), "Text before code block should be included");
        assert!(text.contains("After"), "Text after code block should be included");
    }

    #[test]
    fn inline_code_excluded() {
        let content = "Some `inline` code\n";
        let text = body_text(content);
        assert!(
            !text.contains("`inline`"),
            "Inline code should be excluded, got: {:?}",
            text
        );
        assert!(text.contains("Some "), "Text before inline code should be included");
        assert!(text.contains(" code"), "Text after inline code should be included");
    }

    #[test]
    fn html_comment_excluded() {
        let content = "Before\n\n<!-- comment -->\n\nAfter\n";
        let text = body_text(content);
        assert!(
            !text.contains("<!-- comment -->"),
            "HTML comment should be excluded, got: {:?}",
            text
        );
        assert!(
            !text.contains("comment"),
            "Comment content should be excluded, got: {:?}",
            text
        );
        assert!(text.contains("Before"), "Text before comment should be included");
        assert!(text.contains("After"), "Text after comment should be included");
    }

    #[test]
    fn yaml_frontmatter_excluded() {
        let content = "---\ntitle: test\n---\n\nBody text\n";
        let text = body_text(content);
        assert!(
            !text.contains("title: test"),
            "Frontmatter should be excluded, got: {:?}",
            text
        );
        assert!(
            !text.contains("---"),
            "Frontmatter delimiters should be excluded, got: {:?}",
            text
        );
        assert!(text.contains("Body text"), "Body text should be included");
    }

    #[test]
    fn mixed_content_works_correctly() {
        let content = "---\ntitle: doc\n---\n\n# Heading\n\nSome text with `code` here.\n\n```python\nprint('hi')\n```\n\n<!-- hidden -->\n\nFinal paragraph.\n";
        let text = body_text(content);

        // Excluded content should not appear
        assert!(!text.contains("title: doc"), "Frontmatter should be excluded");
        assert!(!text.contains("`code`"), "Inline code should be excluded");
        assert!(!text.contains("print('hi')"), "Code block should be excluded");
        assert!(!text.contains("<!-- hidden -->"), "HTML comment should be excluded");

        // Included content should appear
        assert!(text.contains("# Heading"), "Heading should be included");
        assert!(text.contains("Some text with "), "Body text should be included");
        assert!(text.contains(" here."), "Body text after inline code should be included");
        assert!(text.contains("Final paragraph."), "Final paragraph should be included");
    }

    #[test]
    fn inline_html_comment_excluded() {
        let content = "Text <!-- comment --> more text\n";
        let text = body_text(content);
        assert!(
            !text.contains("<!-- comment -->"),
            "Inline HTML comment should be excluded, got: {:?}",
            text
        );
        assert!(text.contains("Text "), "Text before inline comment should be included");
        assert!(text.contains(" more text"), "Text after inline comment should be included");
    }

    #[test]
    fn multiple_code_blocks_excluded() {
        let content = "A\n\n```\nblock1\n```\n\nB\n\n```\nblock2\n```\n\nC\n";
        let text = body_text(content);
        assert!(!text.contains("block1"), "First code block should be excluded");
        assert!(!text.contains("block2"), "Second code block should be excluded");
        assert!(text.contains("A"), "Text A should be included");
        assert!(text.contains("B"), "Text B should be included");
        assert!(text.contains("C"), "Text C should be included");
    }

    #[test]
    fn ranges_are_sorted_and_non_overlapping() {
        let content = "---\nfm\n---\n\nA `b` c\n\n```\ncode\n```\n\n<!-- x -->\n\nEnd\n";
        let ranges = body_text_ranges(content);
        for window in ranges.windows(2) {
            assert!(
                window[0].1 <= window[1].0,
                "Ranges should be non-overlapping and sorted: {:?} overlaps {:?}",
                window[0],
                window[1]
            );
        }
    }

    // ── validate_syntax tests ──────────────────────────────────────────

    fn run_validate(content: &str) -> Vec<Diagnostic> {
        validate_syntax(Path::new("test.md"), content)
    }

    // Valid syntax (no diagnostics)

    #[test]
    fn validate_valid_wikilink_no_diagnostics() {
        let diags = run_validate("See [[My Page]] for details.");
        assert!(diags.is_empty());
    }

    #[test]
    fn validate_multiple_wikilinks_no_diagnostics() {
        let diags = run_validate("Link to [[Page A]] and [[Page B]].");
        assert!(diags.is_empty());
    }

    #[test]
    fn validate_wikilink_with_alias_no_diagnostics() {
        let diags = run_validate("See [[Page|display text]] here.");
        assert!(diags.is_empty());
    }

    #[test]
    fn validate_wikilink_with_heading_no_diagnostics() {
        let diags = run_validate("Go to [[Page#Section]].");
        assert!(diags.is_empty());
    }

    // Unclosed brackets

    #[test]
    fn validate_unclosed_wikilink_no_closing() {
        let diags = run_validate("Check [[page name");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].level, DiagnosticLevel::Error);
        assert!(diags[0].message.contains("Unclosed wikilink"));
        assert!(diags[0].message.contains("[[page name"));
        assert_eq!(diags[0].line, 1);
        assert_eq!(diags[0].column, 7); // "Check " is 6 chars, [[ starts at col 7
    }

    #[test]
    fn validate_unclosed_wikilink_single_bracket_close() {
        let diags = run_validate("See [[page name]");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].level, DiagnosticLevel::Error);
        assert!(diags[0].message.contains("Unclosed wikilink"));
        assert_eq!(diags[0].line, 1);
        assert_eq!(diags[0].column, 5);
    }

    #[test]
    fn validate_unclosed_wikilink_on_second_line() {
        let content = "Line one is fine.\n[[broken link";
        let diags = run_validate(content);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].line, 2);
        assert_eq!(diags[0].column, 1);
    }

    // Empty links

    #[test]
    fn validate_empty_wikilink() {
        let diags = run_validate("An empty [[]] link.");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].level, DiagnosticLevel::Error);
        assert_eq!(diags[0].message, "Empty wikilink: '[[]]'");
        assert_eq!(diags[0].line, 1);
        assert_eq!(diags[0].column, 10);
    }

    #[test]
    fn validate_multiple_empty_wikilinks() {
        let diags = run_validate("[[]] and [[]]");
        assert_eq!(diags.len(), 2);
        assert!(diags.iter().all(|d| d.message == "Empty wikilink: '[[]]'"));
        assert_eq!(diags[0].column, 1);
        assert_eq!(diags[1].column, 10);
    }

    // Nested brackets

    #[test]
    fn validate_nested_brackets() {
        let diags = run_validate("[[page [[nested]]]]");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].level, DiagnosticLevel::Error);
        assert_eq!(diags[0].message, "Nested brackets in wikilink");
        assert_eq!(diags[0].line, 1);
        assert_eq!(diags[0].column, 1);
    }

    #[test]
    fn validate_nested_brackets_on_specific_line() {
        let content = "normal text\n[[outer [[inner]]]]";
        let diags = run_validate(content);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].line, 2);
        assert_eq!(diags[0].column, 1);
    }

    // Fenced code block skipping

    #[test]
    fn validate_skips_fenced_code_block_backticks() {
        let content = "```\n[[broken\n```";
        let diags = run_validate(content);
        assert!(diags.is_empty(), "Should skip content inside fenced code blocks");
    }

    #[test]
    fn validate_skips_fenced_code_block_tildes() {
        let content = "~~~\n[[]]\n~~~";
        let diags = run_validate(content);
        assert!(diags.is_empty(), "Should skip content inside ~~~ fenced code blocks");
    }

    #[test]
    fn validate_fenced_code_block_with_language() {
        let content = "```rust\n[[broken\n```";
        let diags = run_validate(content);
        assert!(diags.is_empty());
    }

    #[test]
    fn validate_reports_errors_after_code_block_ends() {
        let content = "```\n[[inside]]\n```\n[[broken";
        let diags = run_validate(content);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].line, 4);
        assert!(diags[0].message.contains("Unclosed wikilink"));
    }

    // Inline code skipping

    #[test]
    fn validate_skips_inline_code() {
        let diags = run_validate("Use `[[broken` syntax.");
        assert!(diags.is_empty(), "Should skip wikilink patterns inside inline code");
    }

    #[test]
    fn validate_reports_after_inline_code() {
        let diags = run_validate("`code` then [[broken");
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("Unclosed wikilink"));
    }

    // Mixed scenarios

    #[test]
    fn validate_multiple_errors_different_lines() {
        let content = "[[unclosed\n[[]]\n[[a [[b]]]]";
        let diags = run_validate(content);
        assert_eq!(diags.len(), 3);

        assert_eq!(diags[0].line, 1);
        assert!(diags[0].message.contains("Unclosed wikilink"));

        assert_eq!(diags[1].line, 2);
        assert!(diags[1].message.contains("Empty wikilink"));

        assert_eq!(diags[2].line, 3);
        assert!(diags[2].message.contains("Nested brackets"));
    }

    #[test]
    fn validate_valid_and_invalid_on_same_line() {
        let diags = run_validate("[[good]] then [[bad");
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("Unclosed wikilink"));
        assert_eq!(diags[0].column, 15);
    }

    #[test]
    fn validate_file_path_is_preserved() {
        let p = Path::new("/vault/notes/test.md");
        let diags = validate_syntax(p, "[[broken");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].file, PathBuf::from("/vault/notes/test.md"));
    }

    #[test]
    fn validate_empty_content_no_diagnostics() {
        let diags = run_validate("");
        assert!(diags.is_empty());
    }

    #[test]
    fn validate_no_wikilinks_no_diagnostics() {
        let diags = run_validate("Just plain text with [single brackets].");
        assert!(diags.is_empty());
    }

    #[test]
    fn validate_unclosed_with_single_close_bracket() {
        let diags = run_validate("[[page name] more text");
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("Unclosed wikilink"));
    }

    // ── extract_wikilinks tests ──────────────────────────────────────────

    #[test]
    fn wikilink_basic_link() {
        let links = extract_wikilinks("See [[Page Name]] for details.");
        assert_eq!(links.len(), 1);
        let l = &links[0];
        assert_eq!(l.target_page, "Page Name");
        assert_eq!(l.raw_target, "Page Name");
        assert_eq!(l.heading, None);
        assert_eq!(l.block_ref, None);
        assert_eq!(l.alias, None);
        assert!(!l.is_embed);
        assert_eq!(l.line, 1);
        assert_eq!(l.column, 5);
    }

    #[test]
    fn wikilink_with_alias() {
        let links = extract_wikilinks("[[Page Name|Display Text]]");
        assert_eq!(links.len(), 1);
        let l = &links[0];
        assert_eq!(l.target_page, "Page Name");
        assert_eq!(l.raw_target, "Page Name");
        assert_eq!(l.alias, Some("Display Text".to_string()));
        assert_eq!(l.heading, None);
        assert_eq!(l.block_ref, None);
        assert!(!l.is_embed);
    }

    #[test]
    fn wikilink_heading_link() {
        let links = extract_wikilinks("[[Page Name#Section One]]");
        assert_eq!(links.len(), 1);
        let l = &links[0];
        assert_eq!(l.target_page, "Page Name");
        assert_eq!(l.raw_target, "Page Name#Section One");
        assert_eq!(l.heading, Some("Section One".to_string()));
        assert_eq!(l.block_ref, None);
        assert_eq!(l.alias, None);
    }

    #[test]
    fn wikilink_heading_with_alias() {
        let links = extract_wikilinks("[[Page Name#Heading|Alias]]");
        assert_eq!(links.len(), 1);
        let l = &links[0];
        assert_eq!(l.target_page, "Page Name");
        assert_eq!(l.raw_target, "Page Name#Heading");
        assert_eq!(l.heading, Some("Heading".to_string()));
        assert_eq!(l.alias, Some("Alias".to_string()));
    }

    #[test]
    fn wikilink_block_reference() {
        let links = extract_wikilinks("[[Page Name^abc-123]]");
        assert_eq!(links.len(), 1);
        let l = &links[0];
        assert_eq!(l.target_page, "Page Name");
        assert_eq!(l.raw_target, "Page Name^abc-123");
        assert_eq!(l.heading, None);
        assert_eq!(l.block_ref, Some("abc-123".to_string()));
    }

    #[test]
    fn wikilink_embed() {
        let links = extract_wikilinks("![[Image.png]]");
        assert_eq!(links.len(), 1);
        let l = &links[0];
        assert_eq!(l.target_page, "Image.png");
        assert!(l.is_embed);
        assert_eq!(l.line, 1);
        assert_eq!(l.column, 1);
    }

    #[test]
    fn wikilink_self_reference_heading() {
        let links = extract_wikilinks("[[#Heading]]");
        assert_eq!(links.len(), 1);
        let l = &links[0];
        assert_eq!(l.target_page, "");
        assert_eq!(l.raw_target, "#Heading");
        assert_eq!(l.heading, Some("Heading".to_string()));
    }

    #[test]
    fn wikilink_multiple_links_multiline() {
        let content = "First [[Alpha]]\nSecond [[Beta#H1|alias]] end\nThird ![[Gamma^blk]]";
        let links = extract_wikilinks(content);
        assert_eq!(links.len(), 3);

        assert_eq!(links[0].target_page, "Alpha");
        assert_eq!(links[0].line, 1);
        assert_eq!(links[0].column, 7);

        assert_eq!(links[1].target_page, "Beta");
        assert_eq!(links[1].heading, Some("H1".to_string()));
        assert_eq!(links[1].alias, Some("alias".to_string()));
        assert_eq!(links[1].line, 2);
        assert_eq!(links[1].column, 8);

        assert_eq!(links[2].target_page, "Gamma");
        assert_eq!(links[2].block_ref, Some("blk".to_string()));
        assert!(links[2].is_embed);
        assert_eq!(links[2].line, 3);
        assert_eq!(links[2].column, 7);
    }

    #[test]
    fn wikilink_no_links() {
        let links = extract_wikilinks("Just some text with no links.");
        assert!(links.is_empty());
    }

    #[test]
    fn wikilink_alias_split_on_first_pipe() {
        let links = extract_wikilinks("[[Page|first|second]]");
        assert_eq!(links.len(), 1);
        let l = &links[0];
        assert_eq!(l.target_page, "Page");
        assert_eq!(l.alias, Some("first|second".to_string()));
    }

    #[test]
    fn wikilink_block_ref_with_alias() {
        let links = extract_wikilinks("[[Page^block-1|show this]]");
        assert_eq!(links.len(), 1);
        let l = &links[0];
        assert_eq!(l.target_page, "Page");
        assert_eq!(l.block_ref, Some("block-1".to_string()));
        assert_eq!(l.alias, Some("show this".to_string()));
    }

    #[test]
    fn wikilink_empty_no_match() {
        let links = extract_wikilinks("[[]]");
        assert!(links.is_empty());
    }

    #[test]
    fn wikilink_adjacent_links() {
        let links = extract_wikilinks("[[A]][[B]]");
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].target_page, "A");
        assert_eq!(links[0].column, 1);
        assert_eq!(links[1].target_page, "B");
        assert_eq!(links[1].column, 6);
    }

    #[test]
    fn wikilink_embed_on_second_line() {
        let content = "line one\n![[embed]]";
        let links = extract_wikilinks(content);
        assert_eq!(links.len(), 1);
        assert!(links[0].is_embed);
        assert_eq!(links[0].line, 2);
        assert_eq!(links[0].column, 1);
    }

    #[test]
    fn wikilink_target_page_trimmed() {
        let links = extract_wikilinks("[[  Spaced Page  ]]");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target_page, "Spaced Page");
    }

    #[test]
    fn wikilink_self_reference_heading_with_alias() {
        let links = extract_wikilinks("[[#My Heading|click here]]");
        assert_eq!(links.len(), 1);
        let l = &links[0];
        assert_eq!(l.target_page, "");
        assert_eq!(l.heading, Some("My Heading".to_string()));
        assert_eq!(l.alias, Some("click here".to_string()));
    }

    #[test]
    fn wikilink_embed_with_heading() {
        let links = extract_wikilinks("![[Note#Section]]");
        assert_eq!(links.len(), 1);
        let l = &links[0];
        assert_eq!(l.target_page, "Note");
        assert_eq!(l.heading, Some("Section".to_string()));
        assert!(l.is_embed);
    }

    #[test]
    fn wikilink_exclamation_not_adjacent() {
        let links = extract_wikilinks("! [[Page]]");
        assert_eq!(links.len(), 1);
        assert!(!links[0].is_embed);
    }

    // ── normalize_page_name tests ────────────────────────────────────────

    #[test]
    fn normalize_lowercases() {
        assert_eq!(normalize_page_name("My Page"), "my page");
    }

    #[test]
    fn normalize_hyphens_to_spaces() {
        assert_eq!(normalize_page_name("my-page"), "my page");
    }

    #[test]
    fn normalize_underscores_to_spaces() {
        assert_eq!(normalize_page_name("my_page"), "my page");
    }

    #[test]
    fn normalize_mixed_separators() {
        assert_eq!(normalize_page_name("My-Page_Name"), "my page name");
    }

    #[test]
    fn normalize_already_normalized() {
        assert_eq!(normalize_page_name("my page"), "my page");
    }

    #[test]
    fn normalize_empty_string() {
        assert_eq!(normalize_page_name(""), "");
    }

    #[test]
    fn normalize_preserves_other_chars() {
        assert_eq!(normalize_page_name("page (v2.0)"), "page (v2.0)");
    }

    // ── resolve_page_name tests ──────────────────────────────────────────

    fn make_index(entries: &[(&str, &str)]) -> Vec<(String, PathBuf)> {
        entries
            .iter()
            .map(|(name, path)| (name.to_string(), PathBuf::from(path)))
            .collect()
    }

    // -- Exact match --

    #[test]
    fn resolve_exact_match_same_case() {
        let index = make_index(&[
            ("Zettelkasten Method", "Zettelkasten Method.md"),
            ("Rust Programming", "Rust Programming.md"),
        ]);
        let result = resolve_page_name("Zettelkasten Method", &index);
        assert_eq!(result, Some("Zettelkasten Method".to_string()));
    }

    #[test]
    fn resolve_exact_match_case_insensitive() {
        let index = make_index(&[
            ("Zettelkasten Method", "Zettelkasten Method.md"),
            ("Rust Programming", "Rust Programming.md"),
        ]);
        let result = resolve_page_name("zettelkasten method", &index);
        assert_eq!(result, Some("Zettelkasten Method".to_string()));
    }

    #[test]
    fn resolve_exact_match_mixed_case() {
        let index = make_index(&[("My Page", "My Page.md")]);
        let result = resolve_page_name("MY PAGE", &index);
        assert_eq!(result, Some("My Page".to_string()));
    }

    // -- Normalized match --

    #[test]
    fn resolve_normalized_hyphen_to_space() {
        let index = make_index(&[("my page", "my page.md")]);
        let result = resolve_page_name("my-page", &index);
        assert_eq!(result, Some("my page".to_string()));
    }

    #[test]
    fn resolve_normalized_underscore_to_space() {
        let index = make_index(&[("my page", "my page.md")]);
        let result = resolve_page_name("my_page", &index);
        assert_eq!(result, Some("my page".to_string()));
    }

    #[test]
    fn resolve_normalized_space_to_hyphen() {
        let index = make_index(&[("my-page", "my-page.md")]);
        let result = resolve_page_name("my page", &index);
        assert_eq!(result, Some("my-page".to_string()));
    }

    #[test]
    fn resolve_normalized_mixed_separators() {
        let index = make_index(&[("my_page", "my_page.md")]);
        let result = resolve_page_name("My-Page", &index);
        assert_eq!(result, Some("my_page".to_string()));
    }

    #[test]
    fn resolve_normalized_all_equivalent() {
        let index = make_index(&[("my-page", "my-page.md")]);
        assert_eq!(
            resolve_page_name("my_page", &index),
            Some("my-page".to_string())
        );
        assert_eq!(
            resolve_page_name("my page", &index),
            Some("my-page".to_string())
        );
        assert_eq!(
            resolve_page_name("My Page", &index),
            Some("my-page".to_string())
        );
    }

    // -- Path-qualified match --

    #[test]
    fn resolve_path_qualified_match() {
        let index = make_index(&[
            ("Meeting Notes", "work/Meeting Notes.md"),
            ("Daily Log", "personal/Daily Log.md"),
        ]);
        let result = resolve_page_name("work/Meeting Notes", &index);
        assert_eq!(result, Some("Meeting Notes".to_string()));
    }

    #[test]
    fn resolve_path_qualified_case_insensitive() {
        let index = make_index(&[("Meeting Notes", "work/Meeting Notes.md")]);
        let result = resolve_page_name("Work/meeting notes", &index);
        assert_eq!(result, Some("Meeting Notes".to_string()));
    }

    #[test]
    fn resolve_path_qualified_nested() {
        let index = make_index(&[("Design Doc", "projects/alpha/Design Doc.md")]);
        let result = resolve_page_name("projects/alpha/Design Doc", &index);
        assert_eq!(result, Some("Design Doc".to_string()));
    }

    #[test]
    fn resolve_path_qualified_no_match() {
        let index = make_index(&[("Meeting Notes", "work/Meeting Notes.md")]);
        let result = resolve_page_name("personal/Meeting Notes", &index);
        assert_eq!(result, None);
    }

    #[test]
    fn resolve_path_qualified_normalized() {
        let index = make_index(&[("Meeting Notes", "work/Meeting_Notes.md")]);
        let result = resolve_page_name("work/Meeting-Notes", &index);
        assert_eq!(result, Some("Meeting Notes".to_string()));
    }

    // -- Ambiguous matches --

    #[test]
    fn resolve_ambiguous_exact_match() {
        let index = make_index(&[
            ("notes", "work/notes.md"),
            ("Notes", "personal/Notes.md"),
        ]);
        let result = resolve_page_name("notes", &index);
        assert_eq!(result, None);
    }

    #[test]
    fn resolve_ambiguous_normalized_match() {
        let index = make_index(&[
            ("my-page", "my-page.md"),
            ("my_page", "my_page.md"),
        ]);
        let result = resolve_page_name("my page", &index);
        assert_eq!(result, None);
    }

    #[test]
    fn resolve_ambiguous_path_qualified() {
        let index = make_index(&[
            ("Page A", "notes/Page A.md"),
            ("Page A", "notes/Page A.md"),
        ]);
        let result = resolve_page_name("notes/Page A", &index);
        assert_eq!(result, None);
    }

    // -- No match --

    #[test]
    fn resolve_no_match_at_all() {
        let index = make_index(&[
            ("Zettelkasten Method", "Zettelkasten Method.md"),
            ("Rust Programming", "Rust Programming.md"),
        ]);
        let result = resolve_page_name("Nonexistent Page", &index);
        assert_eq!(result, None);
    }

    #[test]
    fn resolve_no_match_empty_index() {
        let index: Vec<(String, PathBuf)> = vec![];
        let result = resolve_page_name("anything", &index);
        assert_eq!(result, None);
    }

    #[test]
    fn resolve_no_match_empty_raw() {
        let index = make_index(&[("Page", "Page.md")]);
        let result = resolve_page_name("", &index);
        assert_eq!(result, None);
    }

    // -- Priority / precedence --

    #[test]
    fn resolve_exact_match_takes_priority_over_normalized() {
        let index = make_index(&[("my page", "my page.md")]);
        let result = resolve_page_name("my page", &index);
        assert_eq!(result, Some("my page".to_string()));
    }

    #[test]
    fn resolve_exact_before_path_qualified() {
        let index = make_index(&[
            ("notes/Page", "notes/Page.md"),
            ("Page", "other/Page.md"),
        ]);
        let result = resolve_page_name("Page", &index);
        assert_eq!(result, Some("Page".to_string()));
    }

    #[test]
    fn resolve_path_qualified_disambiguates() {
        let index = make_index(&[
            ("README", "docs/README.md"),
            ("README", "src/README.md"),
        ]);
        let result = resolve_page_name("docs/README", &index);
        assert_eq!(result, Some("README".to_string()));
    }

    #[test]
    fn resolve_slash_triggers_path_match_when_needed() {
        let index = make_index(&[(
            "Status Report",
            "work/projects/Status Report.md",
        )]);
        let result = resolve_page_name("work/projects/Status Report", &index);
        assert_eq!(result, Some("Status Report".to_string()));
    }

    // -- Edge cases --

    #[test]
    fn resolve_single_entry_index() {
        let index = make_index(&[("Solo Page", "Solo Page.md")]);
        assert_eq!(
            resolve_page_name("Solo Page", &index),
            Some("Solo Page".to_string())
        );
        assert_eq!(
            resolve_page_name("solo page", &index),
            Some("Solo Page".to_string())
        );
        assert_eq!(
            resolve_page_name("Solo-Page", &index),
            Some("Solo Page".to_string())
        );
        assert_eq!(resolve_page_name("Other Page", &index), None);
    }

    #[test]
    fn resolve_page_name_with_special_characters() {
        let index = make_index(&[("C++ Guide", "C++ Guide.md")]);
        assert_eq!(
            resolve_page_name("c++ guide", &index),
            Some("C++ Guide".to_string())
        );
    }

    #[test]
    fn resolve_path_without_md_extension() {
        let index = make_index(&[("notes", "notes")]);
        let result = resolve_page_name("notes", &index);
        assert_eq!(result, Some("notes".to_string()));
    }

    #[test]
    fn resolve_path_qualified_strips_md_extension() {
        let index = make_index(&[("Design", "docs/Design.md")]);
        let result = resolve_page_name("docs/Design", &index);
        assert_eq!(result, Some("Design".to_string()));
    }

    #[test]
    fn resolve_multiple_unique_pages_no_ambiguity() {
        let index = make_index(&[
            ("Alpha", "Alpha.md"),
            ("Beta", "Beta.md"),
            ("Gamma", "Gamma.md"),
        ]);
        assert_eq!(
            resolve_page_name("alpha", &index),
            Some("Alpha".to_string())
        );
        assert_eq!(
            resolve_page_name("BETA", &index),
            Some("Beta".to_string())
        );
        assert_eq!(
            resolve_page_name("gamma", &index),
            Some("Gamma".to_string())
        );
    }
}
