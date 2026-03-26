/// Block-level tokens in the CRDT sequence (REQ-020-026).
///
/// Block-level structure is handled as indivisible atomic tokens.
/// Each block boundary is a single atomic token that cannot be
/// concurrently split by different users.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockToken {
    /// Heading prefix, e.g. `## ` for h2. Level is 1–6.
    Heading { level: u8 },
    /// Unordered list marker `- `.
    UnorderedListItem,
    /// Ordered list marker, e.g. `1. `.
    OrderedListItem { number: u32 },
    /// Code fence boundary (opening or closing). Stores the info string if opening.
    CodeFence { info: Option<String> },
    /// Frontmatter boundary (`---`). Always at position 0 in the document.
    Frontmatter,
    /// Blank line separator between blocks.
    BlankLine,
}

impl BlockToken {
    /// The raw markdown string for this block token.
    pub fn to_markdown(&self) -> String {
        match self {
            Self::Heading { level } => {
                let hashes = "#".repeat(*level as usize);
                format!("{hashes} ")
            }
            Self::UnorderedListItem => "- ".to_string(),
            Self::OrderedListItem { number } => format!("{number}. "),
            Self::CodeFence { info } => match info {
                Some(i) => format!("```{i}"),
                None => "```".to_string(),
            },
            Self::Frontmatter => "---".to_string(),
            Self::BlankLine => String::new(),
        }
    }

    /// Parse a block token from a line prefix. Returns the token and the
    /// remaining content after the prefix (if any).
    pub fn parse_line_prefix(line: &str) -> Option<(Self, &str)> {
        // Frontmatter delimiter
        if line == "---" {
            return Some((Self::Frontmatter, ""));
        }

        // Code fence
        if let Some(rest) = line.strip_prefix("```") {
            let info = if rest.is_empty() {
                None
            } else {
                Some(rest.to_string())
            };
            return Some((Self::CodeFence { info }, ""));
        }

        // Heading (# through ######)
        if line.starts_with('#') {
            let level = line.bytes().take_while(|&b| b == b'#').count();
            if level <= 6 {
                let rest = &line[level..];
                if let Some(content) = rest.strip_prefix(' ') {
                    return Some((Self::Heading { level: level as u8 }, content));
                }
            }
        }

        // Unordered list
        if let Some(content) = line.strip_prefix("- ") {
            return Some((Self::UnorderedListItem, content));
        }

        // Ordered list (e.g. "1. ")
        let digit_count = line.bytes().take_while(|b| b.is_ascii_digit()).count();
        if digit_count > 0 {
            let rest = &line[digit_count..];
            if let Some(content) = rest.strip_prefix(". ") {
                if let Ok(number) = line[..digit_count].parse::<u32>() {
                    return Some((Self::OrderedListItem { number }, content));
                }
            }
        }

        None
    }

    /// Whether this token starts an opaque range where formatting marks
    /// should not be applied (code fences, frontmatter).
    pub fn is_opaque(&self) -> bool {
        matches!(self, Self::CodeFence { .. } | Self::Frontmatter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_round_trip() {
        for level in 1..=6u8 {
            let tok = BlockToken::Heading { level };
            let md = tok.to_markdown();
            let line = format!("{md}Some heading");
            let (parsed, content) = BlockToken::parse_line_prefix(&line).unwrap();
            assert_eq!(parsed, tok);
            assert_eq!(content, "Some heading");
        }
    }

    #[test]
    fn list_items() {
        let (tok, content) = BlockToken::parse_line_prefix("- item text").unwrap();
        assert_eq!(tok, BlockToken::UnorderedListItem);
        assert_eq!(content, "item text");

        let (tok, content) = BlockToken::parse_line_prefix("3. ordered").unwrap();
        assert_eq!(tok, BlockToken::OrderedListItem { number: 3 });
        assert_eq!(content, "ordered");
    }

    #[test]
    fn code_fence_with_info() {
        let (tok, _) = BlockToken::parse_line_prefix("```spl").unwrap();
        assert_eq!(
            tok,
            BlockToken::CodeFence {
                info: Some("spl".into())
            }
        );
        assert!(tok.is_opaque());
    }

    #[test]
    fn code_fence_without_info() {
        let (tok, _) = BlockToken::parse_line_prefix("```").unwrap();
        assert_eq!(tok, BlockToken::CodeFence { info: None });
    }

    #[test]
    fn frontmatter_is_opaque() {
        let (tok, _) = BlockToken::parse_line_prefix("---").unwrap();
        assert_eq!(tok, BlockToken::Frontmatter);
        assert!(tok.is_opaque());
    }

    #[test]
    fn plain_text_returns_none() {
        assert!(BlockToken::parse_line_prefix("Just some text").is_none());
        assert!(BlockToken::parse_line_prefix("").is_none());
    }
}
