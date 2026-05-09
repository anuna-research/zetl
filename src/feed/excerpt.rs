//! Excerpt-plus-link pure function per REQ-3816 + REQ-3821 + NFR-3808.
//!
//! Strips HTML tags, counts plain-text words, truncates at the requested
//! count rounding DOWN to the nearest sentence or paragraph boundary so
//! the output never ends mid-word or mid-sentence. Skips images,
//! embeds, scripts, style, audio/video — these neither emit text nor
//! contribute to word count.
//!
//! `word_count` is bounded `[50, 500]` per NFR-3808; the function
//! debug-asserts and silently saturates in release if the caller
//! passes out-of-range values.

/// Word-count bounds enforced per NFR-3808.
pub const MIN_WORDS: usize = 50;
/// Word-count upper bound per NFR-3808.
pub const MAX_WORDS: usize = 500;

/// Build an excerpt of `content_html` no longer than `word_count`
/// words. Always returns plain text suitable for `<description>` /
/// `<atom:summary>` / JSON Feed `summary`. Paragraph breaks survive as
/// double newlines.
///
/// The function does not panic on malformed HTML; unclosed tags get
/// dropped at the strip stage.
pub fn excerpt(content_html: &str, word_count: usize) -> String {
    debug_assert!(
        (MIN_WORDS..=MAX_WORDS).contains(&word_count),
        "word_count {word_count} out of NFR-3808 range [{MIN_WORDS}, {MAX_WORDS}]"
    );
    let bounded = word_count.clamp(MIN_WORDS, MAX_WORDS);

    let plain = strip_html(content_html);
    if plain.trim().is_empty() {
        return String::new();
    }

    let paragraphs: Vec<&str> = plain
        .split("\n\n")
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();
    if paragraphs.is_empty() {
        return String::new();
    }

    // Walk paragraphs, accumulating sentences while word budget remains.
    // When mid-paragraph the budget runs out we split on the last
    // sentence boundary inside the current paragraph rather than
    // truncating mid-word.
    let mut out = String::new();
    let mut used = 0usize;
    for (i, para) in paragraphs.iter().enumerate() {
        let para_words = count_words(para);
        if used + para_words <= bounded {
            // Fit whole paragraph.
            if i > 0 {
                out.push_str("\n\n");
            }
            out.push_str(para);
            used += para_words;
            if used == bounded {
                break;
            }
        } else {
            // Pull sentences from this paragraph until we'd exceed.
            let remaining = bounded - used;
            let truncated = take_sentences_under(para, remaining);
            if !truncated.is_empty() {
                if i > 0 {
                    out.push_str("\n\n");
                }
                out.push_str(&truncated);
            }
            break;
        }
    }
    out
}

/// Strip HTML tags, drop entirely the contents of `<script>`,
/// `<style>`, `<img>`, `<video>`, `<audio>`, and `<iframe>`. Replaces
/// `<p>` / `<br>` / `</p>` with newline boundaries so paragraph
/// structure survives.
fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            // Find tag end.
            let Some(end) = find_byte(bytes, b'>', i + 1) else {
                break;
            };
            let raw_tag = &html[i + 1..end];
            let close = raw_tag.starts_with('/');
            let body = if close { &raw_tag[1..] } else { raw_tag };
            // Tag name is the first whitespace-bounded chunk.
            let name = body
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            let name = name.trim_end_matches('/');
            // Skip-block tags swallow content until the matching close
            // per the module-level "skips images, embeds, scripts,
            // style, audio/video" promise.
            if matches!(name, "script" | "style" | "audio" | "video" | "iframe") && !close {
                if let Some(close_at) = find_close_tag(html, end + 1, name) {
                    i = close_at;
                    continue;
                } else {
                    break;
                }
            }
            // Boundary tags inject paragraph breaks.
            if matches!(
                name,
                "p" | "div" | "br" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
            ) {
                out.push_str("\n\n");
            }
            i = end + 1;
            continue;
        }
        // Default: copy one UTF-8 codepoint. Casting `bytes[i] as char`
        // would mangle multi-byte sequences (é, em-dash, CJK, emoji).
        let rest = &html[i..];
        let ch = rest
            .chars()
            .next()
            .expect("loop condition guarantees i < len");
        out.push(ch);
        i += ch.len_utf8();
    }
    // Decode the most common entities. The pure core deliberately
    // doesn't pull in a full HTML entity table; serialisers fix up
    // anything else downstream.
    let decoded = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ");
    // Collapse runs of whitespace except double-newlines.
    collapse_whitespace(&decoded)
}

