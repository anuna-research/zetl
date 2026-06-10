use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use pulldown_cmark::{CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use regex::Regex;

use crate::web::html::{html_escape, urlencoding};

/// GitHub-style heading slug: lower-case, non-alphanumerics collapsed to
/// `-`, edges trimmed. Unicode letters and digits are preserved; other
/// characters (punctuation, whitespace, emoji) become separators. Returns
/// an empty string for headings whose text slugifies to nothing (e.g.
/// all-punctuation); callers should skip id assignment in that case.
fn slugify_heading(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev_dash = true;
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            out.extend(ch.to_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Walk `events` and populate `id` on every `Tag::Heading` whose id is
/// currently `None`. The id is a slugified form of the heading's visible
/// text (concatenating `Event::Text`, `Event::Code`, and `Event::Html`
/// text-like content between `Start(Heading)` and `End(Heading)`).
/// Collisions within a single document are disambiguated with a `-N`
/// suffix matching GitHub and rustdoc conventions.
///
/// Headings that the author already annotated with `{#custom-id}` (via
/// `ENABLE_HEADING_ATTRIBUTES`) are left alone — the author's explicit
/// choice wins, and we still register the id in the seen-set so later
/// auto-generated ids won't collide with it.
fn inject_heading_ids(events: &mut [Event<'_>]) {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut i = 0;
    while i < events.len() {
        let is_heading_start = matches!(&events[i], Event::Start(Tag::Heading { .. }));
        if !is_heading_start {
            i += 1;
            continue;
        }
        // Find the matching End(Heading). Headings do not nest in
        // CommonMark, so a forward linear scan is sufficient.
        let mut end = i + 1;
        while end < events.len() && !matches!(&events[end], Event::End(TagEnd::Heading(_))) {
            end += 1;
        }
        // Collect visible text between start and end.
        let mut buf = String::new();
        for ev in &events[i + 1..end] {
            match ev {
                Event::Text(t) | Event::Code(t) => buf.push_str(t),
                Event::SoftBreak | Event::HardBreak => buf.push(' '),
                _ => {}
            }
        }
        if let Event::Start(Tag::Heading { id, .. }) = &mut events[i] {
            if id.is_none() {
                let base = slugify_heading(&buf);
                if !base.is_empty() {
                    let n = *seen.get(&base).unwrap_or(&0);
                    let unique = if n == 0 {
                        base.clone()
                    } else {
                        format!("{base}-{n}")
                    };
                    seen.insert(base, n + 1);
                    *id = Some(CowStr::Boxed(unique.into_boxed_str()));
                }
            } else if let Some(explicit) = id {
                // Still register explicit ids so auto-ids after this
                // don't collide with them.
                seen.entry(explicit.to_string()).or_insert(1);
            }
        }
        i = end + 1;
    }
}

/// Render markdown content to HTML, rewriting `[[wikilinks]]` into `<a>` tags.
/// `slug_map` maps resolved page names to their URL slugs (e.g. "Scanner" → "architecture/Scanner").
/// Links whose target is not in `slug_map` get `class="link-error"`.
pub fn render_to_html(
    content: &str,
    slug_map: &HashMap<String, String>,
    root_path: &str,
    index_file: &str,
) -> String {
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
        // Note: we intentionally do NOT match Tag::List(_) here. Placing
        // the injected line-anchor <a> after a list-start event makes it
        // a direct child of <ul>/<ol>, which fails WCAG / axe list-children
        // checks. The Tag::Item branch below still places an anchor inside
        // the first <li> so deep-linking to a list still works.
        let is_block_start = matches!(
            &event,
            Event::Start(Tag::Paragraph | Tag::Heading { .. } | Tag::BlockQuote(_) | Tag::Item)
        );
        events.push(event);
        if is_block_start {
            // 1-based line number in the original file (accounting for frontmatter)
            let line = line_starts.partition_point(|&s| s <= range.start) + fm_lines;
            if anchored_lines.insert(line) {
                events.push(Event::Html(
                    format!("<a id=\"line-{line}\" class=\"line-anchor\"></a>").into(),
                ));
            }
        }
    }

    inject_heading_ids(&mut events);

    let mut html_output = String::new();
    pulldown_cmark::html::push_html(&mut html_output, events.into_iter());

    render_predicate_chips(rewrite_wikilinks(
        &html_output,
        &wikilink_re,
        slug_map,
        root_path,
        index_file,
    ))
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
pub fn render_preview_html(
    content: &str,
    slug_map: &HashMap<String, String>,
    root_path: &str,
    index_file: &str,
) -> String {
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
        // Demote headings inside transclusion previews so the outer page's
        // outline (<h1 class="page-title">) is preserved. The target page's
        // own `# Title` would otherwise emit a competing <h1> for every chip.
        let event = match event {
            Event::Start(Tag::Heading {
                level,
                id,
                classes,
                attrs,
            }) => Event::Start(Tag::Heading {
                level: demote_heading_level(level),
                id,
                classes,
                attrs,
            }),
            Event::End(TagEnd::Heading(level)) => {
                Event::End(TagEnd::Heading(demote_heading_level(level)))
            }
            other => other,
        };

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

    render_predicate_chips(rewrite_wikilinks_for_preview(
        &html_output,
        &wikilink_re,
        slug_map,
        root_path,
        index_file,
    ))
}

fn demote_heading_level(level: HeadingLevel) -> HeadingLevel {
    match level {
        HeadingLevel::H1 => HeadingLevel::H3,
        HeadingLevel::H2 => HeadingLevel::H4,
        HeadingLevel::H3 => HeadingLevel::H5,
        HeadingLevel::H4 | HeadingLevel::H5 | HeadingLevel::H6 => HeadingLevel::H6,
    }
}

/// Replace [[wikilinks]] with plain <a> tags in preview/excerpt HTML.
///
/// Unlike `rewrite_wikilinks`, the links emitted here do NOT carry the
/// `wikilink` CSS class. This prevents the transclusion-panel interaction
/// JS (bridge lines, hover highlights) from treating links *inside*
/// transclusion card excerpts as first-class wikilinks — which would cause
/// "transclusion links persisting" across nested previews.
fn rewrite_wikilinks_for_preview(
    html: &str,
    re: &Regex,
    slug_map: &HashMap<String, String>,
    root_path: &str,
    index_file: &str,
) -> String {
    let mut result = String::with_capacity(html.len());
    let mut depth: usize = 0;

    let mut chars = html.char_indices().peekable();
    let mut segment_start = 0;

    while let Some(&(i, _)) = chars.peek() {
        if html[i..].starts_with("<code") || html[i..].starts_with("<pre") {
            if depth == 0 && i > segment_start {
                result.push_str(&replace_wikilinks_in_segment_preview(
                    &html[segment_start..i],
                    re,
                    slug_map,
                    root_path,
                    index_file,
                ));
            } else if i > segment_start {
                result.push_str(&html[segment_start..i]);
            }
            segment_start = i;
            depth += 1;
            chars.next();
        } else if html[i..].starts_with("</code>") || html[i..].starts_with("</pre>") {
            let tag_end = html[i..].find('>').map(|p| i + p + 1).unwrap_or(html.len());
            depth = depth.saturating_sub(1);
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
            result.push_str(&replace_wikilinks_in_segment_preview(
                &html[segment_start..],
                re,
                slug_map,
                root_path,
                index_file,
            ));
        } else {
            result.push_str(&html[segment_start..]);
        }
    }

    result
}

/// Preview variant of wikilink replacement: emits plain links without the
/// `wikilink` class so they do not participate in transclusion interaction.
fn replace_wikilinks_in_segment_preview(
    segment: &str,
    re: &Regex,
    slug_map: &HashMap<String, String>,
    root_path: &str,
    index_file: &str,
) -> String {
    re.replace_all(segment, |caps: &regex::Captures| {
        let inner = &caps[1];
        let (target, display) = if let Some(pipe_pos) = inner.find('|') {
            (&inner[..pipe_pos], &inner[pipe_pos + 1..])
        } else {
            (inner, inner)
        };
        let page = target.split('#').next().unwrap_or(target).trim();
        let display = html_escape(display.trim());
        let slug = slug_map
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(page))
            .map(|(_, v)| v.as_str());

        if let Some(slug) = slug {
            format!(
                r#"<a href="{root_path}{href}/{index_file}" class="link link-primary">{display}</a>"#,
                root_path = root_path,
                href = urlencoding(slug),
                index_file = index_file,
                display = display,
            )
        } else {
            format!(
                r#"<a href="{root_path}{href}/{index_file}" class="link-error">{display}</a>"#,
                root_path = root_path,
                href = urlencoding(page),
                index_file = index_file,
                display = display,
            )
        }
    })
    .to_string()
}

/// Replace [[wikilinks]] with <a> tags in HTML, skipping content inside <code>/<pre>.
/// SPEC-045: turn a recognised `predicate::` run rendered in front of a
/// wikilink into styled chip spans, so the published page reads
/// "⟨Derived from⟩ 2026 Q1 Retro" instead of leaking the raw
/// `derived_from::` text.
///
/// Runs as a single post-pass over the already-rendered HTML. The match is
/// anchored to a tag-close `>` immediately before the predicate run, which
/// reproduces the scanner's *leading-position* rule without a lookbehind: in
/// list-item form the run always follows the injected line-anchor `</a>`, and
/// a malformed prefix (`derived from::`, `Derived_From::`, `derived_from/…::`,
/// `derived_from:::`) is not `>`-adjacent as a clean `(predicate "::")+` run,
/// so it stays plain text — exactly as the graph treats it (no chip where
/// there is no typed edge). Embeds (`![[…]]`) never render as `<a … wikilink>`,
/// so they are never chipped.
fn render_predicate_chips(html: String) -> String {
    static CHIP_RE: OnceLock<Regex> = OnceLock::new();
    let re = CHIP_RE.get_or_init(|| {
        // (>) ( 1*( (snake|curie) "::" ) ) ( <a … wikilink … > )
        Regex::new(
            r#"(?P<b>>)(?P<run>(?:[a-z][a-z0-9_]*::|[a-z][a-z0-9]*:[A-Za-z][A-Za-z0-9_-]*::)+)(?P<a><a\b[^>]*\bwikilink\b)"#,
        )
        .expect("invalid predicate-chip regex")
    });

    if !html.contains("::") {
        return html; // fast path: nothing to do
    }

    re.replace_all(&html, |caps: &regex::Captures| {
        let run = &caps["run"];
        let mut seen: Vec<&str> = Vec::new();
        let mut chips = String::new();
        for seg in run.split("::") {
            if seg.is_empty() || seen.contains(&seg) {
                continue; // drop the trailing empty + de-duplicate (a::a → one)
            }
            seen.push(seg);
            let label = html_escape(&crate::predicates::auto_label(seg));
            chips.push_str(&format!(
                r#"<span class="zetl-edge-predicate" data-predicate="{}">{}</span>"#,
                html_escape(seg),
                label
            ));
        }
        format!("{}{}{}", &caps["b"], chips, &caps["a"])
    })
    .into_owned()
}

fn rewrite_wikilinks(
    html: &str,
    re: &Regex,
    slug_map: &HashMap<String, String>,
    root_path: &str,
    index_file: &str,
) -> String {
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
                    root_path,
                    index_file,
                ));
            } else if i > segment_start {
                result.push_str(&html[segment_start..i]);
            }
            segment_start = i;
            depth += 1;
            chars.next();
        } else if html[i..].starts_with("</code>") || html[i..].starts_with("</pre>") {
            let tag_end = html[i..].find('>').map(|p| i + p + 1).unwrap_or(html.len());
            depth = depth.saturating_sub(1);
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
                root_path,
                index_file,
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
    root_path: &str,
    index_file: &str,
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
                r#"<a href="{root_path}{href}/{index_file}" class="link link-primary wikilink">{display}</a>"#,
                root_path = root_path,
                href = urlencoding(slug),
                index_file = index_file,
                display = display,
            )
        } else {
            format!(
                r#"<a href="{root_path}{href}/{index_file}" class="link-error wikilink wikilink-dead">{display}</a>"#,
                root_path = root_path,
                href = urlencoding(page),
                index_file = index_file,
                display = display,
            )
        }
    })
    .to_string()
}

