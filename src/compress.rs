use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use regex::Regex;
use std::collections::HashSet;

// ── Stop words for filler-sentence compression ──────────────────────────────

const STOP_WORDS: &[&str] = &[
    "a", "an", "the", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
    "do", "does", "did", "will", "would", "shall", "should", "may", "might", "must", "can",
    "could", "to", "of", "in", "for", "on", "with", "at", "by", "from", "as", "into", "through",
    "during", "before", "after", "above", "below", "between", "out", "off", "over", "under",
    "again", "further", "then", "once", "here", "there", "when", "where", "why", "how", "all",
    "each", "every", "both", "few", "more", "most", "other", "some", "such", "no", "nor", "not",
    "only", "own", "same", "so", "than", "too", "very", "just", "about", "also", "it", "its",
    "this", "that", "these", "those", "i", "me", "my", "we", "our", "you", "your", "he", "him",
    "his", "she", "her", "they", "them", "their", "what", "which", "who", "whom", "and", "but",
    "or", "if", "while", "although", "because", "until", "unless",
];

const SIGNAL_WORDS: &[&str] = &[
    "decided",
    "chose",
    "because",
    "instead of",
    "trade-off",
    "tradeoff",
    "trade off",
    "however",
    "important",
    "critical",
    "required",
    "must",
    "note that",
    "caveat",
    "warning",
    "migration",
    "breaking",
    "deprecated",
];

// ── Token estimation ────────────────────────────────────────────────────────

/// Estimate token count using whitespace-split word count as a proxy.
pub fn estimate_tokens(text: &str) -> usize {
    text.split_whitespace().count()
}

// ── Paragraph compression ───────────────────────────────────────────────────

/// Compress a paragraph using the tiered strategy from SPEC-026 §3.2.
///
/// - Link-bearing sentences (containing `[[wikilink]]`) are preserved verbatim.
/// - Signal sentences (containing decision markers) are preserved verbatim.
/// - Filler sentences have stop words removed.
pub fn compress_paragraph(text: &str, has_wikilink: bool) -> String {
    // If the entire paragraph has a wikilink, preserve verbatim
    if has_wikilink {
        return text.to_string();
    }

    // Split into sentences
    let sentences = split_sentences(text);
    let mut parts = Vec::new();

    for sentence in &sentences {
        let trimmed = sentence.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Check if this sentence contains a wikilink
        if trimmed.contains("[[") && trimmed.contains("]]") {
            parts.push(trimmed.to_string());
            continue;
        }

        // Check for signal words
        let lower = trimmed.to_lowercase();
        if SIGNAL_WORDS.iter().any(|w| lower.contains(w)) {
            parts.push(trimmed.to_string());
            continue;
        }

        // Filler: extract key phrases
        let compressed = extract_key_phrases(trimmed);
        if !compressed.is_empty() {
            parts.push(compressed);
        }
    }

    parts.join(" ")
}

/// Split text into sentences (crude but deterministic).
fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();

    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        current.push(chars[i]);

        // End of sentence: period/exclamation/question followed by space or end
        if (chars[i] == '.' || chars[i] == '!' || chars[i] == '?')
            && (i + 1 >= len || chars[i + 1].is_whitespace())
        {
            sentences.push(current.clone());
            current.clear();
        }

        i += 1;
    }

    if !current.trim().is_empty() {
        sentences.push(current);
    }

    sentences
}

