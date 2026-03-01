use std::collections::{HashMap, HashSet};

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use regex::Regex;

use crate::web::html::{html_escape, urlencoding};

/// Render markdown content to HTML, rewriting `[[wikilinks]]` into `<a>` tags.
/// `slug_map` maps resolved page names to their URL slugs (e.g. "Scanner" → "architecture/Scanner").
/// Links whose target is not in `slug_map` get `class="link-error"`.
pub fn render_to_html(content: &str, slug_map: &HashMap<String, String>) -> String {
    let stripped = strip_frontmatter(content);
    let fm_lines = frontmatter_line_count(content);
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_SMART_PUNCTUATION
        | Options::ENABLE_MATH
        | Options::ENABLE_GFM
        | Options::ENABLE_DEFINITION_LIST;
    let parser = Parser::new_ext(&stripped, options);

    let wikilink_re = Regex::new(r"\[\[([^\[\]]+)\]\]").unwrap();

    // Compute line-start byte offsets for the stripped content
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(stripped.match_indices('\n').map(|(i, _)| i + 1))
        .collect();

    // Collect events with line anchors injected after block-start events
    let mut events: Vec<Event> = Vec::new();
    let mut anchored_lines: HashSet<usize> = HashSet::new();
    for (event, range) in parser.into_offset_iter() {
        let is_block_start = matches!(
            &event,
            Event::Start(
                Tag::Paragraph
                    | Tag::Heading { .. }
                    | Tag::BlockQuote(_)
                    | Tag::List(_)
                    | Tag::Item
            )
        );
        events.push(event);
        if is_block_start {
            // 1-based line number in the original file (accounting for frontmatter)
            let line = line_starts.partition_point(|&s| s <= range.start) + fm_lines;
            if anchored_lines.insert(line) {
                events.push(Event::Html(
                    format!("<a id=\"line-{}\" class=\"line-anchor\"></a>", line).into(),
                ));
            }
        }
    }

    let mut html_output = String::new();
    pulldown_cmark::html::push_html(&mut html_output, events.into_iter());

    rewrite_wikilinks(&html_output, &wikilink_re, slug_map)
}

/// Render a short preview (first ~200 chars of meaningful content) for tooltip.
pub fn render_preview(content: &str) -> String {
    let content = strip_frontmatter(content);
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_SMART_PUNCTUATION
        | Options::ENABLE_MATH
        | Options::ENABLE_GFM
        | Options::ENABLE_DEFINITION_LIST;
    let parser = Parser::new_ext(&content, options);

    let mut text = String::new();
    let mut in_code_block = false;

    for event in parser {
        match event {
            Event::Start(Tag::CodeBlock(_)) => in_code_block = true,
            Event::End(TagEnd::CodeBlock) => in_code_block = false,
            Event::Text(t) if !in_code_block => {
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(&t);
                if text.len() > 1200 {
                    break;
                }
            }
            _ => {}
        }
    }

    if text.len() > 1000 {
        if let Some(pos) = text[..1000].rfind(' ') {
            text.truncate(pos);
        } else {
            text.truncate(1000);
        }
        text.push_str("...");
    }

    html_escape(&text)
}

/// Render a markdown preview as styled HTML, limited to ~12 block-level elements.
/// Wikilinks are rewritten into clickable `<a>` tags.
pub fn render_preview_html(content: &str, slug_map: &HashMap<String, String>) -> String {
    let content = strip_frontmatter(content);
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_SMART_PUNCTUATION
        | Options::ENABLE_MATH
        | Options::ENABLE_GFM
        | Options::ENABLE_DEFINITION_LIST;
    let parser = Parser::new_ext(&content, options);

    let wikilink_re = Regex::new(r"\[\[([^\[\]]+)\]\]").unwrap();

    let mut events: Vec<Event> = Vec::new();
    let mut block_count = 0;
    let max_blocks = 12;

    for event in parser {
        let is_block_end = matches!(
            &event,
            Event::End(
                TagEnd::Paragraph
                    | TagEnd::Heading(_)
                    | TagEnd::List(_)
                    | TagEnd::CodeBlock
                    | TagEnd::BlockQuote(_)
            )
        );
        events.push(event);
        if is_block_end {
            block_count += 1;
            if block_count >= max_blocks {
                break;
            }
        }
    }

    let mut html_output = String::new();
    pulldown_cmark::html::push_html(&mut html_output, events.into_iter());

    rewrite_wikilinks(&html_output, &wikilink_re, slug_map)
}

