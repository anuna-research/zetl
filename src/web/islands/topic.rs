//! SPEC-050 CON-5001 — topic-name recogniser (the single shared build≡runtime grammar).
//!
//! A topic is either a **content-topic** (`content:` prefixed — reserved for content
//! authors) or a **trusted-topic** (any first segment except the literal `content`).
//! The two productions are disjoint by construction, so a topic's trust domain is a
//! decidable property of its name (REQ-5011).
//!
//! Grammar (anchored whole-string):
//! ```text
//! topic         = content-topic | trusted-topic
//! content-topic = "content" ":" segment { ":" segment }
//! trusted-topic = trusted-first { ":" segment }
//! trusted-first = segment \ "content"            (any segment except literal "content")
//! segment       = lower { alnum | "-" } alnum    (>=2 chars, lower-first, no trailing "-")
//! ```
//! Bounds: ASCII, <=128 chars total, <=8 segments. Malformed → `island-topic-malformed`.

use super::IslandError;

/// The trust domain of a recognised topic (REQ-5011).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopicKind {
    /// `content:`-prefixed — publishable by content-author islands.
    Content,
    /// No `content:` prefix — publishable only by trusted in-realm islands.
    Trusted,
}

const MAX_TOPIC_LEN: usize = 128;
const MAX_SEGMENTS: usize = 8;

/// Recognise a single segment: `lower { alnum | "-" } alnum` (≥2 chars, lower-first, no
/// trailing `-`, ASCII).
fn is_segment(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() < 2 {
        return false;
    }
    if !b[0].is_ascii_lowercase() {
        return false;
    }
    let last = b[b.len() - 1];
    if !(last.is_ascii_lowercase() || last.is_ascii_digit()) {
        return false;
    }
    b[1..]
        .iter()
        .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

fn malformed(topic: &str) -> IslandError {
    IslandError::new(
        "island-topic-malformed",
        format!("topic `{topic}` is not a valid CON-5001 topic name"),
    )
}

/// Recognise a topic name, returning its trust domain or `island-topic-malformed`.
pub fn recognise_topic(topic: &str) -> Result<TopicKind, IslandError> {
    if topic.is_empty() || topic.len() > MAX_TOPIC_LEN || !topic.is_ascii() {
        return Err(malformed(topic));
    }
    let segments: Vec<&str> = topic.split(':').collect();
    if segments.is_empty() || segments.len() > MAX_SEGMENTS {
        return Err(malformed(topic));
    }
    if !segments.iter().all(|s| is_segment(s)) {
        return Err(malformed(topic));
    }
    // exactly one production matches: `content:...` (>=2 segments) → Content, else Trusted.
    if segments[0] == "content" {
        if segments.len() < 2 {
            // bare `content` matches neither production (content-topic needs a `:`).
            return Err(malformed(topic));
        }
        Ok(TopicKind::Content)
    } else {
        Ok(TopicKind::Trusted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_topics() {
        assert_eq!(recognise_topic("content:filter").unwrap(), TopicKind::Content);
        assert_eq!(recognise_topic("content:poll:vote").unwrap(), TopicKind::Content);
    }

    #[test]
    fn trusted_topics() {
        assert_eq!(recognise_topic("theme").unwrap(), TopicKind::Trusted);
        assert_eq!(recognise_topic("nav:state").unwrap(), TopicKind::Trusted);
    }

    #[test]
    fn bare_content_rejected() {
        assert!(recognise_topic("content").is_err());
    }

    #[test]
    fn malformed_rejected() {
        for t in [
            "",
            "Theme",        // uppercase
            "a",            // single char segment
            "nav-",         // trailing hyphen
            "1nav",         // leading digit
            "nav::state",   // empty segment
            "a:b:c:d:e:f:g:h:i", // >8 segments
        ] {
            assert!(recognise_topic(t).is_err(), "{t} should be malformed");
        }
    }

    #[test]
    fn over_length_rejected() {
        let long = format!("a{}", "b".repeat(200));
        assert!(recognise_topic(&long).is_err());
    }
}