/// How a denied page's wikilink should be rendered (REQ-020-032).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeniedLinkStyle {
    /// Grayed-out link with page title visible; click → 403.
    GrayedOut,
    /// Lock icon with generic tooltip; page title visible; click → 403.
    Locked,
    /// Dead link — indistinguishable from nonexistent page.
    DeadLink,
}

/// Render markdown to HTML with visibility-aware wikilink rendering (REQ-020-032).
///
/// `denied_pages` maps page names (case-insensitive key) to their denied link style.
/// Pages not in the map are rendered normally.
pub fn render_to_html_with_visibility(
    content: &str,
    slug_map: &HashMap<String, String>,
    denied_pages: &HashMap<String, DeniedLinkStyle>,
    root_path: &str,
    index_file: &str,
) -> String {
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

    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(stripped.match_indices('\n').map(|(i, _)| i + 1))
        .collect();

    let mut events: Vec<Event> = Vec::new();
    let mut anchored_lines: HashSet<usize> = HashSet::new();
    for (event, range) in parser.into_offset_iter() {
        // Note: we intentionally do NOT match Tag::List(_) here. Placing
        // the injected line-anchor <a> after a list-start event makes it
        // a direct child of <ul>/<ol>, which fails WCAG / axe list-children
        // checks. The Tag::Item branch below still places an anchor inside
        // the first <li> so deep-linking to a list still works.
        let is_block_start = matches!(
            &event,
            Event::Start(Tag::Paragraph | Tag::Heading { .. } | Tag::BlockQuote(_) | Tag::Item)
        );
        events.push(event);
        if is_block_start {
            let line = line_starts.partition_point(|&s| s <= range.start) + fm_lines;
            if anchored_lines.insert(line) {
                events.push(Event::Html(
                    format!("<a id=\"line-{line}\" class=\"line-anchor\"></a>").into(),
                ));
            }
        }
    }

    inject_heading_ids(&mut events);

    let mut html_output = String::new();
    pulldown_cmark::html::push_html(&mut html_output, events.into_iter());

    render_predicate_chips(rewrite_wikilinks_with_visibility(
        &html_output,
        &wikilink_re,
        slug_map,
        denied_pages,
        root_path,
        index_file,
    ))
}

