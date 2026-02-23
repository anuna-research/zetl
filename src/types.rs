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