/// Extract key phrases from a sentence by removing stop words and grouping
/// consecutive content words into phrases separated by ` — `.
///
/// A dash separator is only inserted when 2+ consecutive stop words are removed
/// (a meaningful structural gap). Single stop words between content words are
/// silently dropped to keep phrases flowing naturally.
///
/// Input:  "The system is currently running in the production environment."
/// Output: "system currently running — production environment"
fn extract_key_phrases(text: &str) -> String {
    let stop_set: HashSet<&str> = STOP_WORDS.iter().copied().collect();

    let mut phrases: Vec<String> = Vec::new();
    let mut current_phrase: Vec<&str> = Vec::new();
    let mut stop_gap: usize = 0; // consecutive stop words seen

    for word in text.split_whitespace() {
        let clean = word
            .trim_matches(|c: char| c.is_ascii_punctuation())
            .to_lowercase();

        if clean.is_empty() || stop_set.contains(clean.as_str()) {
            stop_gap += 1;
        } else {
            // Content word: decide whether to break phrase or continue
            if stop_gap >= 2 && !current_phrase.is_empty() {
                // Significant gap — flush current phrase, start new one
                phrases.push(current_phrase.join(" "));
                current_phrase.clear();
            }
            // Single stop word (or no gap) — just skip it, continue phrase
            current_phrase.push(word);
            stop_gap = 0;
        }
    }

    // Flush final phrase
    if !current_phrase.is_empty() {
        phrases.push(current_phrase.join(" "));
    }

    if phrases.is_empty() {
        return String::new();
    }

    // Strip trailing punctuation from last phrase
    if let Some(last) = phrases.last_mut() {
        *last = last.trim_end_matches(|c: char| c == '.' || c == ',' || c == ';').to_string();
    }

    phrases.join(" — ")
}

// ── Frontmatter compression ─────────────────────────────────────────────────

const INTERNAL_KEYS: &[&str] = &["zetl-internal", "zetl_internal", "cssclasses"];

/// Compress YAML frontmatter into `@ key: value` lines.
pub fn compress_frontmatter(yaml_text: &str) -> String {
    let mut lines = Vec::new();

    for line in yaml_text.lines() {
        let trimmed = line.trim();

        // Skip empty lines and YAML document markers
        if trimmed.is_empty() || trimmed == "---" {
            continue;
        }

        // Parse key: value
        if let Some(colon_pos) = trimmed.find(':') {
            let key = trimmed[..colon_pos].trim();

            // Skip internal keys
            if INTERNAL_KEYS.iter().any(|&k| k == key) {
                continue;
            }

            let value = trimmed[colon_pos + 1..].trim();

            // Handle YAML array syntax: [item1, item2]
            if value.starts_with('[') && value.ends_with(']') {
                let inner = &value[1..value.len() - 1];
                let items: Vec<&str> = inner.split(',').map(|s| s.trim()).collect();
                lines.push(format!("@ {}: {}", key, items.join(", ")));
            } else if value.is_empty() {
                // Multiline value — skip (we only handle inline for now)
                continue;
            } else {
                lines.push(format!("@ {}: {}", key, value));
            }
        }
    }

    lines.join("\n")
}

// ── Graph summary ───────────────────────────────────────────────────────────

/// Build the multi-page graph summary per SPEC-026 §3.3.
///
/// Each entry is (page_name, forward_link_targets, backlink_sources).
/// Output uses `→` for forward links and `←` for backlinks.
pub fn build_graph_summary(pages: &[(String, Vec<String>, Vec<String>)]) -> String {
    // Count unique edges
    let mut edge_set: HashSet<(String, String)> = HashSet::new();
    for (page, fwd, back) in pages {
        for target in fwd {
            edge_set.insert((page.clone(), target.clone()));
        }
        for source in back {
            edge_set.insert((source.clone(), page.clone()));
        }
    }

    let page_count = pages.len();
    let edge_count = edge_set.len();

    let mut lines = vec![format!("=== GRAPH ({page_count} pages, {edge_count} edges) ===")];

    // Sort pages alphabetically
    let mut sorted_pages: Vec<_> = pages.to_vec();
    sorted_pages.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));

    for (page, fwd, back) in &sorted_pages {
        let mut parts = Vec::new();

        // Backlinks (incoming)
        if !back.is_empty() {
            let mut sorted_back = back.clone();
            sorted_back.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
            parts.push(format!("← {}", sorted_back.join(", ")));
        }

        // Forward links (outgoing)
        if !fwd.is_empty() {
            let mut sorted_fwd = fwd.clone();
            sorted_fwd.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
            parts.push(format!("→ {}", sorted_fwd.join(", ")));
        }

        if !parts.is_empty() {
            lines.push(format!("{} {}", page, parts.join(" ")));
        } else {
            lines.push(page.clone());
        }
    }

    lines.join("\n")
}