/// Replace [[wikilinks]] with visibility-aware <a> tags in HTML, skipping <code>/<pre>.
fn rewrite_wikilinks_with_visibility(
    html: &str,
    re: &Regex,
    slug_map: &HashMap<String, String>,
    denied_pages: &HashMap<String, DeniedLinkStyle>,
    root_path: &str,
    index_file: &str,
) -> String {
    let mut result = String::with_capacity(html.len());
    let mut depth: usize = 0;
    let mut chars = html.char_indices().peekable();
    let mut segment_start = 0;

    while let Some(&(i, _)) = chars.peek() {
        if html[i..].starts_with("<code") || html[i..].starts_with("<pre") {
            if depth == 0 && i > segment_start {
                result.push_str(&replace_wikilinks_visibility_segment(
                    &html[segment_start..i],
                    re,
                    slug_map,
                    denied_pages,
                    root_path,
                    index_file,
                ));
            } else if i > segment_start {
                result.push_str(&html[segment_start..i]);
            }
            segment_start = i;
            depth += 1;
            chars.next();
        } else if html[i..].starts_with("</code>") || html[i..].starts_with("</pre>") {
            let tag_end = html[i..].find('>').map(|p| i + p + 1).unwrap_or(html.len());
            depth = depth.saturating_sub(1);
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
            result.push_str(&replace_wikilinks_visibility_segment(
                &html[segment_start..],
                re,
                slug_map,
                denied_pages,
                root_path,
                index_file,
            ));
        } else {
            result.push_str(&html[segment_start..]);
        }
    }

    result
}

