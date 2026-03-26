use crate::merkle::{
    compute_file_root, compute_leaf_hash, compute_spl_hashes, detect_sections, spl_combined_hash,
};
use crate::types::{
    ContentHash, Diagnostic, DiagnosticLevel, FileMerkle, LeafType, MerkleLeaf, ParsedFile,
    SplBlock, SplLeafCached, WikiLink,
};
use anyhow::Result;
use ignore::WalkBuilder;
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use regex::Regex;
use std::ops::Range;
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
        overrides.add(&format!("!{pattern}"))?;
    }
    builder.overrides(overrides.build()?);

    // Skip subdirectories that are nested vaults (contain their own .zetl/).
    // This prevents a parent vault from absorbing pages that belong to a
    // child vault, similar to how git ignores nested git repos.
    builder.filter_entry(|entry| {
        if entry.depth() > 0
            && entry.file_type().is_some_and(|ft| ft.is_dir())
            && entry.path().join(".zetl").is_dir()
        {
            return false;
        }
        true
    });

    let mut parsed_files = Vec::new();

    for entry in builder.build() {
        let entry = entry?;
        let path = entry.path();

        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str());

        match ext {
            Some("md") => {
                let rel_path = path.strip_prefix(root).unwrap_or(path).to_path_buf();
                let page_name = page_name_from_path(&rel_path);
                let content = std::fs::read_to_string(path)?;
                let mtime = std::fs::metadata(path)?.modified()?;

                let mut parsed = parse_file(&rel_path, &content, &page_name);
                parsed.mtime = mtime;
                parsed_files.push(parsed);
            }
            Some("fountain") => {
                let rel_path = path.strip_prefix(root).unwrap_or(path).to_path_buf();
                let page_name = page_name_from_path(&rel_path);
                let content = std::fs::read_to_string(path)?;
                let mtime = std::fs::metadata(path)?.modified()?;

                let mut parsed = parse_file(&rel_path, &content, &page_name);
                parsed.mtime = mtime;
                parsed_files.push(parsed);
            }
            Some("spl") => {
                let rel_path = path.strip_prefix(root).unwrap_or(path).to_path_buf();
                let page_name = page_name_from_path(&rel_path);
                let content = std::fs::read_to_string(path)?;
                let mtime = std::fs::metadata(path)?.modified()?;

                let end_line = content.lines().count().max(1) as u32;
                let spl_block = SplBlock {
                    source_file: rel_path.clone(),
                    source_page: page_name.clone(),
                    start_line: 1,
                    end_line,
                    content: content.clone(),
                };

                let parsed = ParsedFile {
                    path: rel_path,
                    page_name,
                    links: vec![],
                    spl_blocks: vec![spl_block],
                    diagnostics: vec![],
                    mtime,
                    merkle_leaves: vec![],
                    file_merkle: None,
                };
                parsed_files.push(parsed);
            }
            _ => continue,
        }
    }

    Ok(parsed_files)
}