// ── Page compression (AST walker) ───────────────────────────────────────────

/// Compress a single markdown page by walking the pulldown-cmark event stream.
///
/// - `markdown`: raw markdown text
/// - `dead_links`: set of page names that are known to be dead (phantom nodes)
///
/// Returns the compressed text representation per SPEC-026 §3.1.
pub fn compress_page(markdown: &str, dead_links: &HashSet<String>) -> String {
    let wikilink_re = Regex::new(r"(!?)\[\[([^\[\]]+)\]\]").expect("invalid wikilink regex");

    // First, extract and compress frontmatter if present
    let (frontmatter, body) = extract_frontmatter(markdown);

    let mut output = String::new();

    // Emit frontmatter
    if let Some(fm) = frontmatter {
        let compressed_fm = compress_frontmatter(&fm);
        if !compressed_fm.is_empty() {
            output.push_str(&compressed_fm);
            output.push('\n');
            output.push('\n');
        }
    }

    // Pre-process: replace [[target]] with → notation BEFORE pulldown-cmark parsing,
    // because pulldown-cmark splits [[ and ]] into separate text events.
    // We use a placeholder that pulldown-cmark won't interfere with.
    let mut preprocessed = String::new();
    {
        let mut last_end = 0;
        for cap in wikilink_re.captures_iter(body) {
            let full = cap.get(0).unwrap();
            preprocessed.push_str(&body[last_end..full.start()]);

            let inner = &cap[2];
            let target = match inner.find('|') {
                Some(pos) => inner[..pos].trim(),
                None => inner.trim(),
            };
            let page = target
                .split('#')
                .next()
                .unwrap_or(target)
                .split('^')
                .next()
                .unwrap_or(target)
                .trim();

            if dead_links.contains(page) {
                preprocessed.push_str(&format!("→ ✗ {}", page));
            } else {
                preprocessed.push_str(&format!("→ {}", page));
            }
            last_end = full.end();
        }
        preprocessed.push_str(&body[last_end..]);
    }

    // Now we have a marker regex for detecting → links in text
    let link_arrow_re = Regex::new(r"→\s").expect("invalid arrow regex");

    // Parse the preprocessed body with pulldown-cmark
    let mut options = Options::empty();
    options.insert(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(&preprocessed, options);
    let events: Vec<Event<'_>> = parser.collect();

    let mut i = 0;
    let mut in_heading = false;
    let mut heading_level: u8 = 0;
    let mut heading_text = String::new();

    let mut in_paragraph = false;
    let mut para_text = String::new();
    let mut para_has_wikilink = false;

    let mut in_code_block = false;
    let mut code_lang = String::new();
    let mut code_first_line = String::new();
    let mut code_line_count: usize = 0;

    let mut in_list = false;
    let mut list_items: Vec<String> = Vec::new();
    let mut current_item = String::new();
    let mut list_depth: usize = 0;

    let mut in_blockquote = false;
    let mut bq_depth: usize = 0;
    let mut blockquote_text = String::new();

    let mut in_table = false;
    let mut table_headers: Vec<String> = Vec::new();
    let mut table_row_count: usize = 0;
    let mut in_table_head = false;
    let mut current_table_cell = String::new();

    let mut in_tasklist_item = false;
    let mut tasklist_items: Vec<String> = Vec::new();
    let mut task_checked = false;

    let mut in_image = false;
    let mut image_alt = String::new();

    // Skip metadata block events (we already extracted frontmatter)
    let mut in_metadata = false;

    while i < events.len() {
        let event = &events[i];
        match event {
            // ── Metadata (skip — already handled) ─────────────────
            Event::Start(Tag::MetadataBlock(_)) => {
                in_metadata = true;
            }
            Event::End(TagEnd::MetadataBlock(_)) => {
                in_metadata = false;
            }

            // ── Headings ──────────────────────────────────────────
            Event::Start(Tag::Heading { level, .. }) => {
                in_heading = true;
                heading_level = match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                };
                heading_text.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                in_heading = false;
                let prefix = "#".repeat(heading_level as usize);
                output.push_str(&format!("{} {}\n", prefix, heading_text.trim()));
                output.push('\n');
            }

            // ── Images (must be before Paragraph to handle images in paragraphs) ──
            Event::Start(Tag::Image { .. }) => {
                in_image = true;
                image_alt.clear();
            }
            Event::End(TagEnd::Image) => {
                in_image = false;
                let alt = image_alt.trim();
                if !alt.is_empty() {
                    output.push_str(&format!("[img: {}]\n", alt));
                }
            }

            // ── Paragraphs ────────────────────────────────────────
            Event::Start(Tag::Paragraph) if !in_metadata && !in_blockquote => {
                in_paragraph = true;
                para_text.clear();
                para_has_wikilink = false;
            }
            Event::End(TagEnd::Paragraph) if in_paragraph && !in_blockquote => {
                in_paragraph = false;

                let compressed = compress_paragraph(&para_text, para_has_wikilink);
                if !compressed.trim().is_empty() {
                    output.push_str(compressed.trim());
                    output.push('\n');
                }
            }

            // ── Code blocks ───────────────────────────────────────
            Event::Start(Tag::CodeBlock(kind)) => {
                in_code_block = true;
                code_lang = match kind {
                    CodeBlockKind::Fenced(lang) => {
                        let l = lang.to_string();
                        if l.is_empty() {
                            "code".to_string()
                        } else {
                            l
                        }
                    }
                    CodeBlockKind::Indented => "code".to_string(),
                };
                code_first_line.clear();
                code_line_count = 0;
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                let first = code_first_line.trim();
                if code_line_count > 1 {
                    output.push_str(&format!("⌜{}: {} …⌝\n", code_lang, first));
                } else {
                    output.push_str(&format!("⌜{}: {}⌝\n", code_lang, first));
                }
            }

            // ── Lists ─────────────────────────────────────────────
            Event::Start(Tag::List(_)) => {
                list_depth += 1;
                if !in_list {
                    in_list = true;
                    list_items.clear();
                }
            }
            Event::End(TagEnd::List(_)) => {
                list_depth = list_depth.saturating_sub(1);
                if list_depth == 0 && in_list {
                    in_list = false;
                    if !tasklist_items.is_empty() {
                        output.push_str(&tasklist_items.join(", "));
                        output.push('\n');
                        tasklist_items.clear();
                    } else if !list_items.is_empty() {
                        output.push_str(&format!("• {}\n", list_items.join(", ")));
                    }
                }
            }
            Event::Start(Tag::Item) => {
                current_item.clear();
                in_tasklist_item = false;
                task_checked = false;
            }
            Event::End(TagEnd::Item) => {
                let item = current_item.trim().to_string();
                if in_tasklist_item {
                    let marker = if task_checked { "☑" } else { "☐" };
                    tasklist_items.push(format!("{} {}", marker, item));
                } else if !item.is_empty() {
                    list_items.push(item);
                }
                in_tasklist_item = false;
            }

            // ── Task list markers ─────────────────────────────────
            Event::TaskListMarker(checked) => {
                in_tasklist_item = true;
                task_checked = *checked;
            }

            // ── Block quotes ──────────────────────────────────────
            Event::Start(Tag::BlockQuote(_)) => {
                bq_depth += 1;
                if !in_blockquote {
                    in_blockquote = true;
                    blockquote_text.clear();
                }
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                bq_depth = bq_depth.saturating_sub(1);
                if bq_depth == 0 {
                    in_blockquote = false;
                    let first_sentence = first_sentence_of(&blockquote_text);
                    output.push_str(&format!("> {}\n", first_sentence.trim()));
                }
            }

            // ── Tables ────────────────────────────────────────────
            Event::Start(Tag::Table(_)) => {
                in_table = true;
                table_headers.clear();
                table_row_count = 0;
            }
            Event::End(TagEnd::Table) => {
                in_table = false;
                let header_str = table_headers.join(" │ ");
                output.push_str(&format!("┃ {} ┃ ({} rows)\n", header_str, table_row_count));
            }
            Event::Start(Tag::TableHead) => {
                in_table_head = true;
            }
            Event::End(TagEnd::TableHead) => {
                in_table_head = false;
            }
            Event::Start(Tag::TableRow) if !in_table_head => {
                table_row_count += 1;
            }
            Event::Start(Tag::TableCell) => {
                current_table_cell.clear();
            }
            Event::End(TagEnd::TableCell) if in_table_head => {
                table_headers.push(current_table_cell.trim().to_string());
            }

            // ── HTML (strip) ──────────────────────────────────────
            Event::Html(_) | Event::InlineHtml(_) => {
                // Strip entirely per spec
            }

            // ── Breaks ────────────────────────────────────────────
            Event::SoftBreak | Event::HardBreak => {
                if in_heading {
                    heading_text.push(' ');
                } else if in_paragraph {
                    para_text.push(' ');
                } else if in_blockquote {
                    blockquote_text.push(' ');
                } else if in_list || in_tasklist_item {
                    current_item.push(' ');
                }
            }

            // ── Text content ──────────────────────────────────────
            Event::Text(_) if in_metadata => {
                // Skip metadata text — we already handled frontmatter
            }
            Event::Text(text) => {
                let t = text.as_ref();

                if in_image {
                    image_alt.push_str(t);
                } else if in_heading {
                    heading_text.push_str(t);
                } else if in_code_block {
                    for line in t.lines() {
                        code_line_count += 1;
                        if code_line_count == 1 {
                            code_first_line = line.to_string();
                        }
                    }
                } else if in_paragraph {
                    if link_arrow_re.is_match(t) || t.contains("→ ✗") {
                        para_has_wikilink = true;
                    }
                    para_text.push_str(t);
                } else if in_blockquote {
                    blockquote_text.push_str(t);
                } else if in_list || in_tasklist_item {
                    current_item.push_str(t);
                } else if in_table {
                    current_table_cell.push_str(t);
                }
            }

            // ── Inline code ───────────────────────────────────────
            Event::Code(code) => {
                let c = code.as_ref();
                if in_paragraph {
                    para_text.push('`');
                    para_text.push_str(c);
                    para_text.push('`');
                } else if in_heading {
                    heading_text.push('`');
                    heading_text.push_str(c);
                    heading_text.push('`');
                } else if in_list || in_tasklist_item {
                    current_item.push('`');
                    current_item.push_str(c);
                    current_item.push('`');
                }
            }

            // ── Links (non-wiki, e.g. [text](url)) ───────────────
            Event::Start(Tag::Link { .. }) => {}
            Event::End(TagEnd::Link) => {}

            _ => {}
        }

        i += 1;
    }

    // Clean up trailing whitespace
    let result = output.trim_end().to_string();
    if result.is_empty() {
        result
    } else {
        result + "\n"
    }
}

