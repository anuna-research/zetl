use std::collections::HashSet;

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use regex::Regex;

use crate::web::html::{html_escape, urlencoding};

/// Render markdown content to HTML, rewriting `[[wikilinks]]` into `<a>` tags.
/// Links whose target is not in `resolved` get `class="link-error"`.
pub fn render_to_html(content: &str, resolved: &HashSet<String>) -> String {
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

    rewrite_wikilinks(&html_output, &wikilink_re, resolved)
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
pub fn render_preview_html(content: &str, resolved: &HashSet<String>) -> String {
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

    rewrite_wikilinks(&html_output, &wikilink_re, resolved)
}

/// Replace [[wikilinks]] with <a> tags in HTML, skipping content inside <code>/<pre>.
fn rewrite_wikilinks(html: &str, re: &Regex, resolved: &HashSet<String>) -> String {
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
                    resolved,
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
                resolved,
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
    resolved: &HashSet<String>,
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
        // Use canonical (resolved) name for href so it matches transclusion card hrefs
        let canonical = resolved.iter().find(|r| r.eq_ignore_ascii_case(page));

        if let Some(canon) = canonical {
            format!(
                r#"<a href="/page/{href}" class="link link-primary wikilink">{display}</a>"#,
                href = urlencoding(canon),
                display = display,
            )
        } else {
            format!(
                r#"<a href="/page/{href}" class="link-error wikilink wikilink-dead">{display}</a>"#,
                href = urlencoding(page),
                display = display,
            )
        }
    })
    .to_string()
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
    use std::collections::HashSet;

    #[test]
    fn test_simple_wikilink() {
        let mut resolved = HashSet::new();
        resolved.insert("Target".to_string());
        let html = render_to_html("See [[Target]] here", &resolved);
        assert!(html.contains(r#"href="/page/Target""#));
        assert!(html.contains("link-primary"));
    }

    #[test]
    fn test_aliased_wikilink() {
        let mut resolved = HashSet::new();
        resolved.insert("Target".to_string());
        let html = render_to_html("See [[Target|click me]] here", &resolved);
        assert!(html.contains("click me"));
        assert!(html.contains(r#"href="/page/Target""#));
    }

    #[test]
    fn test_dead_link() {
        let resolved = HashSet::new();
        let html = render_to_html("See [[Missing]] here", &resolved);
        assert!(html.contains("link-error"));
    }

    #[test]
    fn test_wikilink_in_code_block_untouched() {
        let mut resolved = HashSet::new();
        resolved.insert("Target".to_string());
        let html = render_to_html("```\n[[Target]]\n```", &resolved);
        // Inside code block, should NOT be rewritten to <a>
        assert!(!html.contains("link-primary"));
    }

    #[test]
    fn test_strip_frontmatter() {
        let content = "---\ntitle: Test\n---\n# Hello";
        assert_eq!(strip_frontmatter(content), "# Hello");
    }
}