/// Parse wikilinks from markdown content, respecting code blocks and comments.
///
/// Composes `body_text_ranges`, `extract_wikilinks`, `extract_spl_blocks`,
/// `build_merkle_leaves`, and `validate_syntax` to produce a complete `ParsedFile`
/// with only links from body text, extracted SPL blocks, Merkle leaf nodes, and all
/// syntax diagnostics.
///
/// The pulldown-cmark event stream is collected once and passed to `build_merkle_leaves`
/// per §6.4 (no second parse pass for Merkle leaf construction).
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

    // Extract SPL blocks and apply page-level sandbox (REQ-020-059)
    let spl_blocks = {
        #[allow(unused_mut)]
        let mut blocks = extract_spl_blocks(path, content, page_name);
        #[cfg(feature = "reason")]
        {
            for block in &mut blocks {
                crate::acl::sandbox_page_spl(block);
            }
            // Drop blocks that were fully rejected (emptied by sandbox)
            blocks.retain(|b| !b.content.trim().is_empty());
        }
        blocks
    };

    // Collect events once for Merkle leaf construction (§6.4: same parse pass).
    // Enable GFM-compatible extensions so that tables and strikethrough are parsed.
    let mut options = Options::empty();
    options.insert(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    let events: Vec<(Event<'_>, Range<usize>)> = Parser::new_ext(content, options)
        .into_offset_iter()
        .collect();

    // Build Merkle leaves from the collected event stream.
    let merkle_leaves = build_merkle_leaves(content, &events);

    // Compute per-file Merkle root from the ordered leaf hashes (§4.2).
    let file_root = compute_file_root(&merkle_leaves);

    // Detect grounding sections for all SplBlock leaves (§4.5).
    let mut sections = detect_sections(&merkle_leaves);

    // Fill in heading_text from the raw file content using each section's heading_line.
    for section in &mut sections {
        if section.heading_level > 0 {
            section.heading_text = extract_heading_text(content, section.heading_line);
        }
    }

    // Build the spl_leaves Vec: one SplLeafCached per SplBlock leaf in document order.
    // Build a mapping from spl-leaf position (0-indexed) → section index.
    let mut spl_to_section = Vec::new();
    for (sec_idx, sec) in sections.iter().enumerate() {
        for spl_pos in sec.leaf_range.0..sec.leaf_range.1 {
            // Ensure the vec is large enough (spl positions arrive in order).
            if spl_to_section.len() <= spl_pos {
                spl_to_section.resize(spl_pos + 1, 0usize);
            }
            spl_to_section[spl_pos] = sec_idx;
        }
    }

    let mut spl_leaves: Vec<SplLeafCached> = Vec::new();
    for leaf in &merkle_leaves {
        if matches!(leaf.node_type, LeafType::SplBlock) {
            let spl_pos = spl_leaves.len();
            let section_index = spl_to_section.get(spl_pos).copied().unwrap_or(0);
            let (content_hash, ast_hash) = match &leaf.spl_hashes {
                Some(h) => (h.content_hash, h.ast_hash),
                None => (leaf.hash, [0u8; 32]),
            };
            spl_leaves.push(SplLeafCached {
                start_line: leaf.start_line,
                content_hash,
                ast_hash,
                section_index,
                explicit_groundings: vec![],
            });
        }
    }

    let file_merkle = Some(FileMerkle {
        root_hash: file_root,
        sections,
        spl_leaves,
    });

    // Validate syntax
    let diagnostics = validate_syntax(path, content);

    // When the `reason` feature is enabled, emit a warning for any SPL block
    // whose AST could not be parsed (signalled by ast_hash == [0u8; 32]).
    #[cfg(feature = "reason")]
    let diagnostics = {
        let mut d = diagnostics;
        for leaf in &merkle_leaves {
            if matches!(leaf.node_type, LeafType::SplBlock) {
                if let Some(h) = &leaf.spl_hashes {
                    if h.ast_hash == [0u8; 32] {
                        d.push(Diagnostic {
                            level: DiagnosticLevel::Warning,
                            message: "SPL parse failed; ast_hash set to sentinel [0u8; 32]"
                                .to_string(),
                            file: path.to_path_buf(),
                            line: leaf.start_line,
                            column: 0,
                        });
                    }
                }
            }
        }
        d
    };

    ParsedFile {
        path: path.to_path_buf(),
        page_name: page_name.to_string(),
        links,
        spl_blocks,
        diagnostics,
        mtime: std::time::SystemTime::UNIX_EPOCH, // caller sets real mtime
        merkle_leaves,
        file_merkle,
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
                    let snippet: String =
                        chars[open_start..len.min(open_start + 40)].iter().collect();
                    let display = if snippet.len() < (len - open_start) {
                        format!("{snippet}...")
                    } else {
                        snippet
                    };
                    diagnostics.push(Diagnostic {
                        level: DiagnosticLevel::Error,
                        message: format!("Unclosed wikilink: '{display}'"),
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

/// Derive a URL slug from a relative vault path.
///
/// Strips the `.md`/`.spl` extension, lowercases, and replaces spaces with hyphens
/// so URLs are clean kebab-case paths.
///
/// Example: `architecture/Scanner.md` → `architecture/scanner`
/// Example: `concepts/Defeasible Reasoning.md` → `concepts/defeasible-reasoning`
pub fn page_slug_from_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    let stripped = if let Some(s) = s.strip_suffix(".md") {
        s
    } else if let Some(s) = s.strip_suffix(".spl") {
        s
    } else if let Some(s) = s.strip_suffix(".fountain") {
        s
    } else {
        &s
    };
    stripped.to_lowercase().replace(' ', "-")
}

/// Extract an Obsidian block-id annotation from normalised leaf text (REQ-042b).
///
/// Obsidian block IDs are written as ` ^identifier` at the end of a block's
/// content, where `identifier` consists solely of ASCII alphanumeric characters
/// and hyphens.  Returns `Some(identifier)` if the pattern is found at the
/// tail of `normalized_text`, `None` otherwise.
pub(crate) fn extract_block_id(normalized_text: &str) -> Option<String> {
    // Locate the last " ^" sequence.
    let caret_pos = normalized_text.rfind(" ^")?;
    let identifier = &normalized_text[caret_pos + 2..]; // skip the " ^"
                                                        // Identifier must be non-empty and only contain alphanumerics / hyphens.
    if !identifier.is_empty()
        && identifier
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        Some(identifier.to_string())
    } else {
        None
    }
}

/// Extract SPL blocks from markdown content.
///
/// Identifies fenced code blocks tagged `spl` or `spindle` (both backtick and tilde fences)
/// and captures their content with provenance. Blocks inside HTML comments are ignored
/// (consistent with SPEC-001 §3.3). Blocks inside nested fences are naturally handled by
/// pulldown-cmark which only recognises top-level fences.
///
/// Returns blocks in document order.
pub fn extract_spl_blocks(path: &Path, content: &str, page_name: &str) -> Vec<SplBlock> {
    if content.is_empty() {
        return vec![];
    }

    // Pre-compute HTML comment ranges so we can skip SPL blocks inside them.
    let comment_ranges = html_comment_ranges(content);

    // Pre-compute line start byte offsets for byte-offset → line-number conversion.
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(content.match_indices('\n').map(|(i, _)| i + 1))
        .collect();

    let byte_to_line = |byte_offset: usize| -> u32 {
        let idx = line_starts.partition_point(|&start| start <= byte_offset);
        idx as u32 // partition_point returns count of elements ≤, which equals 1-indexed line
    };

    let mut options = Options::empty();
    options.insert(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);

    let parser = Parser::new_ext(content, options);

    let mut blocks = Vec::new();
    let mut in_spl_block = false;
    let mut spl_content = String::new();
    let mut block_start_byte: usize = 0;

    for (event, range) in parser.into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(ref info))) => {
                let tag = info.split_whitespace().next().unwrap_or("");
                if tag == "spl" || tag == "spindle" {
                    // Check if this code block is inside an HTML comment
                    let inside_comment = comment_ranges
                        .iter()
                        .any(|&(cs, ce)| range.start >= cs && range.start < ce);
                    if !inside_comment {
                        in_spl_block = true;
                        spl_content.clear();
                        block_start_byte = range.start;
                    }
                }
            }
            Event::Text(ref text) if in_spl_block => {
                spl_content.push_str(text);
            }
            Event::End(TagEnd::CodeBlock) if in_spl_block => {
                let start_line = byte_to_line(block_start_byte);
                let end_line = byte_to_line(range.end.saturating_sub(1));

                blocks.push(SplBlock {
                    source_file: path.to_path_buf(),
                    source_page: page_name.to_string(),
                    start_line,
                    end_line,
                    content: spl_content.clone(),
                });
                in_spl_block = false;
                spl_content.clear();
            }
            _ => {}
        }
    }

    blocks
}