/// Extract frontmatter from markdown (--- delimited YAML at the start).
fn extract_frontmatter(markdown: &str) -> (Option<String>, &str) {
    let trimmed = markdown.trim_start();
    if !trimmed.starts_with("---") {
        return (None, markdown);
    }

    // Find the closing ---
    let after_first = &trimmed[3..];
    if let Some(end_pos) = after_first.find("\n---") {
        let fm = after_first[..end_pos].to_string();
        let body_start = 3 + end_pos + 4; // skip "---" + content + "\n---"
        let body = if body_start < trimmed.len() {
            &trimmed[body_start..]
        } else {
            ""
        };
        (Some(fm), body)
    } else {
        (None, markdown)
    }
}


/// Extract the first sentence from text.
fn first_sentence_of(text: &str) -> String {
    let text = text.trim();
    // Find the first sentence-ending punctuation followed by space or end
    for (i, c) in text.char_indices() {
        if (c == '.' || c == '!' || c == '?') && (i + 1 >= text.len() || text[i + 1..].starts_with(char::is_whitespace)) {
            return text[..=i].to_string();
        }
    }
    // No sentence boundary found — return the whole thing
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── estimate_tokens ─────────────────────────────────────────

    #[test]
    fn test_estimate_tokens_basic() {
        assert_eq!(estimate_tokens("hello world"), 2);
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("one"), 1);
        assert_eq!(estimate_tokens("  multiple   spaces  "), 2);
    }

    // ── compress_paragraph ──────────────────────────────────────

    #[test]
    fn test_paragraph_with_wikilink_preserved() {
        let text = "Check [[OAuth Flow]] for the token exchange details.";
        let result = compress_paragraph(text, true);
        assert_eq!(result, text);
    }

    #[test]
    fn test_paragraph_signal_sentence_preserved() {
        let text = "We decided to use Clerk because of pricing.";
        let result = compress_paragraph(text, false);
        // Should be preserved verbatim (contains "decided" and "because")
        assert!(result.contains("decided"));
        assert!(result.contains("Clerk"));
        assert!(result.contains("because"));
    }

    #[test]
    fn test_paragraph_filler_compressed() {
        let text = "The system is currently running in the production environment.";
        let result = compress_paragraph(text, false);
        // Should extract key phrases joined by " — "
        assert!(result.contains("system"), "missing 'system'");
        assert!(result.contains("running"), "missing 'running'");
        assert!(result.contains("production environment"), "missing 'production environment' phrase");
        assert!(result.contains(" — "), "phrases should be dash-separated");
        assert!(!result.starts_with("The "), "should not start with stop word");
    }

    // ── compress_frontmatter ────────────────────────────────────

    #[test]
    fn test_frontmatter_basic() {
        let yaml = "tags: [auth, security]\naliases: [authn]\ndate: 2024-01-15";
        let result = compress_frontmatter(yaml);
        assert!(result.contains("@ tags: auth, security"));
        assert!(result.contains("@ aliases: authn"));
        assert!(result.contains("@ date: 2024-01-15"));
    }

    #[test]
    fn test_frontmatter_skips_internal_keys() {
        let yaml = "tags: [auth]\nzetl-internal: something\ncssclasses: wide";
        let result = compress_frontmatter(yaml);
        assert!(result.contains("@ tags: auth"));
        assert!(!result.contains("zetl-internal"));
        assert!(!result.contains("cssclasses"));
    }

    // ── build_graph_summary ─────────────────────────────────────

    #[test]
    fn test_graph_summary_basic() {
        let pages = vec![
            (
                "A".to_string(),
                vec!["B".to_string(), "C".to_string()],
                vec![],
            ),
            (
                "B".to_string(),
                vec!["A".to_string()],
                vec!["A".to_string()],
            ),
            ("D".to_string(), vec!["A".to_string()], vec![]),
        ];
        let result = build_graph_summary(&pages);
        assert!(result.contains("=== GRAPH (3 pages,"));
        assert!(result.contains("A → B, C"));
        assert!(result.contains("B ← A → A"));
        assert!(result.contains("D → A"));
    }

    #[test]
    fn test_graph_summary_counts() {
        let pages = vec![
            (
                "A".to_string(),
                vec!["B".to_string(), "C".to_string()],
                vec![],
            ),
            (
                "B".to_string(),
                vec!["A".to_string()],
                vec!["A".to_string()],
            ),
            ("C".to_string(), vec![], vec!["A".to_string()]),
            ("D".to_string(), vec!["A".to_string()], vec![]),
        ];
        let result = build_graph_summary(&pages);
        // A→B, A→C, B→A, D→A = 4 edges
        assert!(result.contains("4 pages, 4 edges"));
    }

    // ── compress_page ───────────────────────────────────────────

    #[test]
    fn test_compress_page_heading() {
        let md = "## Authentication Design\n\nSome content here.";
        let result = compress_page(md, &HashSet::new());
        assert!(result.contains("## Authentication Design"));
    }

    #[test]
    fn test_compress_page_wikilink() {
        let md = "Check [[OAuth Flow]] for details.";
        let result = compress_page(md, &HashSet::new());
        assert!(result.contains("→ OAuth Flow"));
    }

    #[test]
    fn test_compress_page_dead_link() {
        let md = "See [[Missing Page]] for info.";
        let mut dead = HashSet::new();
        dead.insert("Missing Page".to_string());
        let result = compress_page(md, &dead);
        assert!(result.contains("→ ✗ Missing Page"));
    }

    #[test]
    fn test_compress_page_code_block() {
        let md = "```rust\nfn compress(ast: &[Event]) -> String {\n    // body\n}\n```";
        let result = compress_page(md, &HashSet::new());
        assert!(result.contains("⌜rust:"));
        assert!(result.contains("fn compress"));
        assert!(result.contains("…⌝")); // multi-line = ellipsis
    }

    #[test]
    fn test_compress_page_list() {
        let md = "- token refresh\n- session management\n- CSRF protection";
        let result = compress_page(md, &HashSet::new());
        assert!(result.contains("•"));
        assert!(result.contains("token refresh"));
        assert!(result.contains("session management"));
        assert!(result.contains("CSRF protection"));
    }

    #[test]
    fn test_compress_page_blockquote() {
        let md =
            "> This is a long quotation. It contains important context that spans multiple lines.";
        let result = compress_page(md, &HashSet::new());
        assert!(result.starts_with("> This is a long quotation."));
        // Should not contain the second sentence
        assert!(!result.contains("important context"));
    }

    #[test]
    fn test_compress_page_image() {
        let md = "![architecture diagram](img.png)";
        let result = compress_page(md, &HashSet::new());
        assert!(result.contains("[img: architecture diagram]"));
    }

    #[test]
    fn test_compress_page_html_stripped() {
        let md = "<div class=\"note\">Some text</div>\n\nReal content.";
        let result = compress_page(md, &HashSet::new());
        assert!(!result.contains("<div"));
        assert!(!result.contains("</div>"));
    }

    #[test]
    fn test_compress_page_task_list() {
        let md = "- [ ] CSRF protection strategy\n- [x] provider selection";
        let result = compress_page(md, &HashSet::new());
        assert!(result.contains("☐ CSRF protection strategy"));
        assert!(result.contains("☑ provider selection"));
    }

    #[test]
    fn test_compress_page_table() {
        let md = "| Col1 | Col2 | Col3 |\n|------|------|------|\n| a | b | c |\n| d | e | f |\n| g | h | i |";
        let result = compress_page(md, &HashSet::new());
        assert!(result.contains("┃"));
        assert!(result.contains("Col1"));
        assert!(result.contains("Col2"));
        assert!(result.contains("Col3"));
        assert!(result.contains("3 rows"));
    }

    #[test]
    fn test_compress_page_frontmatter() {
        let md = "---\ntags: [auth, security]\naliases: [authn]\n---\n\n## Auth\n\nContent.";
        let result = compress_page(md, &HashSet::new());
        assert!(result.contains("@ tags: auth, security"));
        assert!(result.contains("@ aliases: authn"));
        assert!(result.contains("## Auth"));
    }

    #[test]
    fn test_compress_page_determinism() {
        let md = "---\ntags: [auth]\n---\n\n## Auth\n\n[[OAuth Flow]] handles tokens.\n\n- item1\n- item2\n\n```rust\nfn main() {}\n```";
        let dead = HashSet::new();
        let result1 = compress_page(md, &dead);
        let result2 = compress_page(md, &dead);
        assert_eq!(result1, result2, "Compression must be deterministic");
    }

    #[test]
    fn test_compress_page_wikilink_alias() {
        let md = "Check [[Session Management|sessions]] for details.";
        let result = compress_page(md, &HashSet::new());
        assert!(result.contains("→ Session Management"));
    }
}