/// Replace [[wikilinks]] with <a> tags in HTML, skipping content inside <code>/<pre>.
fn rewrite_wikilinks(html: &str, re: &Regex, slug_map: &HashMap<String, String>) -> String {
    let mut result = String::with_capacity(html.len());
    let mut depth: usize = 0;

    let mut chars = html.char_indices().peekable();
    let mut segment_start = 0;

    while let Some(&(i, _)) = chars.peek() {
        if html[i..].starts_with("<code") || html[i..].starts_with("<pre") {
            if depth == 0 && i > segment_start {
                result.push_str(&replace_wikilinks_in_segment(
                    &html[segment_start..i],
                    re,
                    slug_map,
                ));
            } else if i > segment_start {
                result.push_str(&html[segment_start..i]);
            }
            segment_start = i;
            depth += 1;
            chars.next();
        } else if html[i..].starts_with("</code>") || html[i..].starts_with("</pre>") {
            let tag_end = html[i..].find('>').map(|p| i + p + 1).unwrap_or(html.len());
            if depth > 0 {
                depth -= 1;
            }
            if depth == 0 {
                result.push_str(&html[segment_start..tag_end]);
                segment_start = tag_end;
            }
            while let Some(&(j, _)) = chars.peek() {
                if j >= tag_end {
                    break;
                }
                chars.next();
            }
        } else {
            chars.next();
        }
    }

    if segment_start < html.len() {
        if depth == 0 {
            result.push_str(&replace_wikilinks_in_segment(
                &html[segment_start..],
                re,
                slug_map,
            ));
        } else {
            result.push_str(&html[segment_start..]);
        }
    }

    result
}

fn replace_wikilinks_in_segment(
    segment: &str,
    re: &Regex,
    slug_map: &HashMap<String, String>,
) -> String {
    re.replace_all(segment, |caps: &regex::Captures| {
        let inner = &caps[1];
        let (target, display) = if let Some(pipe_pos) = inner.find('|') {
            (&inner[..pipe_pos], &inner[pipe_pos + 1..])
        } else {
            (inner, inner)
        };
        // Strip heading/block refs for page resolution
        let page = target.split('#').next().unwrap_or(target).trim();
        let display = html_escape(display.trim());
        // Look up slug by case-insensitive page name match
        let slug = slug_map
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(page))
            .map(|(_, v)| v.as_str());

        if let Some(slug) = slug {
            format!(
                r#"<a href="/{href}" class="link link-primary wikilink">{display}</a>"#,
                href = urlencoding(slug),
                display = display,
            )
        } else {
            format!(
                r#"<a href="/{href}" class="link-error wikilink wikilink-dead">{display}</a>"#,
                href = urlencoding(page),
                display = display,
            )
        }
    })
    .to_string()
}

/// Parse YAML frontmatter from the beginning of a markdown file into a JSON value.
/// Returns an empty object `{}` if no frontmatter is present or if the YAML is malformed.
pub fn parse_frontmatter(content: &str) -> serde_json::Value {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return serde_json::Value::Object(serde_json::Map::new());
    }

    let after_first = &trimmed[3..];
    if let Some(end_pos) = after_first.find("\n---") {
        let yaml_block = &after_first[..end_pos];
        match serde_yaml_ng::from_str::<serde_json::Value>(yaml_block) {
            Ok(val) if val.is_object() => val,
            Ok(_) => serde_json::Value::Object(serde_json::Map::new()),
            Err(e) => {
                eprintln!("Warning: malformed frontmatter YAML: {e}");
                serde_json::Value::Object(serde_json::Map::new())
            }
        }
    } else {
        serde_json::Value::Object(serde_json::Map::new())
    }
}