/// Group pulldown-cmark events into block-level [`MerkleLeaf`] nodes.
///
/// Processes the event stream produced by a single pulldown-cmark parse pass and maps
/// each top-level block element to a [`MerkleLeaf`] per SPEC-006 §4.3. The function is
/// designed to be called with events collected during the same parse pass used for
/// wikilink and SPL extraction (§6.4 — no second parse pass).
///
/// **Leaf type mapping** (§4.3):
/// - `Start(Heading)..End(Heading)` → `Heading { level }`
/// - `Start(Paragraph)..End(Paragraph)` → `Paragraph`
/// - `Start(CodeBlock(Fenced("spl"|"spindle")))..End(CodeBlock)` → `SplBlock` (flagged for dual hashing)
/// - `Start(CodeBlock(..))..End(CodeBlock)` → `CodeBlock { language }`
/// - `Start(List)..End(List)` → `List { ordered }`
/// - `Start(BlockQuote)..End(BlockQuote)` → `BlockQuote`
/// - `Start(Table)..End(Table)` → `Table`
/// - `Start(MetadataBlock)..End(MetadataBlock)` → `Frontmatter`
/// - `Rule` → `ThematicBreak`
/// - `Html(..)` (block-level) → `HtmlBlock`
///
/// **Normalisation** (§4.3):
/// - Line endings normalised to `\n`
/// - Consecutive whitespace collapsed to a single space
/// - Leading/trailing whitespace trimmed
/// - Bold/italic/strikethrough markers stripped (they are structural events, not text)
/// - `[[wikilinks]]` preserved (appear as raw `Text` events in pulldown-cmark)
/// - `[text](url)` links reconstructed into their markdown form
/// - `![alt](url)` images reconstructed into their markdown form
/// - Case preserved
///
/// For code blocks and frontmatter, the raw text is used unchanged (whitespace is
/// significant in code). For all other block types, the normalised text is used.
pub fn build_merkle_leaves<'a>(
    content: &str,
    events: &[(Event<'a>, Range<usize>)],
) -> Vec<MerkleLeaf> {
    if content.is_empty() || events.is_empty() {
        return vec![];
    }

    // Pre-compute line start byte offsets for byte → 1-indexed line number conversion.
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(content.match_indices('\n').map(|(i, _)| i + 1))
        .collect();

    let byte_to_line = |byte_offset: usize| -> u32 {
        // partition_point returns the count of elements ≤ byte_offset, which equals
        // the 1-indexed line number.
        line_starts.partition_point(|&start| start <= byte_offset) as u32
    };

    // Hash a leaf: BLAKE3(type_tag_byte ‖ content_bytes) per §4.2.
    // For ThematicBreak and HtmlBlock (standalone events), we need a type-aware wrapper.
    let hash_leaf = |lt: &LeafType, input: &[u8]| -> ContentHash { compute_leaf_hash(lt, input) };

    // Normalise body text: collapse all whitespace to single space, trim.
    // Line endings are already handled (SoftBreak→space, HardBreak→\n which then collapses).
    let normalize = |s: &str| -> String {
        let mut out = String::with_capacity(s.len());
        let mut prev_space = false;
        for ch in s.chars() {
            if ch.is_whitespace() {
                if !prev_space {
                    out.push(' ');
                }
                prev_space = true;
            } else {
                out.push(ch);
                prev_space = false;
            }
        }
        // Trim by stripping a single leading/trailing space produced by the loop.
        let trimmed = out.trim();
        trimmed.to_string()
    };

    let mut leaves: Vec<MerkleLeaf> = Vec::new();

    // ── State machine variables ───────────────────────────────────────────────
    // `current_leaf_type` being Some(_) indicates we are inside a block.
    let mut current_leaf_type: Option<LeafType> = None;
    let mut current_start_byte: usize = 0;
    let mut current_depth: usize = 0; // block-nesting depth; 0 = top-level
    let mut text_buf: String = String::new(); // normalised body text for hash
    let mut raw_buf: String = String::new(); // raw content (code / frontmatter / HTML)
                                             // Stack of link/image URLs for reconstructing `[text](url)` / `![alt](url)`.
    let mut link_url_stack: Vec<String> = Vec::new();

    // Helper closure: build and push a MerkleLeaf, then reset mutable state.
    // Because we cannot use a closure that mutates multiple captured vars, we inline
    // the finalization logic inline in the End-event branch below.

    for (event, range) in events {
        if current_leaf_type.is_none() {
            // ── Top-level: waiting for a block to start ───────────────────────
            match event {
                Event::Start(tag) => {
                    let leaf_type = match tag {
                        Tag::Heading { level, .. } => {
                            let lvl: u8 = match level {
                                HeadingLevel::H1 => 1,
                                HeadingLevel::H2 => 2,
                                HeadingLevel::H3 => 3,
                                HeadingLevel::H4 => 4,
                                HeadingLevel::H5 => 5,
                                HeadingLevel::H6 => 6,
                            };
                            Some(LeafType::Heading { level: lvl })
                        }
                        Tag::Paragraph => Some(LeafType::Paragraph),
                        Tag::CodeBlock(CodeBlockKind::Fenced(info)) => {
                            let lang_tag = info.split_whitespace().next().unwrap_or("");
                            if lang_tag == "spl" || lang_tag == "spindle" {
                                Some(LeafType::SplBlock)
                            } else {
                                let language = if info.is_empty() || lang_tag.is_empty() {
                                    None
                                } else {
                                    Some(lang_tag.to_string())
                                };
                                Some(LeafType::CodeBlock { language })
                            }
                        }
                        Tag::CodeBlock(CodeBlockKind::Indented) => {
                            Some(LeafType::CodeBlock { language: None })
                        }
                        Tag::List(ordered) => Some(LeafType::List {
                            ordered: ordered.is_some(),
                        }),
                        Tag::BlockQuote(_) => Some(LeafType::BlockQuote),
                        Tag::Table(_) => Some(LeafType::Table),
                        Tag::MetadataBlock(_) => Some(LeafType::Frontmatter),
                        _ => None, // sub-block tags (Item, TableRow, …) — shouldn't appear at depth 0
                    };
                    if let Some(lt) = leaf_type {
                        current_leaf_type = Some(lt);
                        current_start_byte = range.start;
                        current_depth = 1;
                        text_buf.clear();
                        raw_buf.clear();
                        link_url_stack.clear();
                    }
                }

                // Thematic break — standalone event, no Start/End wrapper.
                Event::Rule => {
                    let start_line = byte_to_line(range.start);
                    let end_line = byte_to_line(range.end.saturating_sub(1)).max(start_line);
                    leaves.push(MerkleLeaf {
                        node_type: LeafType::ThematicBreak,
                        start_line,
                        end_line,
                        hash: hash_leaf(&LeafType::ThematicBreak, b"---"),
                        spl_hashes: None,
                        block_id: None,
                    });
                }

                // Block-level HTML — `Event::Html` (as opposed to `Event::InlineHtml`).
                Event::Html(html) => {
                    let start_line = byte_to_line(range.start);
                    let end_line = byte_to_line(range.end.saturating_sub(1)).max(start_line);
                    leaves.push(MerkleLeaf {
                        node_type: LeafType::HtmlBlock,
                        start_line,
                        end_line,
                        hash: hash_leaf(&LeafType::HtmlBlock, html.as_bytes()),
                        spl_hashes: None,
                        block_id: None,
                    });
                }

                _ => {} // other standalone events at the top level (soft/hard breaks between blocks, etc.)
            }
        } else {
            // ── Inside a block: accumulate content and track nesting depth ────
            match event {
                Event::Start(tag) => {
                    match tag {
                        // Links: push `[` and remember the destination URL.
                        Tag::Link { dest_url, .. } => {
                            text_buf.push('[');
                            link_url_stack.push(dest_url.to_string());
                            // Do NOT increment depth — links are inline and their End
                            // event is handled specially (it does not decrement depth).
                        }
                        // Images: push `![` and remember the destination URL.
                        Tag::Image { dest_url, .. } => {
                            text_buf.push_str("![");
                            link_url_stack.push(dest_url.to_string());
                            // Do NOT increment depth (same reason as Link).
                        }
                        // All other Start events (block-level sub-elements like Item,
                        // TableRow, TableCell, and inline like Strong, Emphasis,
                        // Strikethrough) increment the nesting counter.
                        // Bold/italic/strikethrough markers are intentionally NOT
                        // written to text_buf — only Text events are collected.
                        _ => {
                            current_depth += 1;
                        }
                    }
                }

                Event::End(tag_end) => {
                    match tag_end {
                        // Close a link: append `](url)`.
                        TagEnd::Link => {
                            let url = link_url_stack.pop().unwrap_or_default();
                            text_buf.push_str(&format!("]({url})"));
                            // Do NOT decrement depth.
                        }
                        // Close an image: append `](url)`.
                        TagEnd::Image => {
                            let url = link_url_stack.pop().unwrap_or_default();
                            text_buf.push_str(&format!("]({url})"));
                            // Do NOT decrement depth.
                        }
                        _ => {
                            current_depth -= 1;
                            if current_depth == 0 {
                                // The outermost block has closed — finalise the leaf.
                                let end_byte = range.end;
                                let start_line = byte_to_line(current_start_byte);
                                let end_line =
                                    byte_to_line(end_byte.saturating_sub(1)).max(start_line);

                                let leaf_type = current_leaf_type.take().unwrap();

                                // Pre-compute normalised body text once so we can
                                // both use it in the hash and inspect it for block IDs.
                                let normalized_text = normalize(&text_buf);

                                // Extract Obsidian block-id annotation (REQ-042b).
                                // Only applicable to body-text leaf types; code/SPL/
                                // frontmatter/HTML blocks cannot carry block IDs.
                                let block_id: Option<String> = match &leaf_type {
                                    LeafType::Paragraph
                                    | LeafType::Heading { .. }
                                    | LeafType::List { .. }
                                    | LeafType::BlockQuote
                                    | LeafType::Table => extract_block_id(&normalized_text),
                                    _ => None,
                                };

                                // Compute hash input per §4.3.
                                let hash_input: String = match &leaf_type {
                                    LeafType::Heading { level } => {
                                        format!("{}{}", level, &normalized_text)
                                    }
                                    LeafType::Paragraph => normalized_text.clone(),
                                    LeafType::List { ordered } => {
                                        let flag = if *ordered { "1" } else { "0" };
                                        format!("{}{}", flag, &normalized_text)
                                    }
                                    LeafType::BlockQuote => normalized_text.clone(),
                                    LeafType::Table => normalized_text.clone(),
                                    // Code blocks: language tag + raw content (whitespace-significant).
                                    LeafType::CodeBlock { language } => {
                                        let lang = language.as_deref().unwrap_or("");
                                        format!("{lang}\n{raw_buf}")
                                    }
                                    // SPL: raw content hash (dual AST hash is a separate pass).
                                    LeafType::SplBlock => raw_buf.clone(),
                                    // Frontmatter: raw YAML text.
                                    LeafType::Frontmatter => raw_buf.clone(),
                                    // Shouldn't reach these via the End branch, but handle for completeness.
                                    LeafType::ThematicBreak => "---".to_string(),
                                    LeafType::HtmlBlock => raw_buf.clone(),
                                };

                                // SplBlock leaves use dual hashing (§4.4):
                                // combined hash = BLAKE3(content_hash ‖ ast_hash).
                                // All other leaves use BLAKE3(type_tag ‖ content).
                                if matches!(leaf_type, LeafType::SplBlock) {
                                    let spl = compute_spl_hashes(&raw_buf);
                                    let combined = spl_combined_hash(&spl);
                                    leaves.push(MerkleLeaf {
                                        node_type: leaf_type,
                                        start_line,
                                        end_line,
                                        hash: combined,
                                        spl_hashes: Some(spl),
                                        block_id: None,
                                    });
                                } else {
                                    let hash = hash_leaf(&leaf_type, hash_input.as_bytes());
                                    leaves.push(MerkleLeaf {
                                        node_type: leaf_type,
                                        start_line,
                                        end_line,
                                        hash,
                                        spl_hashes: None,
                                        block_id,
                                    });
                                }

                                // Reset accumulators for the next leaf.
                                text_buf.clear();
                                raw_buf.clear();
                                link_url_stack.clear();
                            }
                        }
                    }
                }

                // Plain text — the primary content carrier.
                // For code blocks and frontmatter, accumulate into raw_buf (preserves whitespace).
                // For all other leaf types, accumulate into text_buf (will be normalised).
                Event::Text(t) => match &current_leaf_type {
                    Some(LeafType::CodeBlock { .. })
                    | Some(LeafType::SplBlock)
                    | Some(LeafType::Frontmatter) => {
                        raw_buf.push_str(t);
                    }
                    _ => {
                        text_buf.push_str(t);
                    }
                },

                // Inline code span — preserve with backtick delimiters.
                Event::Code(code) => {
                    text_buf.push('`');
                    text_buf.push_str(code);
                    text_buf.push('`');
                }

                // Soft break (source newline within a paragraph) → single space.
                Event::SoftBreak => {
                    text_buf.push(' ');
                }

                // Hard break (two trailing spaces or `\` at end of line) → newline,
                // which the normaliser will later collapse to a space.
                Event::HardBreak => {
                    text_buf.push('\n');
                }

                // Inline HTML inside a block — include as-is in text_buf so it
                // affects the hash (change to inline HTML is a content change).
                Event::InlineHtml(html) => {
                    text_buf.push_str(html);
                }

                _ => {} // rule, block Html, etc. cannot appear inside a block
            }
        }
    }

    leaves
}

/// Extract heading text from a specific 1-indexed line in the file content.
///
/// Handles both ATX headings (`## Title`) by stripping leading `#` characters
/// and the optional following space, and setext headings where the text is the
/// entire line content.
fn extract_heading_text(content: &str, line: u32) -> String {
    let line_content = content
        .lines()
        .nth((line as usize).saturating_sub(1))
        .unwrap_or("");
    if line_content.starts_with('#') {
        // ATX heading: strip leading '#' chars and one optional space.
        let stripped = line_content.trim_start_matches('#');
        stripped
            .strip_prefix(' ')
            .unwrap_or(stripped)
            .trim()
            .to_string()
    } else {
        // Setext heading: the text is the full line.
        line_content.trim().to_string()
    }
}

/// Find byte ranges of HTML comments (`<!-- ... -->`) in content.
fn html_comment_ranges(content: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut search_start = 0;
    while let Some(open_offset) = content[search_start..].find("<!--") {
        let abs_open = search_start + open_offset;
        if let Some(close_offset) = content[abs_open..].find("-->") {
            let abs_close = abs_open + close_offset + 3;
            ranges.push((abs_open, abs_close));
            search_start = abs_close;
        } else {
            // Unclosed comment extends to end
            ranges.push((abs_open, content.len()));
            break;
        }
    }
    ranges
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
            "Code block content should be excluded, got: {text:?}"
        );
        assert!(
            !text.contains("```"),
            "Code fence markers should be excluded, got: {text:?}"
        );
        assert!(
            text.contains("Before"),
            "Text before code block should be included"
        );
        assert!(
            text.contains("After"),
            "Text after code block should be included"
        );
    }

    #[test]
    fn inline_code_excluded() {
        let content = "Some `inline` code\n";
        let text = body_text(content);
        assert!(
            !text.contains("`inline`"),
            "Inline code should be excluded, got: {text:?}"
        );
        assert!(
            text.contains("Some "),
            "Text before inline code should be included"
        );
        assert!(
            text.contains(" code"),
            "Text after inline code should be included"
        );
    }

    #[test]
    fn html_comment_excluded() {
        let content = "Before\n\n<!-- comment -->\n\nAfter\n";
        let text = body_text(content);
        assert!(
            !text.contains("<!-- comment -->"),
            "HTML comment should be excluded, got: {text:?}"
        );
        assert!(
            !text.contains("comment"),
            "Comment content should be excluded, got: {text:?}"
        );
        assert!(
            text.contains("Before"),
            "Text before comment should be included"
        );
        assert!(
            text.contains("After"),
            "Text after comment should be included"
        );
    }

    #[test]
    fn yaml_frontmatter_excluded() {
        let content = "---\ntitle: test\n---\n\nBody text\n";
        let text = body_text(content);
        assert!(
            !text.contains("title: test"),
            "Frontmatter should be excluded, got: {text:?}"
        );
        assert!(
            !text.contains("---"),
            "Frontmatter delimiters should be excluded, got: {text:?}"
        );
        assert!(text.contains("Body text"), "Body text should be included");
    }

    #[test]
    fn mixed_content_works_correctly() {
        let content = "---\ntitle: doc\n---\n\n# Heading\n\nSome text with `code` here.\n\n```python\nprint('hi')\n```\n\n<!-- hidden -->\n\nFinal paragraph.\n";
        let text = body_text(content);

        // Excluded content should not appear
        assert!(
            !text.contains("title: doc"),
            "Frontmatter should be excluded"
        );
        assert!(!text.contains("`code`"), "Inline code should be excluded");
        assert!(
            !text.contains("print('hi')"),
            "Code block should be excluded"
        );
        assert!(
            !text.contains("<!-- hidden -->"),
            "HTML comment should be excluded"
        );

        // Included content should appear
        assert!(text.contains("# Heading"), "Heading should be included");
        assert!(
            text.contains("Some text with "),
            "Body text should be included"
        );
        assert!(
            text.contains(" here."),
            "Body text after inline code should be included"
        );
        assert!(
            text.contains("Final paragraph."),
            "Final paragraph should be included"
        );
    }

    #[test]
    fn inline_html_comment_excluded() {
        let content = "Text <!-- comment --> more text\n";
        let text = body_text(content);
        assert!(
            !text.contains("<!-- comment -->"),
            "Inline HTML comment should be excluded, got: {text:?}"
        );
        assert!(
            text.contains("Text "),
            "Text before inline comment should be included"
        );
        assert!(
            text.contains(" more text"),
            "Text after inline comment should be included"
        );
    }

    #[test]
    fn multiple_code_blocks_excluded() {
        let content = "A\n\n```\nblock1\n```\n\nB\n\n```\nblock2\n```\n\nC\n";
        let text = body_text(content);
        assert!(
            !text.contains("block1"),
            "First code block should be excluded"
        );
        assert!(
            !text.contains("block2"),
            "Second code block should be excluded"
        );
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
        assert!(
            diags.is_empty(),
            "Should skip content inside fenced code blocks"
        );
    }

    #[test]
    fn validate_skips_fenced_code_block_tildes() {
        let content = "~~~\n[[]]\n~~~";
        let diags = run_validate(content);
        assert!(
            diags.is_empty(),
            "Should skip content inside ~~~ fenced code blocks"
        );
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
        assert!(
            diags.is_empty(),
            "Should skip wikilink patterns inside inline code"
        );
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
        let index = make_index(&[("notes", "work/notes.md"), ("Notes", "personal/Notes.md")]);
        let result = resolve_page_name("notes", &index);
        assert_eq!(result, None);
    }

    #[test]
    fn resolve_ambiguous_normalized_match() {
        let index = make_index(&[("my-page", "my-page.md"), ("my_page", "my_page.md")]);
        let result = resolve_page_name("my page", &index);
        assert_eq!(result, None);
    }

    #[test]
    fn resolve_ambiguous_path_qualified() {
        let index = make_index(&[("Page A", "notes/Page A.md"), ("Page A", "notes/Page A.md")]);
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
        let index = make_index(&[("notes/Page", "notes/Page.md"), ("Page", "other/Page.md")]);
        let result = resolve_page_name("Page", &index);
        assert_eq!(result, Some("Page".to_string()));
    }

    #[test]
    fn resolve_path_qualified_disambiguates() {
        let index = make_index(&[("README", "docs/README.md"), ("README", "src/README.md")]);
        let result = resolve_page_name("docs/README", &index);
        assert_eq!(result, Some("README".to_string()));
    }

    #[test]
    fn resolve_slash_triggers_path_match_when_needed() {
        let index = make_index(&[("Status Report", "work/projects/Status Report.md")]);
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
        assert_eq!(resolve_page_name("BETA", &index), Some("Beta".to_string()));
        assert_eq!(
            resolve_page_name("gamma", &index),
            Some("Gamma".to_string())
        );
    }

    // ── extract_spl_blocks tests ──────────────────────────────────────────

    fn run_extract_spl(content: &str) -> Vec<SplBlock> {
        extract_spl_blocks(Path::new("test.md"), content, "test")
    }

    #[test]
    fn spl_no_blocks() {
        let blocks = run_extract_spl("Just plain text.");
        assert!(blocks.is_empty());
    }

    #[test]
    fn spl_empty_content() {
        let blocks = run_extract_spl("");
        assert!(blocks.is_empty());
    }

    #[test]
    fn spl_single_backtick_block() {
        let content = "Before\n\n```spl\n(given foo)\n```\n\nAfter\n";
        let blocks = run_extract_spl(content);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].content, "(given foo)\n");
        assert_eq!(blocks[0].source_file, PathBuf::from("test.md"));
        assert_eq!(blocks[0].source_page, "test");
        assert_eq!(blocks[0].start_line, 3);
        assert_eq!(blocks[0].end_line, 5);
    }

    #[test]
    fn spl_single_tilde_block() {
        let content = "Before\n\n~~~spl\n(given bar)\n~~~\n\nAfter\n";
        let blocks = run_extract_spl(content);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].content, "(given bar)\n");
    }

    #[test]
    fn spl_spindle_tag() {
        let content = "```spindle\n(normally r1 a b)\n```\n";
        let blocks = run_extract_spl(content);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].content, "(normally r1 a b)\n");
    }

    #[test]
    fn spl_multiple_blocks_document_order() {
        let content = "\
Before

```spl
(given alpha)
```

Middle

```spl
(given beta)
```

After
";
        let blocks = run_extract_spl(content);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].content, "(given alpha)\n");
        assert_eq!(blocks[1].content, "(given beta)\n");
        assert!(blocks[0].start_line < blocks[1].start_line);
    }

    #[test]
    fn spl_non_spl_code_block_ignored() {
        let content = "```rust\nlet x = 1;\n```\n\n```spl\n(given y)\n```\n";
        let blocks = run_extract_spl(content);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].content, "(given y)\n");
    }

    #[test]
    fn spl_plain_code_block_ignored() {
        let content = "```\nplain code\n```\n";
        let blocks = run_extract_spl(content);
        assert!(blocks.is_empty());
    }

    #[test]
    fn spl_inside_html_comment_ignored() {
        let content = "\
<!--
```spl
(given hidden)
```
-->

```spl
(given visible)
```
";
        let blocks = run_extract_spl(content);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].content, "(given visible)\n");
    }

    #[test]
    fn spl_multiline_content() {
        let content = "\
```spl
(given evaluated-redis)
(given redis-supports-persistence)
(normally r-prefer-redis
  (and evaluated-redis redis-supports-persistence)
  decided-use-redis)
```
";
        let blocks = run_extract_spl(content);
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].content.contains("evaluated-redis"));
        assert!(blocks[0].content.contains("decided-use-redis"));
    }

    #[test]
    fn spl_info_string_with_extra_text() {
        // pulldown-cmark passes the full info string; we match on the first word
        let content = "```spl some-extra-info\n(given x)\n```\n";
        let blocks = run_extract_spl(content);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].content, "(given x)\n");
    }

    #[test]
    fn spl_with_frontmatter() {
        let content = "---\ntitle: test\n---\n\n```spl\n(given fact)\n```\n";
        let blocks = run_extract_spl(content);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].content, "(given fact)\n");
    }

    #[test]
    fn spl_mixed_spl_and_spindle() {
        let content = "\
```spl
(given a)
```

```spindle
(given b)
```
";
        let blocks = run_extract_spl(content);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].content, "(given a)\n");
        assert_eq!(blocks[1].content, "(given b)\n");
    }

    #[test]
    fn spl_line_numbers_correct() {
        let content = "line 1\nline 2\nline 3\n\n```spl\n(given test)\n```\nline 8\n";
        let blocks = run_extract_spl(content);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].start_line, 5);
        assert_eq!(blocks[0].end_line, 7);
    }

    #[test]
    fn spl_parse_file_populates_spl_blocks() {
        let content = "See [[Page]].\n\n```spl\n(given fact)\n```\n";
        let parsed = parse_file(Path::new("notes/test.md"), content, "test");
        assert_eq!(parsed.spl_blocks.len(), 1);
        assert_eq!(parsed.spl_blocks[0].content, "(given fact)\n");
        assert_eq!(parsed.links.len(), 1);
    }

    // ── scan_vault integration tests for .spl files ───────────────────────

    #[test]
    fn scan_vault_finds_spl_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let spl_dir = dir.path().join("theories");
        std::fs::create_dir_all(&spl_dir).unwrap();

        let spl_content = "; caching theory\n(given eval-redis)\n";
        std::fs::write(spl_dir.join("caching.spl"), spl_content).unwrap();

        let files = scan_vault(dir.path(), &[]).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].page_name, "caching");
        assert_eq!(files[0].spl_blocks.len(), 1);
        assert_eq!(files[0].spl_blocks[0].start_line, 1);
        assert_eq!(files[0].spl_blocks[0].end_line, 2);
        assert_eq!(files[0].spl_blocks[0].content, spl_content);
        assert!(files[0].links.is_empty());
    }

    #[test]
    fn scan_vault_finds_both_md_and_spl_files() {
        let dir = tempfile::TempDir::new().unwrap();

        let md_content = "# Decision\n\n```spl\n(given x)\n```\n";
        std::fs::write(dir.path().join("decision.md"), md_content).unwrap();

        let spl_content = "(given y)\n";
        std::fs::write(dir.path().join("theory.spl"), spl_content).unwrap();

        let files = scan_vault(dir.path(), &[]).unwrap();
        assert_eq!(files.len(), 2);

        let md_file = files.iter().find(|f| f.page_name == "decision").unwrap();
        assert_eq!(md_file.spl_blocks.len(), 1);

        let spl_file = files.iter().find(|f| f.page_name == "theory").unwrap();
        assert_eq!(spl_file.spl_blocks.len(), 1);
        assert_eq!(spl_file.spl_blocks[0].content, spl_content);
    }

    // ── build_merkle_leaves tests ─────────────────────────────────────────

    /// Helper: parse content and return leaves (mirrors the options used in `parse_file`).
    fn parse_leaves(content: &str) -> Vec<crate::types::MerkleLeaf> {
        use pulldown_cmark::{Event, Options, Parser};
        use std::ops::Range;

        let mut opts = Options::empty();
        opts.insert(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);
        opts.insert(Options::ENABLE_TABLES);
        opts.insert(Options::ENABLE_STRIKETHROUGH);
        let events: Vec<(Event<'_>, Range<usize>)> =
            Parser::new_ext(content, opts).into_offset_iter().collect();
        build_merkle_leaves(content, &events)
    }

    /// Return the variant name of a LeafType for concise assertions.
    fn leaf_variant(leaf: &crate::types::MerkleLeaf) -> &'static str {
        match &leaf.node_type {
            crate::types::LeafType::Heading { .. } => "Heading",
            crate::types::LeafType::Paragraph => "Paragraph",
            crate::types::LeafType::CodeBlock { .. } => "CodeBlock",
            crate::types::LeafType::SplBlock => "SplBlock",
            crate::types::LeafType::List { .. } => "List",
            crate::types::LeafType::BlockQuote => "BlockQuote",
            crate::types::LeafType::Table => "Table",
            crate::types::LeafType::Frontmatter => "Frontmatter",
            crate::types::LeafType::ThematicBreak => "ThematicBreak",
            crate::types::LeafType::HtmlBlock => "HtmlBlock",
        }
    }

    // ── Empty / trivial content ───────────────────────────────────────────

    #[test]
    fn leaves_empty_content_returns_empty() {
        let leaves = parse_leaves("");
        assert!(leaves.is_empty());
    }

    #[test]
    fn leaves_whitespace_only_returns_empty() {
        let leaves = parse_leaves("   \n\n  \n");
        assert!(leaves.is_empty());
    }

    // ── Heading ───────────────────────────────────────────────────────────

    #[test]
    fn leaves_heading_h1() {
        let leaves = parse_leaves("# Hello World\n");
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaf_variant(&leaves[0]), "Heading");
        if let crate::types::LeafType::Heading { level } = &leaves[0].node_type {
            assert_eq!(*level, 1);
        }
        assert_eq!(leaves[0].start_line, 1);
        assert_eq!(leaves[0].end_line, 1);
    }

    #[test]
    fn leaves_heading_h3() {
        let leaves = parse_leaves("### Sub section\n");
        assert_eq!(leaves.len(), 1);
        if let crate::types::LeafType::Heading { level } = &leaves[0].node_type {
            assert_eq!(*level, 3);
        }
    }

    #[test]
    fn leaves_heading_hash_changes_for_different_levels() {
        let l1 = parse_leaves("# Heading\n");
        let l2 = parse_leaves("## Heading\n");
        // Level is included in the hash input, so hashes must differ.
        assert_ne!(l1[0].hash, l2[0].hash);
    }

    // ── Paragraph ────────────────────────────────────────────────────────

    #[test]
    fn leaves_paragraph() {
        let leaves = parse_leaves("Hello, world.\n");
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaf_variant(&leaves[0]), "Paragraph");
        assert_eq!(leaves[0].start_line, 1);
    }

    #[test]
    fn leaves_paragraph_hash_stable_across_whitespace() {
        // Extra trailing space / single vs double blank line separating paras.
        let a = parse_leaves("Hello world.\n");
        let b = parse_leaves("Hello  world.\n"); // extra space inside
                                                 // Consecutive whitespace is collapsed → hashes must match.
        assert_eq!(a[0].hash, b[0].hash);
    }

    #[test]
    fn leaves_paragraph_hash_differs_on_content_change() {
        let a = parse_leaves("Hello world.\n");
        let b = parse_leaves("Hello earth.\n");
        assert_ne!(a[0].hash, b[0].hash);
    }

    // ── Code blocks ───────────────────────────────────────────────────────

    #[test]
    fn leaves_fenced_code_block_rust() {
        let content = "```rust\nlet x = 1;\n```\n";
        let leaves = parse_leaves(content);
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaf_variant(&leaves[0]), "CodeBlock");
        if let crate::types::LeafType::CodeBlock { language } = &leaves[0].node_type {
            assert_eq!(language.as_deref(), Some("rust"));
        }
    }

    #[test]
    fn leaves_fenced_code_block_no_language() {
        let leaves = parse_leaves("```\nplain code\n```\n");
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaf_variant(&leaves[0]), "CodeBlock");
        if let crate::types::LeafType::CodeBlock { language } = &leaves[0].node_type {
            assert!(language.is_none());
        }
    }

    #[test]
    fn leaves_spl_block() {
        let content = "```spl\n(given foo)\n```\n";
        let leaves = parse_leaves(content);
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaf_variant(&leaves[0]), "SplBlock");
        assert!(leaves[0].spl_hashes.is_some()); // dual hashing done during build_merkle_leaves
    }

    #[test]
    fn leaves_spindle_block() {
        let content = "```spindle\n(normally r1 a b)\n```\n";
        let leaves = parse_leaves(content);
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaf_variant(&leaves[0]), "SplBlock");
    }

    #[test]
    fn leaves_spl_hash_differs_from_same_content_code_block() {
        // Same content, different fence tag → different leaf type → different hash.
        let spl = parse_leaves("```spl\n(given foo)\n```\n");
        let rust = parse_leaves("```rust\n(given foo)\n```\n");
        assert_ne!(spl[0].hash, rust[0].hash);
    }

    #[test]
    fn leaves_code_block_language_affects_hash() {
        let rust = parse_leaves("```rust\nfoo\n```\n");
        let py = parse_leaves("```python\nfoo\n```\n");
        assert_ne!(rust[0].hash, py[0].hash);
    }

    // ── List ─────────────────────────────────────────────────────────────

    #[test]
    fn leaves_unordered_list() {
        let leaves = parse_leaves("- item one\n- item two\n");
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaf_variant(&leaves[0]), "List");
        if let crate::types::LeafType::List { ordered } = &leaves[0].node_type {
            assert!(!ordered);
        }
    }

    #[test]
    fn leaves_ordered_list() {
        let leaves = parse_leaves("1. first\n2. second\n");
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaf_variant(&leaves[0]), "List");
        if let crate::types::LeafType::List { ordered } = &leaves[0].node_type {
            assert!(ordered);
        }
    }

    #[test]
    fn leaves_ordered_vs_unordered_hash_differ() {
        let ordered = parse_leaves("1. item\n");
        let unordered = parse_leaves("- item\n");
        // The ordered flag is included in the hash input.
        assert_ne!(ordered[0].hash, unordered[0].hash);
    }

    // ── BlockQuote ────────────────────────────────────────────────────────

    #[test]
    fn leaves_block_quote() {
        let leaves = parse_leaves("> a quoted paragraph\n");
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaf_variant(&leaves[0]), "BlockQuote");
    }

    // ── Table ─────────────────────────────────────────────────────────────

    #[test]
    fn leaves_table() {
        let content = "| A | B |\n|---|---|\n| 1 | 2 |\n";
        let leaves = parse_leaves(content);
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaf_variant(&leaves[0]), "Table");
    }

    // ── Frontmatter ───────────────────────────────────────────────────────

    #[test]
    fn leaves_frontmatter() {
        let content = "---\ntitle: test\ndate: 2024-01-01\n---\n\nBody text.\n";
        let leaves = parse_leaves(content);
        // Must have a Frontmatter leaf and a Paragraph leaf.
        assert!(leaves.iter().any(|l| leaf_variant(l) == "Frontmatter"));
        assert!(leaves.iter().any(|l| leaf_variant(l) == "Paragraph"));
    }

    // ── ThematicBreak ─────────────────────────────────────────────────────

    #[test]
    fn leaves_thematic_break() {
        let leaves = parse_leaves("---\n");
        // pulldown-cmark parses `---` alone as a ThematicBreak (not frontmatter).
        let has_break = leaves.iter().any(|l| leaf_variant(l) == "ThematicBreak");
        assert!(
            has_break,
            "Expected ThematicBreak leaf, got: {:?}",
            leaves.iter().map(leaf_variant).collect::<Vec<_>>()
        );
    }

    #[test]
    fn leaves_thematic_break_constant_hash() {
        // All thematic breaks hash the same sentinel.
        let a = parse_leaves("---\n");
        let b = parse_leaves("***\n");
        let breaks_a: Vec<_> = a
            .iter()
            .filter(|l| leaf_variant(l) == "ThematicBreak")
            .collect();
        let breaks_b: Vec<_> = b
            .iter()
            .filter(|l| leaf_variant(l) == "ThematicBreak")
            .collect();
        if !breaks_a.is_empty() && !breaks_b.is_empty() {
            assert_eq!(breaks_a[0].hash, breaks_b[0].hash);
        }
    }

    // ── HtmlBlock ─────────────────────────────────────────────────────────

    #[test]
    fn leaves_html_block() {
        let content = "<div class=\"note\">\nsome content\n</div>\n\n";
        let leaves = parse_leaves(content);
        assert!(
            leaves.iter().any(|l| leaf_variant(l) == "HtmlBlock"),
            "Expected HtmlBlock, got: {:?}",
            leaves.iter().map(leaf_variant).collect::<Vec<_>>()
        );
    }

    // ── Mixed document ────────────────────────────────────────────────────

    #[test]
    fn leaves_mixed_document_order() {
        let content = "\
# Title

Some paragraph.

```rust
code here
```

- list item

---

> blockquote
";
        let leaves = parse_leaves(content);
        let types: Vec<_> = leaves.iter().map(leaf_variant).collect();
        assert!(types.contains(&"Heading"), "Missing Heading");
        assert!(types.contains(&"Paragraph"), "Missing Paragraph");
        assert!(types.contains(&"CodeBlock"), "Missing CodeBlock");
        assert!(types.contains(&"List"), "Missing List");
        assert!(types.contains(&"ThematicBreak"), "Missing ThematicBreak");
        assert!(types.contains(&"BlockQuote"), "Missing BlockQuote");

        // Verify document order: Heading comes before Paragraph.
        let heading_idx = types.iter().position(|&t| t == "Heading").unwrap();
        let para_idx = types.iter().position(|&t| t == "Paragraph").unwrap();
        assert!(heading_idx < para_idx);
    }

    #[test]
    fn leaves_line_numbers_are_correct() {
        // Line 1: heading
        // Line 3: paragraph (blank line between)
        let content = "# Heading\n\nParagraph text.\n";
        let leaves = parse_leaves(content);
        assert_eq!(leaves.len(), 2);
        assert_eq!(leaves[0].start_line, 1, "Heading should start on line 1");
        assert_eq!(leaves[1].start_line, 3, "Paragraph should start on line 3");
    }

    #[test]
    fn leaves_multiline_paragraph_line_range() {
        let content = "Line one.\nLine two.\nLine three.\n";
        let leaves = parse_leaves(content);
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaves[0].start_line, 1);
        assert!(leaves[0].end_line >= 1);
    }

    // ── Normalisation: formatting markers stripped ─────────────────────────

    #[test]
    fn leaves_bold_stripped_from_hash() {
        // Bold markers should be stripped; hash should be the same as plain text.
        let bold = parse_leaves("**Hello world**\n");
        let plain = parse_leaves("Hello world\n");
        assert_eq!(
            bold[0].hash, plain[0].hash,
            "Bold should be stripped from hash input"
        );
    }

    #[test]
    fn leaves_italic_stripped_from_hash() {
        let italic = parse_leaves("_Hello world_\n");
        let plain = parse_leaves("Hello world\n");
        assert_eq!(
            italic[0].hash, plain[0].hash,
            "Italic should be stripped from hash input"
        );
    }

    // ── Normalisation: links preserved ────────────────────────────────────

    #[test]
    fn leaves_standard_link_preserved_in_hash() {
        // A paragraph with a link should hash differently from the same text without it.
        let with_link = parse_leaves("See [the page](https://example.com) for details.\n");
        let without_link = parse_leaves("See the page for details.\n");
        assert_ne!(
            with_link[0].hash, without_link[0].hash,
            "Link syntax should be preserved in hash (content change)"
        );
    }

    #[test]
    fn leaves_wikilink_preserved_in_hash() {
        // Wikilinks appear as raw Text in pulldown-cmark and are naturally preserved.
        let with_link = parse_leaves("See [[My Page]] for details.\n");
        let without_link = parse_leaves("See My Page for details.\n");
        assert_ne!(
            with_link[0].hash, without_link[0].hash,
            "[[wikilinks]] should be preserved in hash"
        );
    }

    // ── Hash consistency ──────────────────────────────────────────────────

    #[test]
    fn leaves_same_content_same_hash() {
        let a = parse_leaves("# My Heading\n");
        let b = parse_leaves("# My Heading\n");
        assert_eq!(a[0].hash, b[0].hash);
    }

    #[test]
    fn leaves_different_content_different_hash() {
        let a = parse_leaves("# Alpha\n");
        let b = parse_leaves("# Beta\n");
        assert_ne!(a[0].hash, b[0].hash);
    }

    // ── parse_file integration ────────────────────────────────────────────

    #[test]
    fn parse_file_populates_merkle_leaves() {
        let content = "# Title\n\nParagraph.\n\n```spl\n(given fact)\n```\n";
        let parsed = parse_file(Path::new("test.md"), content, "test");
        // Should have: Heading, Paragraph, SplBlock
        assert_eq!(parsed.merkle_leaves.len(), 3);
        assert_eq!(leaf_variant(&parsed.merkle_leaves[0]), "Heading");
        assert_eq!(leaf_variant(&parsed.merkle_leaves[1]), "Paragraph");
        assert_eq!(leaf_variant(&parsed.merkle_leaves[2]), "SplBlock");
    }

    #[test]
    fn parse_file_leaf_count_matches() {
        let content = "# H1\n\n## H2\n\nPara.\n";
        let parsed = parse_file(Path::new("test.md"), content, "test");
        assert_eq!(parsed.merkle_leaves.len(), 3); // H1, H2, Para
    }

    // ── extract_block_id ──────────────────────────────────────────────────

    #[test]
    fn extract_block_id_basic() {
        assert_eq!(
            extract_block_id("Hello world ^my-block"),
            Some("my-block".to_string())
        );
    }

    #[test]
    fn extract_block_id_alphanumeric() {
        assert_eq!(
            extract_block_id("Some text ^abc123"),
            Some("abc123".to_string())
        );
    }

    #[test]
    fn extract_block_id_dashes_only() {
        assert_eq!(extract_block_id("Text ^a-b-c"), Some("a-b-c".to_string()));
    }

    #[test]
    fn extract_block_id_no_annotation_returns_none() {
        assert_eq!(extract_block_id("Plain paragraph text."), None);
    }

    #[test]
    fn extract_block_id_caret_mid_sentence_not_trailing_returns_none() {
        // The " ^" must be followed by the identifier to the end of string.
        assert_eq!(extract_block_id("foo ^bar baz"), None);
    }

    #[test]
    fn extract_block_id_empty_identifier_returns_none() {
        assert_eq!(extract_block_id("Text ^"), None);
    }

    #[test]
    fn extract_block_id_invalid_chars_in_identifier_returns_none() {
        // Spaces inside the identifier are invalid.
        assert_eq!(extract_block_id("Text ^foo bar"), None);
    }

    #[test]
    fn extract_block_id_uppercase_and_digits() {
        assert_eq!(
            extract_block_id("Heading text ^Ref2024"),
            Some("Ref2024".to_string())
        );
    }

    // ── block_id extraction in build_merkle_leaves ────────────────────────

    #[test]
    fn leaf_paragraph_with_block_id() {
        let leaves = parse_leaves("Hello world ^my-block\n");
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaf_variant(&leaves[0]), "Paragraph");
        assert_eq!(leaves[0].block_id, Some("my-block".to_string()));
    }

    #[test]
    fn leaf_paragraph_without_block_id() {
        let leaves = parse_leaves("Hello world.\n");
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaves[0].block_id, None);
    }

    #[test]
    fn leaf_heading_with_block_id() {
        let leaves = parse_leaves("# My Heading ^heading-ref\n");
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaf_variant(&leaves[0]), "Heading");
        assert_eq!(leaves[0].block_id, Some("heading-ref".to_string()));
    }

    #[test]
    fn leaf_list_with_block_id() {
        // Obsidian block IDs on list items appear at the end of the list block.
        let leaves = parse_leaves("- item one\n- item two ^list-id\n");
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaf_variant(&leaves[0]), "List");
        assert_eq!(leaves[0].block_id, Some("list-id".to_string()));
    }

    #[test]
    fn leaf_code_block_no_block_id() {
        // Code blocks never have block IDs extracted.
        let leaves = parse_leaves("```rust\nfn main() {} ^not-an-id\n```\n");
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaf_variant(&leaves[0]), "CodeBlock");
        assert_eq!(leaves[0].block_id, None);
    }

    #[test]
    fn leaf_spl_block_no_block_id() {
        let leaves = parse_leaves("```spl\n(given foo)\n```\n");
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaf_variant(&leaves[0]), "SplBlock");
        assert_eq!(leaves[0].block_id, None);
    }

    #[test]
    fn leaf_thematic_break_no_block_id() {
        let leaves = parse_leaves("---\n");
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaf_variant(&leaves[0]), "ThematicBreak");
        assert_eq!(leaves[0].block_id, None);
    }

    #[test]
    fn leaf_block_id_does_not_affect_hash_differently_from_content() {
        // Two paragraphs with the same text but different block IDs should
        // produce different hashes (block_id is part of the normalized content).
        let a = parse_leaves("Same text ^id-alpha\n");
        let b = parse_leaves("Same text ^id-beta\n");
        assert_ne!(a[0].hash, b[0].hash);
    }

    #[test]
    fn leaf_block_id_preserved_and_hash_includes_it() {
        // The hash should differ from the same text without the block ID.
        let with_id = parse_leaves("Some content ^myid\n");
        let without = parse_leaves("Some content\n");
        assert_ne!(with_id[0].hash, without[0].hash);
        assert_eq!(with_id[0].block_id, Some("myid".to_string()));
        assert_eq!(without[0].block_id, None);
    }
}
