use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::SystemTime;

/// An extracted SPL block from a Markdown file or standalone `.spl` file.
///
/// For Markdown files, this captures the raw text between `` ```spl `` / `` ```spindle ``
/// fences with provenance. For `.spl` files, the entire file content is captured.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplBlock {
    /// Relative path from vault root.
    pub source_file: PathBuf,
    /// Page name (filename sans extension).
    pub source_page: String,
    /// 1-indexed line of opening fence (or 1 for `.spl` files).
    pub start_line: u32,
    /// 1-indexed line of closing fence (or last line for `.spl` files).
    pub end_line: u32,
    /// Raw SPL text between fences (or entire file for `.spl`).
    pub content: String,
}

/// A single extracted wikilink occurrence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiLink {
    /// Resolved/normalized page name (before # or ^)
    pub target_page: String,
    /// Original text inside [[ ]]
    pub raw_target: String,
    /// #heading reference
    pub heading: Option<String>,
    /// ^block-id reference
    pub block_ref: Option<String>,
    /// Display text after |
    pub alias: Option<String>,
    /// Preceded by !
    pub is_embed: bool,
    /// 1-indexed line number
    pub line: u32,
    /// 1-indexed column
    pub column: u32,
}

/// Parsed result for a single file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedFile {
    /// Relative path from vault root
    pub path: PathBuf,
    /// Page name derived from filename (sans .md)
    pub page_name: String,
    /// Extracted wikilinks
    pub links: Vec<WikiLink>,
    /// Extracted SPL blocks (from ```spl/```spindle fences or standalone .spl files)
    pub spl_blocks: Vec<SplBlock>,
    /// Syntax warnings/errors
    pub diagnostics: Vec<Diagnostic>,
    /// File modification time
    #[serde(with = "system_time_serde")]
    pub mtime: SystemTime,
    /// Merkle leaf nodes built from block-level AST events (SPEC-006 §4.3)
    #[serde(default)]
    pub merkle_leaves: Vec<MerkleLeaf>,
    /// Per-file Merkle tree with root hash (SPEC-006 §4.2). `None` for files
    /// that have not been hashed yet (e.g. standalone `.spl` files).
    #[serde(default)]
    pub file_merkle: Option<FileMerkle>,
}

/// A syntax issue
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub level: DiagnosticLevel,
    pub message: String,
    pub file: PathBuf,
    pub line: u32,
    pub column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticLevel {
    Error,
    Warning,
}

mod system_time_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    pub fn serialize<S: Serializer>(time: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
        let duration = time.duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO);
        duration.as_secs_f64().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SystemTime, D::Error> {
        let secs = f64::deserialize(d)?;
        Ok(UNIX_EPOCH + Duration::from_secs_f64(secs))
    }
}

// ── SPEC-006 Merkle types ─────────────────────────────────────────────────────

/// 32-byte BLAKE3 hash, serialised as a 64-character lowercase hex string.
pub type ContentHash = [u8; 32];

/// Serde module that serialises `ContentHash` ([u8; 32]) as a lowercase hex string.
pub mod content_hash_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(hash: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        let hex: String = hash.iter().map(|b| format!("{:02x}", b)).collect();
        hex.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let hex = String::deserialize(d)?;
        if hex.len() != 64 {
            return Err(serde::de::Error::custom(format!(
                "expected 64 hex chars, got {}",
                hex.len()
            )));
        }
        let mut bytes = [0u8; 32];
        for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
            let high = hex_nibble(chunk[0]).map_err(serde::de::Error::custom)?;
            let low = hex_nibble(chunk[1]).map_err(serde::de::Error::custom)?;
            bytes[i] = (high << 4) | low;
        }
        Ok(bytes)
    }

    fn hex_nibble(c: u8) -> Result<u8, &'static str> {
        match c {
            b'0'..=b'9' => Ok(c - b'0'),
            b'a'..=b'f' => Ok(c - b'a' + 10),
            b'A'..=b'F' => Ok(c - b'A' + 10),
            _ => Err("invalid hex character"),
        }
    }
}

/// The semantic type of a Markdown block node in the Merkle leaf tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum LeafType {
    Heading { level: u8 },
    Paragraph,
    CodeBlock { language: Option<String> },
    SplBlock,
    List { ordered: bool },
    BlockQuote,
    Table,
    Frontmatter,
    ThematicBreak,
    HtmlBlock,
}

/// Pair of hashes for a Merkle leaf that contains an SPL block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplLeafHash {
    #[serde(with = "content_hash_serde")]
    pub content_hash: ContentHash,
    #[serde(with = "content_hash_serde")]
    pub ast_hash: ContentHash,
}

/// A single leaf in the Merkle leaf tree, corresponding to one Markdown block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleLeaf {
    pub node_type: LeafType,
    pub start_line: u32,
    pub end_line: u32,
    #[serde(with = "content_hash_serde")]
    pub hash: ContentHash,
    pub spl_hashes: Option<SplLeafHash>,
    /// Obsidian block-id annotation (`^identifier`) extracted from the leaf's
    /// normalised content (REQ-042b, REQ-042c).  `None` if the leaf carries no
    /// block-id annotation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
}

/// A heading-delimited section within a file's Merkle tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Section {
    pub heading_line: u32,
    pub heading_text: String,
    pub heading_level: u8,
    /// Half-open range `[start, end)` into `FileMerkle::spl_leaves`.
    pub leaf_range: (usize, usize),
    #[serde(with = "content_hash_serde")]
    pub grounding_hash: ContentHash,
}

/// Merkle tree for a single file, stored in the cache.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMerkle {
    #[serde(with = "content_hash_serde")]
    pub root_hash: ContentHash,
    pub sections: Vec<Section>,
    pub spl_leaves: Vec<SplLeafCached>,
}

/// A cached SPL leaf with resolved grounding metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplLeafCached {
    pub start_line: u32,
    #[serde(with = "content_hash_serde")]
    pub content_hash: ContentHash,
    #[serde(with = "content_hash_serde")]
    pub ast_hash: ContentHash,
    pub section_index: usize,
    pub explicit_groundings: Vec<ExplicitGrounding>,
}

/// An explicit grounding declaration attached to an SPL block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplicitGrounding {
    pub construct: String,
    pub source_refs: Vec<String>,
    pub targets: Vec<ResolvedTarget>,
}

/// A resolved cross-file grounding target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedTarget {
    pub source_ref: String,
    pub target_file: PathBuf,
    #[serde(with = "content_hash_serde")]
    pub target_leaf_hash: ContentHash,
}

/// A drift diagnostic emitted when a grounding target has drifted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftDiagnostic {
    pub file: PathBuf,
    pub spl_line: u32,
    pub drift_type: DriftType,
    pub severity: DriftSeverity,
    pub message: String,
}

/// The category of drift detected.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DriftType {
    SectionDrift {
        section_heading: String,
    },
    ExplicitDrift {
        construct: String,
        source_ref: String,
    },
}

/// Severity level for a drift diagnostic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DriftSeverity {
    Info,
    Warning,
}