/// Count the number of lines consumed by YAML frontmatter (including delimiters).
fn frontmatter_line_count(content: &str) -> usize {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return 0;
    }
    let after_first = &trimmed[3..];
    if let Some(end_pos) = after_first.find("\n---") {
        let fm_end = 3 + end_pos + 4; // opening "---" + content + "\n---"
        let fm_text = &trimmed[..fm_end];
        // Count lines in frontmatter + 1 for the line after closing "---"
        fm_text.matches('\n').count() + 1
    } else {
        0
    }
}

/// Strip YAML frontmatter (--- ... ---) from the beginning of content.
fn strip_frontmatter(content: &str) -> String {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return content.to_string();
    }

    let after_first = &trimmed[3..];
    if let Some(end_pos) = after_first.find("\n---") {
        let skip = 3 + end_pos + 4;
        let rest = &trimmed[skip..];
        let rest = rest.strip_prefix('\n').unwrap_or(rest);
        rest.to_string()
    } else {
        content.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_simple_wikilink() {
        let mut slug_map = HashMap::new();
        slug_map.insert("Target".to_string(), "folder/target".to_string());
        let html = render_to_html("See [[Target]] here", &slug_map);
        assert!(html.contains(r#"href="/folder/target""#));
        assert!(html.contains("link-primary"));
    }

    #[test]
    fn test_aliased_wikilink() {
        let mut slug_map = HashMap::new();
        slug_map.insert("Target".to_string(), "folder/target".to_string());
        let html = render_to_html("See [[Target|click me]] here", &slug_map);
        assert!(html.contains("click me"));
        assert!(html.contains(r#"href="/folder/target""#));
    }

    #[test]
    fn test_dead_link() {
        let slug_map = HashMap::new();
        let html = render_to_html("See [[Missing]] here", &slug_map);
        assert!(html.contains("link-error"));
    }

    #[test]
    fn test_wikilink_in_code_block_untouched() {
        let mut slug_map = HashMap::new();
        slug_map.insert("Target".to_string(), "folder/target".to_string());
        let html = render_to_html("```\n[[Target]]\n```", &slug_map);
        // Inside code block, should NOT be rewritten to <a>
        assert!(!html.contains("link-primary"));
    }

    #[test]
    fn test_strip_frontmatter() {
        let content = "---\ntitle: Test\n---\n# Hello";
        assert_eq!(strip_frontmatter(content), "# Hello");
    }

    #[test]
    fn test_root_level_slug() {
        let mut slug_map = HashMap::new();
        slug_map.insert("Notes".to_string(), "notes".to_string());
        let html = render_to_html("See [[Notes]] here", &slug_map);
        assert!(html.contains(r#"href="/notes""#));
    }

    #[test]
    fn test_kebab_case_slug() {
        let mut slug_map = HashMap::new();
        slug_map.insert("Link Graph".to_string(), "architecture/link-graph".to_string());
        let html = render_to_html("See [[Link Graph]] here", &slug_map);
        assert!(html.contains(r#"href="/architecture/link-graph""#));
    }

    #[test]
    fn test_parse_frontmatter_valid() {
        let content = "---\ntitle: Hello\ntags:\n  - rust\n  - cli\n---\n# Body";
        let fm = parse_frontmatter(content);
        assert_eq!(fm["title"], "Hello");
        let tags = fm["tags"].as_array().unwrap();
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0], "rust");
    }

    #[test]
    fn test_parse_frontmatter_none() {
        let content = "# No frontmatter here";
        let fm = parse_frontmatter(content);
        assert!(fm.is_object());
        assert!(fm.as_object().unwrap().is_empty());
    }

    #[test]
    fn test_parse_frontmatter_malformed() {
        let content = "---\n: [invalid yaml\n---\n# Body";
        let fm = parse_frontmatter(content);
        assert!(fm.is_object());
        assert!(fm.as_object().unwrap().is_empty());
    }

    #[test]
    fn test_parse_frontmatter_unclosed() {
        let content = "---\ntitle: Test\n# No closing fence";
        let fm = parse_frontmatter(content);
        assert!(fm.is_object());
        assert!(fm.as_object().unwrap().is_empty());
    }
}
