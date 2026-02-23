use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::prelude::*;
use regex::Regex;

/// Render markdown text into styled ratatui Lines for terminal display.
pub fn render_markdown(content: &str) -> Vec<Line<'static>> {
    let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();
    let mut style_stack: Vec<Style> = vec![Style::default()];
    let mut list_depth: usize = 0;
    let mut ordered_index: Vec<Option<u64>> = Vec::new();
    let mut in_code_block = false;
    let mut code_block_lines: Vec<String> = Vec::new();
    let mut heading_level: u8 = 0;
    let mut in_blockquote = false;

    // Skip YAML frontmatter (--- ... ---)
    let content = strip_frontmatter(content);
    let parser = Parser::new_ext(&content, options);

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    heading_level = level as u8;
                    let color = match heading_level {
                        1 => Color::Magenta,
                        2 => Color::Blue,
                        3 => Color::Cyan,
                        _ => Color::Green,
                    };
                    style_stack.push(Style::default().fg(color).add_modifier(Modifier::BOLD));
                }
                Tag::Paragraph => {}
                Tag::Emphasis => {
                    let base = *style_stack.last().unwrap_or(&Style::default());
                    style_stack.push(base.add_modifier(Modifier::ITALIC));
                }
                Tag::Strong => {
                    let base = *style_stack.last().unwrap_or(&Style::default());
                    style_stack.push(base.add_modifier(Modifier::BOLD));
                }
                Tag::Strikethrough => {
                    let base = *style_stack.last().unwrap_or(&Style::default());
                    style_stack.push(base.add_modifier(Modifier::CROSSED_OUT));
                }
                Tag::CodeBlock(_) => {
                    in_code_block = true;
                    code_block_lines.clear();
                }
                Tag::List(start) => {
                    list_depth += 1;
                    ordered_index.push(start);
                }
                Tag::Item => {
                    let indent = "  ".repeat(list_depth);
                    let bullet = if let Some(Some(idx)) = ordered_index.last_mut() {
                        let s = format!("{indent}{idx}. ");
                        *idx += 1;
                        s
                    } else {
                        format!("{indent}  ")
                    };
                    current_spans.push(Span::styled(bullet, Style::default().fg(Color::DarkGray)));
                }
                Tag::BlockQuote(_) => {
                    in_blockquote = true;
                }
                Tag::Link { .. } => {
                    let base = *style_stack.last().unwrap_or(&Style::default());
                    style_stack.push(base.fg(Color::Cyan).add_modifier(Modifier::UNDERLINED));
                }
                _ => {}
            },
            Event::End(tag_end) => match tag_end {
                TagEnd::Heading(_) => {
                    let prefix = match heading_level {
                        1 => "# ",
                        2 => "## ",
                        3 => "### ",
                        _ => "#### ",
                    };
                    let style = style_stack.pop().unwrap_or_default();
                    let mut heading_spans = vec![Span::styled(prefix.to_string(), style)];
                    heading_spans.append(&mut current_spans);
                    lines.push(Line::from(heading_spans));
                    lines.push(Line::raw(""));
                }
                TagEnd::Paragraph => {
                    if in_blockquote {
                        let mut bq_spans = vec![Span::styled(
                            "  | ".to_string(),
                            Style::default().fg(Color::DarkGray),
                        )];
                        bq_spans.append(&mut current_spans);
                        lines.push(Line::from(bq_spans));
                    } else {
                        flush_line(&mut lines, &mut current_spans);
                    }
                    lines.push(Line::raw(""));
                }
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => {
                    style_stack.pop();
                }
                TagEnd::CodeBlock => {
                    in_code_block = false;
                    let code_style = Style::default().fg(Color::Gray);
                    lines.push(Line::styled(
                        "  ┌────────".to_string(),
                        Style::default().fg(Color::DarkGray),
                    ));
                    for code_line in &code_block_lines {
                        lines.push(Line::from(vec![
                            Span::styled("  │ ".to_string(), Style::default().fg(Color::DarkGray)),
                            Span::styled(code_line.clone(), code_style),
                        ]));
                    }
                    lines.push(Line::styled(
                        "  └────────".to_string(),
                        Style::default().fg(Color::DarkGray),
                    ));
                    lines.push(Line::raw(""));
                    code_block_lines.clear();
                }
                TagEnd::List(_) => {
                    list_depth = list_depth.saturating_sub(1);
                    ordered_index.pop();
                    if list_depth == 0 {
                        lines.push(Line::raw(""));
                    }
                }
                TagEnd::Item => {
                    flush_line(&mut lines, &mut current_spans);
                }
                TagEnd::BlockQuote(_) => {
                    in_blockquote = false;
                }
                _ => {}
            },
            Event::Text(text) => {
                if in_code_block {
                    for line in text.lines() {
                        code_block_lines.push(line.to_string());
                    }
                } else {
                    let style = *style_stack.last().unwrap_or(&Style::default());
                    // Highlight wikilinks within text
                    let spans = highlight_wikilinks(&text, style);
                    current_spans.extend(spans);
                }
            }
            Event::Code(code) => {
                current_spans.push(Span::styled(
                    format!("`{code}`"),
                    Style::default().fg(Color::Yellow),
                ));
            }
            Event::SoftBreak | Event::HardBreak => {
                flush_line(&mut lines, &mut current_spans);
            }
            Event::Rule => {
                lines.push(Line::styled(
                    "  ────────────────────────────────────────".to_string(),
                    Style::default().fg(Color::DarkGray),
                ));
                lines.push(Line::raw(""));
            }
            _ => {}
        }
    }

    // Flush remaining spans
    if !current_spans.is_empty() {
        flush_line(&mut lines, &mut current_spans);
    }

    lines
}