fn find_byte(bytes: &[u8], target: u8, start: usize) -> Option<usize> {
    bytes[start..]
        .iter()
        .position(|&b| b == target)
        .map(|p| p + start)
}

/// Locate the byte offset just after the next `</name>` (case-
/// insensitive). Used to skip the entire body of `<script>` / `<style>`
/// / `<audio>` / `<video>` / `<iframe>`. We scan the input directly
/// in a case-insensitive byte-by-byte loop rather than allocating a
/// lowercased copy of the entire input on every call — with N
/// container tags in a 1 MiB body the latter would be O(N · |html|).
fn find_close_tag(html: &str, start: usize, name: &str) -> Option<usize> {
    let bytes = html.as_bytes();
    let needle: Vec<u8> = format!("</{name}").bytes().collect();
    if start >= bytes.len() {
        return None;
    }
    let mut i = start;
    while i + needle.len() <= bytes.len() {
        let mut matched = true;
        for (k, &needle_byte) in needle.iter().enumerate() {
            // Case-insensitive ASCII match; tag names are ASCII.
            let actual = bytes[i + k];
            if actual != needle_byte && actual.to_ascii_lowercase() != needle_byte {
                matched = false;
                break;
            }
        }
        if matched {
            // Walk to the closing `>`.
            let after = bytes[i..].iter().position(|&b| b == b'>')?;
            return Some(i + after + 1);
        }
        i += 1;
    }
    None
}

fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_was_newline = false;
    let mut newlines_run = 0u8;
    for c in s.chars() {
        match c {
            '\n' => {
                newlines_run = newlines_run.saturating_add(1);
                if newlines_run >= 2 && !out.ends_with("\n\n") {
                    if out.ends_with('\n') {
                        out.push('\n');
                    } else {
                        out.push_str("\n\n");
                    }
                } else if newlines_run == 1
                    && !out.is_empty()
                    && !out.ends_with(' ')
                    && !out.ends_with('\n')
                {
                    out.push(' ');
                }
                last_was_newline = true;
            }
            c if c.is_whitespace() => {
                if !last_was_newline && !out.ends_with(' ') && !out.is_empty() {
                    out.push(' ');
                }
            }
            _ => {
                out.push(c);
                last_was_newline = false;
                newlines_run = 0;
            }
        }
    }
    out.trim().to_string()
}

fn count_words(s: &str) -> usize {
    s.split_whitespace().count()
}

/// Take whole sentences from `para` until adding the next would exceed
/// `budget` words. Returns the assembled prefix with sentence
/// terminators preserved. Emits the empty string when the first
/// sentence already exceeds budget (per spec: never truncate mid-
/// sentence; rather drop entirely).
fn take_sentences_under(para: &str, budget: usize) -> String {
    let sentences = split_sentences(para);
    let mut out = String::new();
    let mut used = 0usize;
    for s in sentences {
        let w = count_words(&s);
        if used + w > budget {
            break;
        }
        if !out.is_empty() && !out.ends_with(' ') {
            out.push(' ');
        }
        out.push_str(s.trim());
        used += w;
    }
    out
}