fn replace_wikilinks_visibility_segment(
    segment: &str,
    re: &Regex,
    slug_map: &HashMap<String, String>,
    denied_pages: &HashMap<String, DeniedLinkStyle>,
    root_path: &str,
    index_file: &str,
) -> String {
    re.replace_all(segment, |caps: &regex::Captures| {
        let inner = &caps[1];
        let (target, display) = if let Some(pipe_pos) = inner.find('|') {
            (&inner[..pipe_pos], &inner[pipe_pos + 1..])
        } else {
            (inner, inner)
        };
        let page = target.split('#').next().unwrap_or(target).trim();
        let display = html_escape(display.trim());

        // Check if page is denied (case-insensitive lookup)
        let denied_style = denied_pages
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(page))
            .map(|(_, v)| *v);

        let slug = slug_map
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(page))
            .map(|(_, v)| v.as_str());

        if let Some(style) = denied_style {
            let href_slug = slug.unwrap_or(page);
            match style {
                DeniedLinkStyle::GrayedOut => {
                    // transparent mode: grayed-out link, title visible, click → 403
                    format!(
                        r#"<a href="{root_path}{href}/{index_file}" class="wikilink wikilink-denied-transparent" style="opacity:0.5;color:gray;" title="Access denied">{display}</a>"#,
                        root_path = root_path,
                        href = urlencoding(href_slug),
                        index_file = index_file,
                        display = display,
                    )
                }
                DeniedLinkStyle::Locked => {
                    // mixed mode: lock icon, title visible, click → 403 (REQ-020-054)
                    format!(
                        "<a href=\"{root_path}{href}/{index_file}\" class=\"wikilink wikilink-denied-locked\" title=\"This page is restricted. Click to request access.\">\u{1f512} {display}</a>",
                        root_path = root_path,
                        href = urlencoding(href_slug),
                        index_file = index_file,
                        display = display,
                    )
                }
                DeniedLinkStyle::DeadLink => {
                    // hidden mode: dead link, indistinguishable from nonexistent
                    format!(
                        r#"<a href="{root_path}{href}/{index_file}" class="link-error wikilink wikilink-dead">{display}</a>"#,
                        root_path = root_path,
                        href = urlencoding(href_slug),
                        index_file = index_file,
                        display = display,
                    )
                }
            }
        } else if let Some(slug) = slug {
            format!(
                r#"<a href="{root_path}{href}/{index_file}" class="link link-primary wikilink">{display}</a>"#,
                root_path = root_path,
                href = urlencoding(slug),
                index_file = index_file,
                display = display,
            )
        } else {
            format!(
                r#"<a href="{root_path}{href}/{index_file}" class="link-error wikilink wikilink-dead">{display}</a>"#,
                root_path = root_path,
                href = urlencoding(page),
                index_file = index_file,
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

/// Extract a plain-text description suitable for `<meta name="description">`.
///
/// Preference order: frontmatter `description` field → first paragraph of body
/// (stripped of markdown syntax), truncated to ~160 chars on a word boundary.
pub fn extract_description(content: &str, frontmatter: &serde_json::Value) -> String {
    if let Some(s) = frontmatter.get("description").and_then(|v| v.as_str()) {
        return truncate_description(s);
    }
    let body = strip_frontmatter(content);
    let mut out = String::new();
    let mut in_code_fence = false;
    for line in body.lines() {
        let t = line.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_code_fence = !in_code_fence;
            continue;
        }
        if in_code_fence {
            continue;
        }
        if t.is_empty() || t.starts_with('#') || t.starts_with('>') || t.starts_with('|') {
            if !out.is_empty() {
                break;
            }
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(t);
        if out.len() >= 400 {
            break;
        }
    }
    truncate_description(&strip_inline_markdown(&out))
}

fn strip_inline_markdown(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        // ![alt](url) → drop
        if c == '!' && bytes.get(i + 1) == Some(&b'[') {
            if let Some(end) = s[i..].find(')') {
                i += end + 1;
                continue;
            }
        }
        // [[target|label]] or [[target]] → label/target
        if c == '[' && bytes.get(i + 1) == Some(&b'[') {
            if let Some(end) = s[i + 2..].find("]]") {
                let inner = &s[i + 2..i + 2 + end];
                let label = inner.split('|').next_back().unwrap_or(inner);
                out.push_str(label);
                i += 2 + end + 2;
                continue;
            }
        }
        // [text](url) → text
        if c == '[' {
            if let Some(close) = s[i + 1..].find(']') {
                let text = &s[i + 1..i + 1 + close];
                if bytes.get(i + 1 + close + 1) == Some(&b'(') {
                    if let Some(paren) = s[i + 1 + close + 2..].find(')') {
                        out.push_str(text);
                        i += 1 + close + 2 + paren + 1;
                        continue;
                    }
                }
            }
        }
        // inline `code` → strip backticks
        if c == '`' {
            if let Some(end) = s[i + 1..].find('`') {
                out.push_str(&s[i + 1..i + 1 + end]);
                i += 1 + end + 1;
                continue;
            }
        }
        // Emphasis markers
        if c == '*' || c == '_' {
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_description(s: &str) -> String {
    const MAX: usize = 160;
    let s = s.trim();
    if s.chars().count() <= MAX {
        return s.to_string();
    }
    let mut end = 0;
    for (count, (idx, _)) in s.char_indices().enumerate() {
        if count >= MAX {
            end = idx;
            break;
        }
    }
    let slice = &s[..end];
    let cut = slice.rfind(' ').unwrap_or(end);
    format!(
        "{}…",
        slice[..cut].trim_end_matches(|c: char| c.is_ascii_punctuation())
    )
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
    fn list_has_no_direct_anchor_children() {
        // Line anchors must land inside <li>, never as direct children of
        // <ul>/<ol>, or axe/Lighthouse flag a WCAG list-children violation.
        let slug_map = HashMap::new();
        let md = "Intro paragraph.\n\n- first item\n- [[Target]] item\n- third item\n\n1. numbered\n2. list\n";
        let html = render_to_html(md, &slug_map, "/", "");
        // Easy regex: no `<a` tag should appear immediately after `<ul>` or `<ol>`
        // without an intervening `<li>`.
        for open in ["<ul>", "<ol>"] {
            let mut pos = 0;
            while let Some(i) = html[pos..].find(open) {
                let after = &html[pos + i + open.len()..];
                // Skip whitespace.
                let trimmed = after.trim_start();
                assert!(
                    trimmed.starts_with("<li"),
                    "list element {open} followed by {:?} — would fail WCAG list-children",
                    &trimmed[..trimmed.len().min(60)]
                );
                pos += i + open.len();
            }
        }
    }

    #[test]
    fn test_simple_wikilink() {
        let mut slug_map = HashMap::new();
        slug_map.insert("Target".to_string(), "folder/target".to_string());
        let html = render_to_html("See [[Target]] here", &slug_map, "/", "");
        assert!(html.contains(r#"href="/folder/target/""#));
        assert!(html.contains("link-primary"));
    }

    #[test]
    fn spec045_predicate_renders_as_chip() {
        // A list-item named edge renders the predicate as a styled chip span
        // (auto-derived label) in front of a clean link — not raw `pred::` text.
        let mut slug_map = HashMap::new();
        slug_map.insert("Retro".to_string(), "retro".to_string());
        let html = render_to_html("- derived_from::[[Retro]]", &slug_map, "/", "");
        assert!(html.contains(
            r#"<span class="zetl-edge-predicate" data-predicate="derived_from">Derived from</span>"#
        ));
        // The raw `derived_from::` text must NOT leak into the body.
        assert!(!html.contains("derived_from::<a"));
        // The link itself stays clean.
        assert!(html.contains(r#"class="link link-primary wikilink">Retro</a>"#));
    }

    #[test]
    fn spec045_multiple_predicates_render_two_chips() {
        let mut slug_map = HashMap::new();
        slug_map.insert("Retro".to_string(), "retro".to_string());
        let html = render_to_html("- derived_from::informed_by::[[Retro]]", &slug_map, "/", "");
        assert!(html.contains(r#"data-predicate="derived_from">Derived from</span>"#));
        assert!(html.contains(r#"data-predicate="informed_by">Informed by</span>"#));
    }

    #[test]
    fn spec045_malformed_prefix_is_not_chipped() {
        // Render recognition matches the graph: a space in the predicate name
        // means no typed edge, so no chip — the text stays verbatim.
        let mut slug_map = HashMap::new();
        slug_map.insert("X".to_string(), "x".to_string());
        let html = render_to_html("- derived from::[[X]]", &slug_map, "/", "");
        assert!(!html.contains("zetl-edge-predicate"));
        assert!(html.contains("derived from::"));
    }

    #[test]
    fn spec045_curie_predicate_chip_uses_local_part() {
        let mut slug_map = HashMap::new();
        slug_map.insert("X".to_string(), "x".to_string());
        let html = render_to_html("- prov:wasDerivedFrom::[[X]]", &slug_map, "/", "");
        assert!(html.contains(
            r#"<span class="zetl-edge-predicate" data-predicate="prov:wasDerivedFrom">wasDerivedFrom</span>"#
        ));
    }

    #[test]
    fn test_aliased_wikilink() {
        let mut slug_map = HashMap::new();
        slug_map.insert("Target".to_string(), "folder/target".to_string());
        let html = render_to_html("See [[Target|click me]] here", &slug_map, "/", "");
        assert!(html.contains("click me"));
        assert!(html.contains(r#"href="/folder/target/""#));
    }

    #[test]
    fn test_dead_link() {
        let slug_map = HashMap::new();
        let html = render_to_html("See [[Missing]] here", &slug_map, "/", "");
        assert!(html.contains("link-error"));
    }

    #[test]
    fn test_wikilink_in_code_block_untouched() {
        let mut slug_map = HashMap::new();
        slug_map.insert("Target".to_string(), "folder/target".to_string());
        let html = render_to_html("```\n[[Target]]\n```", &slug_map, "/", "");
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
        let html = render_to_html("See [[Notes]] here", &slug_map, "/", "");
        assert!(html.contains(r#"href="/notes/""#));
    }

    #[test]
    fn test_kebab_case_slug() {
        let mut slug_map = HashMap::new();
        slug_map.insert(
            "Link Graph".to_string(),
            "architecture/link-graph".to_string(),
        );
        let html = render_to_html("See [[Link Graph]] here", &slug_map, "/", "");
        assert!(html.contains(r#"href="/architecture/link-graph/""#));
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

    // ── Visibility-aware wikilink rendering tests (TEST-020-032) ─────

    #[test]
    fn wikilink_grayed_out_in_transparent_mode() {
        let content = "Check [[Secret Project]] for details.";
        let mut slug_map = HashMap::new();
        slug_map.insert("Secret Project".to_string(), "secret-project".to_string());
        let mut denied = HashMap::new();
        denied.insert("Secret Project".to_string(), DeniedLinkStyle::GrayedOut);

        let html = render_to_html_with_visibility(content, &slug_map, &denied, "/", "");
        assert!(html.contains("wikilink-denied-transparent"));
        assert!(html.contains("opacity:0.5"));
        assert!(html.contains("Secret Project"));
        assert!(html.contains("/secret-project/"));
    }

    #[test]
    fn wikilink_locked_in_mixed_mode() {
        let content = "See [[Secret Project]] for info.";
        let mut slug_map = HashMap::new();
        slug_map.insert("Secret Project".to_string(), "secret-project".to_string());
        let mut denied = HashMap::new();
        denied.insert("Secret Project".to_string(), DeniedLinkStyle::Locked);

        let html = render_to_html_with_visibility(content, &slug_map, &denied, "/", "");
        assert!(html.contains("wikilink-denied-locked"));
        assert!(html.contains("This page is restricted. Click to request access."));
        assert!(html.contains("\u{1f512}")); // lock emoji
        assert!(html.contains("Secret Project"));
    }

    #[test]
    fn wikilink_dead_link_in_hidden_mode() {
        let content = "See [[Secret Project]] for info.";
        let mut slug_map = HashMap::new();
        slug_map.insert("Secret Project".to_string(), "secret-project".to_string());
        let mut denied = HashMap::new();
        denied.insert("Secret Project".to_string(), DeniedLinkStyle::DeadLink);

        let html = render_to_html_with_visibility(content, &slug_map, &denied, "/", "");
        assert!(html.contains("wikilink-dead"));
        assert!(html.contains("link-error"));
        // Should NOT contain lock or grayed styling
        assert!(!html.contains("wikilink-denied-locked"));
        assert!(!html.contains("wikilink-denied-transparent"));
    }

    #[test]
    fn preview_demotes_headings_by_two_levels() {
        // Transclusion previews live inside a page that already owns the <h1>.
        // Target page `# Title` must render as <h3>, not <h1>, so every chip
        // doesn't inject a top-level heading into the outline (see memo-index
        // where 36 <h1> elements were counted across previews).
        let slug_map = HashMap::new();
        let md = "# Top\n\n## Sub\n\n### Deep\n\n#### Deeper\n\n##### Deepest\n\n###### Bottom\n";
        let html = render_preview_html(md, &slug_map, "/", "");
        assert!(!html.contains("<h1"), "preview must not emit <h1>: {html}");
        assert!(!html.contains("<h2"), "preview must not emit <h2>: {html}");
        assert!(html.contains("<h3>Top</h3>"));
        assert!(html.contains("<h4>Sub</h4>"));
        assert!(html.contains("<h5>Deep</h5>"));
        assert!(html.contains("<h6>Deeper</h6>"));
        assert!(html.contains("<h6>Deepest</h6>"));
        assert!(html.contains("<h6>Bottom</h6>"));
    }

    #[test]
    fn allowed_pages_render_normally_with_denied_map() {
        let content = "See [[Public Page]] and [[Secret]].";
        let mut slug_map = HashMap::new();
        slug_map.insert("Public Page".to_string(), "public-page".to_string());
        slug_map.insert("Secret".to_string(), "secret".to_string());
        let mut denied = HashMap::new();
        denied.insert("Secret".to_string(), DeniedLinkStyle::Locked);

        let html = render_to_html_with_visibility(content, &slug_map, &denied, "/", "");
        // Public Page should have normal link styling
        assert!(html.contains("link link-primary wikilink"));
        assert!(html.contains("/public-page/"));
        // Secret should have locked styling
        assert!(html.contains("wikilink-denied-locked"));
    }

    #[test]
    fn slugify_heading_basics() {
        assert_eq!(slugify_heading("Writing SPL"), "writing-spl");
        assert_eq!(slugify_heading("What-if & Abduction"), "what-if-abduction");
        assert_eq!(slugify_heading("  lots   of   space  "), "lots-of-space");
        assert_eq!(slugify_heading("!!!"), "");
        assert_eq!(
            slugify_heading("Rules: normally and always"),
            "rules-normally-and-always"
        );
        // Unicode letters stay (URL-safe in modern browsers).
        assert_eq!(slugify_heading("Schrödinger's cat"), "schrödinger-s-cat");
    }

    #[test]
    fn heading_ids_are_injected_and_deduped() {
        let slug_map = HashMap::new();
        let md = "# Writing SPL\n\n## Facts\n\n## Facts\n\n### Related\n";
        let html = render_to_html(md, &slug_map, "/", "");
        assert!(
            html.contains(r#"<h1 id="writing-spl">"#),
            "h1 should get slug id; got: {html}"
        );
        assert!(
            html.contains(r#"<h2 id="facts">"#),
            "first duplicate should keep bare slug; got: {html}"
        );
        assert!(
            html.contains(r#"<h2 id="facts-1">"#),
            "second duplicate should get -1 suffix; got: {html}"
        );
        assert!(
            html.contains(r#"<h3 id="related">"#),
            "h3 should get slug id too; got: {html}"
        );
    }

    #[test]
    fn explicit_heading_id_is_preserved_and_blocks_collision() {
        // `## Custom {#mine}` should keep `mine`; a later `## Mine`
        // must auto-slug to `mine-1`, not clash.
        let slug_map = HashMap::new();
        let md = "## Custom {#mine}\n\n## Mine\n";
        let html = render_to_html(md, &slug_map, "/", "");
        assert!(
            html.contains(r#"id="mine""#),
            "explicit id kept; got: {html}"
        );
        assert!(
            html.contains(r#"id="mine-1""#),
            "auto-slug should dodge explicit id; got: {html}"
        );
    }

    #[test]
    fn line_anchors_coexist_with_heading_ids() {
        let slug_map = HashMap::new();
        let md = "# A Heading\n\nbody\n";
        let html = render_to_html(md, &slug_map, "/", "");
        assert!(
            html.contains(r#"<h1 id="a-heading">"#),
            "heading id present; got: {html}"
        );
        assert!(
            html.contains(r#"<a id="line-1" class="line-anchor">"#),
            "line anchor still emitted inside heading; got: {html}"
        );
    }
}