fn flush_line(lines: &mut Vec<Line<'static>>, spans: &mut Vec<Span<'static>>) {
    if !spans.is_empty() {
        lines.push(Line::from(std::mem::take(spans)));
    }
}

/// Highlight [[wikilinks]] within text, preserving the surrounding style.
fn highlight_wikilinks(text: &str, base_style: Style) -> Vec<Span<'static>> {
    let re = Regex::new(r"\[\[([^\[\]]+)\]\]").unwrap();
    let mut spans = Vec::new();
    let mut last_end = 0;

    for cap in re.captures_iter(text) {
        let m = cap.get(0).unwrap();
        if m.start() > last_end {
            spans.push(Span::styled(
                text[last_end..m.start()].to_string(),
                base_style,
            ));
        }
        spans.push(Span::styled(
            m.as_str().to_string(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
        last_end = m.end();
    }

    if last_end < text.len() {
        spans.push(Span::styled(text[last_end..].to_string(), base_style));
    }

    if spans.is_empty() {
        spans.push(Span::styled(text.to_string(), base_style));
    }

    spans
}

/// Strip YAML frontmatter (--- ... ---) from the beginning of content.
fn strip_frontmatter(content: &str) -> String {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return content.to_string();
    }

    // Find closing ---
    let after_first = &trimmed[3..];
    if let Some(end_pos) = after_first.find("\n---") {
        let skip = 3 + end_pos + 4; // "---" + content + "\n---"
                                    // Skip optional newline after closing ---
        let rest = &trimmed[skip..];
        let rest = rest.strip_prefix('\n').unwrap_or(rest);
        rest.to_string()
    } else {
        content.to_string()
    }
}

/// Parse YAML frontmatter into key-value pairs (simple flat parsing).
pub fn parse_frontmatter(content: &str) -> Vec<(String, String)> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return Vec::new();
    }

    let after_first = &trimmed[3..];
    let fm_content = if let Some(end_pos) = after_first.find("\n---") {
        &after_first[..end_pos]
    } else {
        return Vec::new();
    };

    let mut pairs = Vec::new();
    for line in fm_content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(colon_pos) = line.find(':') {
            let key = line[..colon_pos].trim().to_string();
            let value = line[colon_pos + 1..].trim().to_string();
            if !key.is_empty() {
                pairs.push((key, value));
            }
        }
    }

    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_frontmatter() {
        let content = "---\ntitle: Test\n---\n# Hello";
        assert_eq!(strip_frontmatter(content), "# Hello");
    }

    #[test]
    fn test_strip_frontmatter_no_frontmatter() {
        let content = "# Hello\nWorld";
        assert_eq!(strip_frontmatter(content), content);
    }

    #[test]
    fn test_parse_frontmatter() {
        let content = "---\ntitle: My Page\ntags: [a, b]\n---\n# Hello";
        let pairs = parse_frontmatter(content);
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0], ("title".to_string(), "My Page".to_string()));
        assert_eq!(pairs[1], ("tags".to_string(), "[a, b]".to_string()));
    }

    #[test]
    fn test_render_heading() {
        let lines = render_markdown("# Hello World");
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_render_wikilink() {
        let lines = render_markdown("See [[Page Name]] here");
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_render_code_block() {
        let lines = render_markdown("```\nlet x = 1;\n```");
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_render_list() {
        let lines = render_markdown("- item 1\n- item 2\n- item 3");
        assert!(!lines.is_empty());
    }
}