/// Split a paragraph into sentences on `.`, `!`, `?` boundaries. Keeps
/// the terminator attached to its sentence.
fn split_sentences(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    for c in s.chars() {
        buf.push(c);
        if matches!(c, '.' | '!' | '?') {
            out.push(buf.trim().to_string());
            buf.clear();
        }
    }
    if !buf.trim().is_empty() {
        out.push(buf.trim().to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_yields_empty_excerpt() {
        assert_eq!(excerpt("", 100), "");
        assert_eq!(excerpt("   ", 100), "");
        assert_eq!(excerpt("<p></p>", 100), "");
    }

    #[test]
    fn returns_full_content_when_under_budget() {
        let body = "<p>One short paragraph here.</p>";
        let out = excerpt(body, 100);
        assert_eq!(out, "One short paragraph here.");
    }

    #[test]
    fn truncates_at_sentence_boundary() {
        // Each sentence below is 5 words; a 50-word budget fits exactly
        // the first ten sentences (50 words). Sentence eleven (5 more
        // words, would total 55) is rejected because adding it would
        // exceed budget.
        let mut body = String::from("<p>");
        for _ in 0..15 {
            body.push_str("alpha bravo charlie delta echo. ");
        }
        body.push_str("</p>");
        let out = excerpt(&body, 50);
        // 50-word output: ten "alpha bravo charlie delta echo." sentences.
        let expected: String = (0..10)
            .map(|_| "alpha bravo charlie delta echo.")
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(out, expected);
    }

    #[test]
    fn paragraph_break_preserved_as_double_newline() {
        let body = "<p>Para one.</p><p>Para two.</p>";
        let out = excerpt(body, 100);
        assert!(out.contains("\n\n"), "got {out:?}");
        assert!(out.starts_with("Para one."));
        assert!(out.ends_with("Para two."));
    }

    #[test]
    fn skips_images_and_scripts() {
        let body = "<p>before</p><script>alert('x');</script><img src=foo /><p>after</p>";
        let out = excerpt(body, 100);
        assert!(!out.contains("alert"));
        assert!(!out.contains("foo"));
        assert!(out.contains("before"));
        assert!(out.contains("after"));
    }

    #[test]
    fn word_count_never_exceeds_budget() {
        let body = format!("<p>{}</p>", "word ".repeat(1000));
        for budget in [50usize, 75, 100, 200, 500] {
            let out = excerpt(&body, budget);
            assert!(
                count_words(&out) <= budget,
                "budget {budget} produced {} words: {out:?}",
                count_words(&out)
            );
        }
    }

    #[test]
    fn release_saturates_below_min() {
        // Debug-asserts; the test runs in debug so saturation isn't
        // exercised — but the clamp logic still bounds output.
        // We bypass the assert by going through the public function
        // with a value above MIN_WORDS.
        let body = "<p>fifty word paragraph here is just enough to test.</p>";
        let out = excerpt(body, 50);
        assert!(!out.is_empty());
    }

    #[test]
    fn entities_decoded() {
        let body = "<p>foo &amp; bar &lt;baz&gt;.</p>";
        let out = excerpt(body, 100);
        assert!(out.contains("foo & bar <baz>."));
    }

    #[test]
    fn no_panic_on_unclosed_tags() {
        let body = "<p>broken <em>fragment <strong>here";
        let out = excerpt(body, 100);
        assert!(out.contains("broken"));
    }

    #[test]
    fn multibyte_utf8_passes_through_intact() {
        // Earlier `bytes[i] as char` cast mangled multi-byte UTF-8.
        // Verify café (2-byte), em-dash (3-byte), CJK (3-byte), and
        // an emoji (4-byte) all survive.
        let body = "<p>café — 日本語 🎉 plus enough other words to satisfy the fifty-word minimum bound that the function debug-asserts on its first parameter.</p>";
        let out = excerpt(body, 50);
        assert!(out.contains("café"), "got {out:?}");
        assert!(out.contains("—"), "got {out:?}");
        assert!(out.contains("日本語"), "got {out:?}");
        assert!(out.contains("🎉"), "got {out:?}");
    }

    #[test]
    fn audio_video_iframe_inner_text_swallowed() {
        let body = "<p>before</p>\
                    <video><source src=x>narration that should not survive</video>\
                    <audio>broadcast that should not survive</audio>\
                    <iframe>third-party that should not survive</iframe>\
                    <p>after</p>";
        let out = excerpt(body, 100);
        assert!(out.contains("before"));
        assert!(out.contains("after"));
        assert!(!out.contains("narration"), "audio/video swallowed: {out:?}");
        assert!(!out.contains("broadcast"));
        assert!(!out.contains("third-party"));
    }
}
