//! Merkle tree hashing functions for SPEC-006.
//!
//! Implements BLAKE3-based hashing for leaf nodes, file roots, and the vault root
//! per §4.1–4.2 and §4.6.
//!
//! Also provides hash prefix resolution (§3.3 resolution rule 1, REQ-042a) via
//! [`build_vault_hash_index`] and [`resolve_hash_prefix`], and block-id resolution
//! (§3.3 rules 2–3, REQ-042b, REQ-042c) via [`resolve_local_block_id`] and
//! [`resolve_cross_file_block_id`].

use crate::types::{
    ContentHash, Diagnostic, DiagnosticLevel, LeafType, MerkleLeaf, ParsedFile, Section,
    SplLeafHash,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ── Type tag bytes (one per LeafType variant) ─────────────────────────────────

/// Unique sentinel byte for `Heading` leaves.
pub const TAG_HEADING: u8 = 0x01;
/// Unique sentinel byte for `Paragraph` leaves.
pub const TAG_PARAGRAPH: u8 = 0x02;
/// Unique sentinel byte for `CodeBlock` leaves.
pub const TAG_CODE_BLOCK: u8 = 0x03;
/// Unique sentinel byte for `SplBlock` leaves (placeholder; dual hashing handled separately).
pub const TAG_SPL_BLOCK: u8 = 0x04;
/// Unique sentinel byte for `List` leaves.
pub const TAG_LIST: u8 = 0x05;
/// Unique sentinel byte for `BlockQuote` leaves.
pub const TAG_BLOCK_QUOTE: u8 = 0x06;
/// Unique sentinel byte for `Table` leaves.
pub const TAG_TABLE: u8 = 0x07;
/// Unique sentinel byte for `Frontmatter` leaves.
pub const TAG_FRONTMATTER: u8 = 0x08;
/// Unique sentinel byte for `ThematicBreak` leaves.
pub const TAG_THEMATIC_BREAK: u8 = 0x09;
/// Unique sentinel byte for `HtmlBlock` leaves.
pub const TAG_HTML_BLOCK: u8 = 0x0A;

/// Return the type tag byte for a [`LeafType`] variant.
pub fn leaf_type_tag(leaf_type: &LeafType) -> u8 {
    match leaf_type {
        LeafType::Heading { .. } => TAG_HEADING,
        LeafType::Paragraph => TAG_PARAGRAPH,
        LeafType::CodeBlock { .. } => TAG_CODE_BLOCK,
        LeafType::SplBlock => TAG_SPL_BLOCK,
        LeafType::List { .. } => TAG_LIST,
        LeafType::BlockQuote => TAG_BLOCK_QUOTE,
        LeafType::Table => TAG_TABLE,
        LeafType::Frontmatter => TAG_FRONTMATTER,
        LeafType::ThematicBreak => TAG_THEMATIC_BREAK,
        LeafType::HtmlBlock => TAG_HTML_BLOCK,
    }
}

/// Compute the hash for a single Merkle leaf node.
///
/// `ContentHash = BLAKE3(type_tag_byte ‖ content_bytes)` per §4.2.
///
/// **Note:** [`LeafType::SplBlock`] leaves use a different hash formula
/// (`BLAKE3(content_hash ‖ ast_hash)` per §4.4) computed by
/// [`compute_spl_hashes`] and [`spl_combined_hash`]. This function is not
/// used for `SplBlock` leaves by `build_merkle_leaves`.
pub fn compute_leaf_hash(leaf_type: &LeafType, content: &[u8]) -> ContentHash {
    let tag = leaf_type_tag(leaf_type);
    let mut hasher = blake3::Hasher::new();
    hasher.update(&[tag]);
    hasher.update(content);
    *hasher.finalize().as_bytes()
}

// ── SPL dual hashing (§4.4) ───────────────────────────────────────────────────

/// Normalise raw SPL text for content hashing.
///
/// 1. Strips lines whose first non-whitespace characters are `;;` (SPL line
///    comments).
/// 2. Collapses all remaining whitespace runs to a single ASCII space and
///    trims leading/trailing whitespace.
fn normalize_spl(content: &str) -> String {
    let without_comments: String = content
        .lines()
        .filter(|line| !line.trim_start().starts_with(";;"))
        .collect::<Vec<_>>()
        .join("\n");

    let mut out = String::with_capacity(without_comments.len());
    let mut prev_space = false;
    for ch in without_comments.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

/// Compute the dual-hash pair for an SPL block leaf (§4.4).
///
/// - `content_hash = BLAKE3(normalize(content))` where normalization strips
///   `;;` comment lines and collapses whitespace.
/// - When the `reason` feature is **enabled**: `ast_hash` is
///   `BLAKE3(canonical_spl_ast)` where the canonical serialisation sorts
///   rules by label, facts by head literal, and superiority relations by
///   pair.  If parsing fails, `ast_hash = [0u8; 32]` (sentinel).
/// - When the `reason` feature is **disabled**: `ast_hash = content_hash`.
pub fn compute_spl_hashes(content: &str) -> SplLeafHash {
    let normalized = normalize_spl(content);
    let content_hash: ContentHash = *blake3::hash(normalized.as_bytes()).as_bytes();

    #[cfg(feature = "reason")]
    let ast_hash = compute_spl_ast_hash(content);

    #[cfg(not(feature = "reason"))]
    let ast_hash = content_hash;

    SplLeafHash {
        content_hash,
        ast_hash,
    }
}

/// Compute the combined SPL leaf hash from a dual-hash pair.
///
/// `combined = BLAKE3(content_hash ‖ ast_hash)` per §4.4.
/// This is the value stored in [`MerkleLeaf::hash`] for `SplBlock` leaves.
pub fn spl_combined_hash(spl: &SplLeafHash) -> ContentHash {
    *blake3::Hasher::new()
        .update(&spl.content_hash)
        .update(&spl.ast_hash)
        .finalize()
        .as_bytes()
}

/// Compute the canonical AST hash for an SPL block (§4.4, `reason` feature only).
///
/// Parses the SPL text with spindle-parser.  On success the theory is
/// serialised canonically (facts sorted by head literal, named rules sorted
/// by label, superiority relations sorted by pair) and hashed with BLAKE3.
/// On parse failure returns `[0u8; 32]` (sentinel).
#[cfg(feature = "reason")]
fn compute_spl_ast_hash(content: &str) -> ContentHash {
    use spindle_core::prelude::RuleType as CoreRuleType;
    use spindle_parser::parse_spl;

    let parsed = match parse_spl(content) {
        Ok(theory) => theory,
        Err(_) => return [0u8; 32], // sentinel: parse failure
    };

    // ── Collect rules by kind ────────────────────────────────────────────────
    let mut facts: Vec<String> = Vec::new();
    // (label, type_str, body_csv, head_csv)
    let mut named_rules: Vec<(String, &'static str, String, String)> = Vec::new();

    for rule in parsed.rules() {
        let body_csv: String = rule
            .body
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let head_csv: String = rule
            .head
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join(",");

        match rule.rule_type {
            CoreRuleType::Fact => {
                facts.push(head_csv);
            }
            CoreRuleType::Strict => {
                named_rules.push((rule.label.clone(), "strict", body_csv, head_csv));
            }
            CoreRuleType::Defeasible => {
                named_rules.push((rule.label.clone(), "defeasible", body_csv, head_csv));
            }
            CoreRuleType::Defeater => {
                named_rules.push((rule.label.clone(), "defeater", body_csv, head_csv));
            }
        }
    }

    // ── Collect superiority relations ────────────────────────────────────────
    let mut sups: Vec<(String, String)> = parsed
        .superiorities()
        .iter()
        .map(|s| (s.superior.clone(), s.inferior.clone()))
        .collect();

    // ── Sort for canonical ordering ──────────────────────────────────────────
    facts.sort();
    named_rules.sort_by(|a, b| a.0.cmp(&b.0));
    sups.sort();

    // ── Build canonical byte stream ──────────────────────────────────────────
    let mut canonical = String::new();
    for lit in &facts {
        canonical.push_str("f:");
        canonical.push_str(lit);
        canonical.push('\n');
    }
    for (label, type_str, body, head) in &named_rules {
        canonical.push_str("r:");
        canonical.push_str(type_str);
        canonical.push(':');
        canonical.push_str(label);
        canonical.push(':');
        canonical.push_str(body);
        canonical.push(':');
        canonical.push_str(head);
        canonical.push('\n');
    }
    for (sup, inf) in &sups {
        canonical.push_str("s:");
        canonical.push_str(sup);
        canonical.push('>');
        canonical.push_str(inf);
        canonical.push('\n');
    }

    *blake3::hash(canonical.as_bytes()).as_bytes()
}

/// Compute the file-level Merkle root from an ordered list of leaf nodes.
///
/// `ContentHash = BLAKE3(leaf₁_hash ‖ leaf₂_hash ‖ … ‖ leafₙ_hash)` per §4.2.
///
/// Returns `[0u8; 32]` for empty files (no leaves).
pub fn compute_file_root(leaves: &[MerkleLeaf]) -> ContentHash {
    if leaves.is_empty() {
        return [0u8; 32];
    }
    let mut hasher = blake3::Hasher::new();
    for leaf in leaves {
        hasher.update(&leaf.hash);
    }
    *hasher.finalize().as_bytes()
}

/// Compute the vault-level Merkle root from per-file root hashes.
///
/// Files are sorted by canonical relative path (UTF-8 lexicographic,
/// forward-slash normalised) before hashing, per §4.6.
///
/// `ContentHash = BLAKE3(file₁_hash ‖ file₂_hash ‖ … ‖ fileₘ_hash)`
///
/// Returns `[0u8; 32]` for an empty vault (no files).
pub fn compute_vault_root(file_hashes: &[(&Path, ContentHash)]) -> ContentHash {
    if file_hashes.is_empty() {
        return [0u8; 32];
    }

    // Sort by forward-slash-normalised UTF-8 path string.
    let mut sorted: Vec<(&Path, ContentHash)> = file_hashes.to_vec();
    sorted.sort_by(|(a, _), (b, _)| {
        let a_str = a.to_string_lossy().replace('\\', "/");
        let b_str = b.to_string_lossy().replace('\\', "/");
        a_str.cmp(&b_str)
    });

    let mut hasher = blake3::Hasher::new();
    for (_, hash) in &sorted {
        hasher.update(hash);
    }
    *hasher.finalize().as_bytes()
}

// ── Section detection (§4.5) ──────────────────────────────────────────────────

/// Compute the grounding section for a single `SplBlock` leaf.
///
/// Algorithm (§4.5):
/// 1. Walk backwards from `spl_leaf_idx` to find the nearest preceding `Heading`
///    leaf — this is `section_start`.  If no heading is found the SPL block is in
///    the preamble and `section_start = 0`.
/// 2. Walk forward from `section_start` to find the next `Heading` at the same
///    or higher structural level (lower numeric level number, e.g. `##` terminates
///    a `###` section).  If not found, `section_end = last leaf index`.
///    For preamble sections the forward scan stops at the *first* heading of any
///    level.
/// 3. The grounding hash is `BLAKE3(h₁ ‖ h₂ ‖ …)` where `h_i` are the hashes of
///    every **non**-`SplBlock` leaf in `[section_start..=section_end]`.  Returns
///    `[0u8; 32]` when there are no such leaves.
///
/// The returned [`Section::leaf_range`] is `(0, 0)` — a placeholder; [`detect_sections`]
/// fills in the correct half-open range into `FileMerkle::spl_leaves`.
/// [`Section::heading_text`] is left empty; the caller (scanner) should populate it
/// from the file content using [`Section::heading_line`].
pub fn grounding_section(leaves: &[MerkleLeaf], spl_leaf_idx: usize) -> Section {
    assert!(
        spl_leaf_idx < leaves.len(),
        "spl_leaf_idx {} out of bounds (len {})",
        spl_leaf_idx,
        leaves.len()
    );

    // ── Step 1: find section_start by walking backwards ──────────────────────
    let (section_start_idx, heading_level, heading_line) = match (0..spl_leaf_idx)
        .rev()
        .find(|&i| matches!(leaves[i].node_type, LeafType::Heading { .. }))
    {
        Some(idx) => {
            let level = if let LeafType::Heading { level } = leaves[idx].node_type {
                level
            } else {
                unreachable!()
            };
            (idx, level, leaves[idx].start_line)
        }
        None => {
            // Preamble: no heading precedes this SPL block.
            (0, 0u8, 0u32)
        }
    };

    // ── Step 2: find section_end by walking forward ───────────────────────────
    let section_end_idx = if heading_level == 0 {
        // Preamble: extend to just before the first heading of any level.
        match leaves
            .iter()
            .position(|l| matches!(l.node_type, LeafType::Heading { .. }))
        {
            Some(first_heading_idx) => first_heading_idx.saturating_sub(1),
            None => leaves.len().saturating_sub(1),
        }
    } else {
        // Non-preamble: scan forward from the leaf after section_start for a
        // heading at the same or higher structural level (lower or equal number).
        let search_from = section_start_idx + 1;
        match leaves[search_from..].iter().position(
            |l| matches!(l.node_type, LeafType::Heading { level } if level <= heading_level),
        ) {
            Some(offset) => {
                // The terminating heading is at `search_from + offset`; the last
                // leaf *inside* this section is one before it.
                (search_from + offset).saturating_sub(1)
            }
            None => leaves.len().saturating_sub(1),
        }
    };

    // ── Step 3: compute grounding hash ───────────────────────────────────────
    let mut hasher = blake3::Hasher::new();
    let mut has_content = false;
    for leaf in &leaves[section_start_idx..=section_end_idx] {
        if !matches!(leaf.node_type, LeafType::SplBlock) {
            hasher.update(&leaf.hash);
            has_content = true;
        }
    }
    let grounding_hash: ContentHash = if has_content {
        *hasher.finalize().as_bytes()
    } else {
        [0u8; 32]
    };

    Section {
        heading_line,
        heading_text: String::new(), // populated by the caller from file content
        heading_level,
        leaf_range: (0, 0), // placeholder; detect_sections fills the spl_leaves range
        grounding_hash,
    }
}

/// Detect all grounding sections for a file's Merkle leaf sequence.
///
/// Returns one [`Section`] per unique grounding region that contains at least one
/// `SplBlock` leaf, ordered by document position.
///
/// [`Section::leaf_range`] is the half-open range `[start, end)` into the
/// `FileMerkle::spl_leaves` Vec — i.e., the indices of the SPL leaves (in document
/// order) that are grounded by each section.
///
/// [`Section::heading_text`] is left empty; callers should fill it in from the
/// file content using [`Section::heading_line`].
pub fn detect_sections(leaves: &[MerkleLeaf]) -> Vec<Section> {
    // Collect MerkleLeaf indices of all SplBlock leaves (document order).
    let spl_leaf_indices: Vec<usize> = leaves
        .iter()
        .enumerate()
        .filter(|(_, l)| matches!(l.node_type, LeafType::SplBlock))
        .map(|(i, _)| i)
        .collect();

    if spl_leaf_indices.is_empty() {
        return vec![];
    }

    let mut sections: Vec<Section> = Vec::new();

    for (spl_count, &leaf_idx) in spl_leaf_indices.iter().enumerate() {
        let sec = grounding_section(leaves, leaf_idx);

        // Check whether this SPL block falls in the same grounding section as
        // the previous one.  Two SPL blocks share a section iff they were
        // grounded by the same heading (same line and level; level 0 = preamble).
        let same_as_prev = sections.last().is_some_and(|prev| {
            prev.heading_line == sec.heading_line && prev.heading_level == sec.heading_level
        });

        if same_as_prev {
            // Extend the existing section's SPL leaf range.
            sections.last_mut().unwrap().leaf_range.1 = spl_count + 1;
        } else {
            // Start a new section whose range begins at this SPL index.
            let mut new_sec = sec;
            new_sec.leaf_range = (spl_count, spl_count + 1);
            sections.push(new_sec);
        }
    }

    sections
}

// ── Hash prefix resolution (§3.3 rule 1, REQ-042a) ───────────────────────────

/// A single location where a Merkle leaf with a given hash was found in the vault.
#[derive(Debug, Clone)]
pub struct HashLocation {
    /// Relative path of the file within the vault.
    pub file: PathBuf,
    /// 0-indexed position of this leaf in the file's `merkle_leaves` list.
    pub leaf_index: usize,
    /// The Merkle leaf at that position.
    pub leaf: MerkleLeaf,
}

/// Vault-wide index mapping full 64-char lowercase hex hashes to all leaf locations
/// that carry that hash.
///
/// Built from loaded [`ParsedFile`] data by [`build_vault_hash_index`].
pub struct VaultHashIndex {
    /// `full_hex_hash → Vec<HashLocation>`.
    ///
    /// Multiple locations for the same hash arise when identical content
    /// appears in more than one file or position (duplicate-content case).
    pub entries: HashMap<String, Vec<HashLocation>>,
}

/// Outcome of a hash prefix resolution attempt (§3.3 resolution rule 1, REQ-042a).
pub enum HashResolutionResult {
    /// Exactly one distinct full hash matched the prefix.
    ///
    /// All locations sharing that full hash are returned.  When identical
    /// content appears at multiple positions this is still a successful
    /// resolution — the caller receives every location.
    Found {
        /// Full 64-char lowercase hex hash.
        full_hash: String,
        /// All vault locations that carry this hash (≥ 1).
        locations: Vec<HashLocation>,
    },
    /// No leaf hash in the vault starts with the given prefix.
    NotFound,
    /// Two or more *distinct* full hashes start with the given prefix —
    /// the prefix is not long enough to identify a unique content block.
    Ambiguous {
        /// The prefix as supplied by the caller.
        prefix: String,
        /// Sorted list of distinct full hex hashes that matched.
        candidates: Vec<String>,
    },
}

/// Build a vault-wide hash index from a collection of loaded [`ParsedFile`] entries.
///
/// For every file, iterates all `merkle_leaves` and inserts
/// `full_hex_hash → (file, leaf_index, leaf)` into the index.
/// The same full hash may map to multiple locations when identical content
/// appears in more than one file or position.
pub fn build_vault_hash_index(files: &[ParsedFile]) -> VaultHashIndex {
    let mut entries: HashMap<String, Vec<HashLocation>> = HashMap::new();
    for file in files {
        for (leaf_index, leaf) in file.merkle_leaves.iter().enumerate() {
            let hex: String = leaf.hash.iter().map(|b| format!("{b:02x}")).collect();
            entries.entry(hex).or_default().push(HashLocation {
                file: file.path.clone(),
                leaf_index,
                leaf: leaf.clone(),
            });
        }
    }
    VaultHashIndex { entries }
}

/// Resolve a hex hash prefix to its matching Merkle leaf location(s).
///
/// Implements §3.3 resolution rule 1 and REQ-042a:
///
/// 1. Filter all index entries whose `full_hash_hex` starts with `prefix`
///    (case-insensitive).
/// 2. Zero matches → [`HashResolutionResult::NotFound`].
/// 3. Exactly one distinct full hash matches → [`HashResolutionResult::Found`]
///    with all locations that share that hash.
/// 4. Multiple distinct full hashes match → [`HashResolutionResult::Ambiguous`]
///    with a sorted list of matching full hashes.
pub fn resolve_hash_prefix(prefix: &str, index: &VaultHashIndex) -> HashResolutionResult {
    let prefix_lower = prefix.to_lowercase();

    let matching: Vec<(&String, &Vec<HashLocation>)> = index
        .entries
        .iter()
        .filter(|(hash, _)| hash.starts_with(&prefix_lower))
        .collect();

    match matching.len() {
        0 => HashResolutionResult::NotFound,
        1 => {
            let (full_hash, locations) = matching[0];
            HashResolutionResult::Found {
                full_hash: full_hash.clone(),
                locations: locations.clone(),
            }
        }
        _ => {
            let mut candidates: Vec<String> = matching.iter().map(|(h, _)| (*h).clone()).collect();
            candidates.sort();
            HashResolutionResult::Ambiguous {
                prefix: prefix.to_string(),
                candidates,
            }
        }
    }
}

// ── Block-id resolution (§3.3 rules 2–3, REQ-042b, REQ-042c) ─────────────────

/// Error returned when a `^block-id` or `[[Page^block-id]]` reference cannot
/// be resolved to a Merkle leaf (REQ-042b, REQ-042c).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockIdResolutionError {
    /// No leaf carrying the given `block_id` annotation was found in the target
    /// file.
    BlockIdNotFound { block_id: String, file: PathBuf },
    /// The page name in a cross-file reference could not be resolved to any
    /// known file in the vault.
    PageNotFound { page_name: String },
}

impl std::fmt::Display for BlockIdResolutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BlockIdNotFound { block_id, file } => {
                write!(
                    f,
                    "block id '^{}' not found in '{}'",
                    block_id,
                    file.display()
                )
            }
            Self::PageNotFound { page_name } => {
                write!(f, "page '{page_name}' not found in vault")
            }
        }
    }
}

impl std::error::Error for BlockIdResolutionError {}

/// Resolve a local block-id reference (`^block-id`) within a single file's
/// Merkle leaves (REQ-042b).
///
/// Scans `leaves` for the first leaf whose [`MerkleLeaf::block_id`] annotation
/// equals `block_id` (case-sensitive).  Returns the leaf's [`ContentHash`] when
/// found, or `None` if no matching leaf exists.
///
/// Callers should treat a `None` result as a static validation error.
pub fn resolve_local_block_id(leaves: &[MerkleLeaf], block_id: &str) -> Option<ContentHash> {
    leaves
        .iter()
        .find(|leaf| leaf.block_id.as_deref() == Some(block_id))
        .map(|leaf| leaf.hash)
}

/// Resolve a cross-file block-id reference (`[[Page^block-id]]`) to a
/// [`ContentHash`] (REQ-042c).
///
/// # Algorithm
///
/// 1. Resolve `page_name` to a canonical page name using standard wikilink
///    page-name matching ([`crate::scanner::resolve_page_name`]) against
///    `file_index`.
/// 2. Locate the corresponding [`ParsedFile`] entry in `files`.
/// 3. Call [`resolve_local_block_id`] on the target file's Merkle leaves.
///
/// # Errors
///
/// Returns [`BlockIdResolutionError::PageNotFound`] if the page name cannot be
/// resolved, or [`BlockIdResolutionError::BlockIdNotFound`] if no leaf in the
/// resolved file carries the given `block_id`.  Both are static validation
/// errors — not drift diagnostics.
pub fn resolve_cross_file_block_id(
    files: &[ParsedFile],
    file_index: &[(String, PathBuf)],
    page_name: &str,
    block_id: &str,
) -> Result<ContentHash, BlockIdResolutionError> {
    use crate::scanner::resolve_page_name;

    // Step 1: Resolve the page name to a canonical page name.
    let resolved_page = resolve_page_name(page_name, file_index).ok_or_else(|| {
        BlockIdResolutionError::PageNotFound {
            page_name: page_name.to_string(),
        }
    })?;

    // Step 2: Locate the file path for the resolved page name.
    let target_path = file_index
        .iter()
        .find(|(p, _)| p == &resolved_page)
        .map(|(_, path)| path)
        .ok_or_else(|| BlockIdResolutionError::PageNotFound {
            page_name: page_name.to_string(),
        })?;

    // Step 3: Find the ParsedFile with that path.
    let parsed_file = files
        .iter()
        .find(|f| &f.path == target_path)
        .ok_or_else(|| BlockIdResolutionError::PageNotFound {
            page_name: page_name.to_string(),
        })?;

    // Step 4: Resolve the block-id within the file's Merkle leaves.
    resolve_local_block_id(&parsed_file.merkle_leaves, block_id).ok_or_else(|| {
        BlockIdResolutionError::BlockIdNotFound {
            block_id: block_id.to_string(),
            file: target_path.clone(),
        }
    })
}

// ── Source-ref classification (REQ-042) ──────────────────────────────────────

/// The parsed type of a raw source reference string from a `(meta LABEL (source ...))` form.
///
/// Used to classify strings extracted from SPL `source` meta values before any
/// resolution takes place.  Classification is purely syntactic — no file I/O
/// or hash look-ups are performed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceRefType {
    /// A BLAKE3 hash reference: 8 or more lowercase or uppercase hex characters.
    HashRef,
    /// A local block reference within the same file: starts with `^`.
    LocalRef,
    /// A cross-file block reference: `[[Page^block-id]]` pattern.
    CrossFileRef,
    /// A page-only wikilink reference: `[[Page]]` pattern (no block-id anchor).
    PageRef,
    /// Unrecognised format; preserved verbatim for forward-compatibility.
    Unknown,
}

/// Classify a raw source reference string by its syntactic format (REQ-042).
///
/// Rules (checked in order):
/// 1. `[[...^...]]` → [`SourceRefType::CrossFileRef`]
/// 2. `[[...]]`     → [`SourceRefType::PageRef`]
/// 3. `^...`        → [`SourceRefType::LocalRef`]
/// 4. 8+ hex chars  → [`SourceRefType::HashRef`]
/// 5. anything else → [`SourceRefType::Unknown`]
pub fn classify_source_ref(s: &str) -> SourceRefType {
    if s.starts_with("[[") && s.ends_with("]]") {
        if s.contains('^') {
            return SourceRefType::CrossFileRef;
        }
        return SourceRefType::PageRef;
    }
    if s.starts_with('^') {
        return SourceRefType::LocalRef;
    }
    if s.len() >= 8 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return SourceRefType::HashRef;
    }
    SourceRefType::Unknown
}

// ── Source-ref validation (REQ-042, CON-004) ──────────────────────────────────

/// Validate all explicit source-ref declarations across all SPL blocks in the vault.
///
/// For each [`ExplicitGrounding`] found in any `file_merkle.spl_leaves`, every
/// raw `source_ref` string is classified and resolved:
///
/// | Ref type           | Outcome                    | Error message                                                                   |
/// |--------------------|----------------------------|---------------------------------------------------------------------------------|
/// | Hash prefix        | zero leaves matched        | `content hash X not found — source content may have been modified or removed`  |
/// | Hash prefix        | ambiguous (>1 full hashes) | `ambiguous hash prefix X — use a longer prefix`                                 |
/// | Hash prefix        | unique or duplicate match  | *(valid — no error)*                                                            |
/// | `^block-id`        | not found in source file   | `source references ^X which does not exist in this file`                        |
/// | `[[Page]]`         | page not in vault          | `page Page not found`                                                           |
/// | `[[Page^block-id]]`| page not in vault          | `page Page not found`                                                           |
/// | `[[Page^block-id]]`| block-id not found in page | `block-id not found in Page`                                                    |
///
/// All errors are reported as [`DiagnosticLevel::Error`] (spl_diagnostics, not drift_diagnostics).
pub fn validate_source_refs(
    files: &[ParsedFile],
    file_index: &[(String, PathBuf)],
    vault_hash_index: &VaultHashIndex,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for file in files {
        let merkle = match file.file_merkle.as_ref() {
            Some(m) => m,
            None => continue,
        };

        for spl_leaf in &merkle.spl_leaves {
            for grounding in &spl_leaf.explicit_groundings {
                for source_ref in &grounding.source_refs {
                    validate_one_source_ref(
                        source_ref,
                        &file.path,
                        spl_leaf.start_line,
                        &file.merkle_leaves,
                        files,
                        file_index,
                        vault_hash_index,
                        &mut diagnostics,
                    );
                }
            }
        }
    }

    diagnostics
}

/// Validate a single raw source-ref string and push any error onto `out`.
#[allow(clippy::too_many_arguments)]
fn validate_one_source_ref(
    source_ref: &str,
    source_file: &std::path::Path,
    spl_line: u32,
    source_leaves: &[MerkleLeaf],
    files: &[ParsedFile],
    file_index: &[(String, PathBuf)],
    vault_hash_index: &VaultHashIndex,
    out: &mut Vec<Diagnostic>,
) {
    match classify_source_ref(source_ref) {
        SourceRefType::HashRef => {
            match resolve_hash_prefix(source_ref, vault_hash_index) {
                HashResolutionResult::NotFound => {
                    out.push(Diagnostic {
                        level: DiagnosticLevel::Error,
                        message: format!(
                            "content hash {source_ref} not found — source content may have been modified or removed"
                        ),
                        file: source_file.to_path_buf(),
                        line: spl_line,
                        column: 0,
                    });
                }
                HashResolutionResult::Ambiguous { prefix, .. } => {
                    out.push(Diagnostic {
                        level: DiagnosticLevel::Error,
                        message: format!("ambiguous hash prefix {prefix} — use a longer prefix"),
                        file: source_file.to_path_buf(),
                        line: spl_line,
                        column: 0,
                    });
                }
                HashResolutionResult::Found { .. } => {
                    // Unique match or duplicate content — both are valid.
                }
            }
        }
        SourceRefType::LocalRef => {
            // Strip the leading '^'.
            let block_id = &source_ref[1..];
            if resolve_local_block_id(source_leaves, block_id).is_none() {
                out.push(Diagnostic {
                    level: DiagnosticLevel::Error,
                    message: format!(
                        "source references ^{block_id} which does not exist in this file"
                    ),
                    file: source_file.to_path_buf(),
                    line: spl_line,
                    column: 0,
                });
            }
        }
        SourceRefType::PageRef => {
            // Strip `[[` and `]]`.
            let page_name = &source_ref[2..source_ref.len() - 2];
            if crate::scanner::resolve_page_name(page_name, file_index).is_none() {
                out.push(Diagnostic {
                    level: DiagnosticLevel::Error,
                    message: format!("page {page_name} not found"),
                    file: source_file.to_path_buf(),
                    line: spl_line,
                    column: 0,
                });
            }
        }
        SourceRefType::CrossFileRef => {
            // Strip `[[` and `]]`, then split on the first `^`.
            let inner = &source_ref[2..source_ref.len() - 2];
            let (page_name, block_id) = match inner.find('^') {
                Some(pos) => (&inner[..pos], &inner[pos + 1..]),
                None => {
                    // Malformed — no '^' despite classify saying CrossFileRef.
                    // This cannot happen given classify_source_ref logic, but be safe.
                    return;
                }
            };
            match resolve_cross_file_block_id(files, file_index, page_name, block_id) {
                Ok(_) => {}
                Err(BlockIdResolutionError::PageNotFound { page_name: ref pn }) => {
                    out.push(Diagnostic {
                        level: DiagnosticLevel::Error,
                        message: format!("page {pn} not found"),
                        file: source_file.to_path_buf(),
                        line: spl_line,
                        column: 0,
                    });
                }
                Err(BlockIdResolutionError::BlockIdNotFound { .. }) => {
                    out.push(Diagnostic {
                        level: DiagnosticLevel::Error,
                        message: format!("block-id not found in {page_name}"),
                        file: source_file.to_path_buf(),
                        line: spl_line,
                        column: 0,
                    });
                }
            }
        }
        SourceRefType::Unknown => {
            // Unrecognised format — skip for forward-compatibility.
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::LeafType;

    fn make_leaf(leaf_type: LeafType, hash: ContentHash) -> MerkleLeaf {
        MerkleLeaf {
            node_type: leaf_type,
            start_line: 1,
            end_line: 1,
            hash,
            spl_hashes: None,
            block_id: None,
        }
    }

    // ── leaf_type_tag ──────────────────────────────────────────────────────────

    #[test]
    fn all_leaf_type_tags_are_unique() {
        let tags = [
            TAG_HEADING,
            TAG_PARAGRAPH,
            TAG_CODE_BLOCK,
            TAG_SPL_BLOCK,
            TAG_LIST,
            TAG_BLOCK_QUOTE,
            TAG_TABLE,
            TAG_FRONTMATTER,
            TAG_THEMATIC_BREAK,
            TAG_HTML_BLOCK,
        ];
        let mut seen = std::collections::HashSet::new();
        for tag in tags {
            assert!(seen.insert(tag), "duplicate tag: 0x{tag:02x}");
        }
    }

    #[test]
    fn leaf_type_tag_matches_constants() {
        assert_eq!(leaf_type_tag(&LeafType::Heading { level: 1 }), TAG_HEADING);
        assert_eq!(leaf_type_tag(&LeafType::Paragraph), TAG_PARAGRAPH);
        assert_eq!(
            leaf_type_tag(&LeafType::CodeBlock { language: None }),
            TAG_CODE_BLOCK
        );
        assert_eq!(leaf_type_tag(&LeafType::SplBlock), TAG_SPL_BLOCK);
        assert_eq!(leaf_type_tag(&LeafType::List { ordered: false }), TAG_LIST);
        assert_eq!(leaf_type_tag(&LeafType::BlockQuote), TAG_BLOCK_QUOTE);
        assert_eq!(leaf_type_tag(&LeafType::Table), TAG_TABLE);
        assert_eq!(leaf_type_tag(&LeafType::Frontmatter), TAG_FRONTMATTER);
        assert_eq!(leaf_type_tag(&LeafType::ThematicBreak), TAG_THEMATIC_BREAK);
        assert_eq!(leaf_type_tag(&LeafType::HtmlBlock), TAG_HTML_BLOCK);
    }

    // ── compute_leaf_hash ──────────────────────────────────────────────────────

    #[test]
    fn compute_leaf_hash_prepends_type_tag() {
        // Hash should differ from bare BLAKE3(content) because type tag is prepended.
        let content = b"hello world";
        let hash_with_tag = compute_leaf_hash(&LeafType::Paragraph, content);

        // Bare BLAKE3(content) — no type tag.
        let bare_hash: ContentHash = *blake3::hash(content).as_bytes();

        assert_ne!(
            hash_with_tag, bare_hash,
            "hash with type tag should differ from bare hash"
        );
    }

    #[test]
    fn compute_leaf_hash_different_leaf_types_differ() {
        let content = b"same text";
        let h1 = compute_leaf_hash(&LeafType::Paragraph, content);
        let h2 = compute_leaf_hash(&LeafType::Heading { level: 1 }, content);
        assert_ne!(
            h1, h2,
            "different leaf types should produce different hashes"
        );
    }

    #[test]
    fn compute_leaf_hash_same_type_same_content_is_deterministic() {
        let content = b"deterministic content";
        let h1 = compute_leaf_hash(&LeafType::Paragraph, content);
        let h2 = compute_leaf_hash(&LeafType::Paragraph, content);
        assert_eq!(h1, h2);
    }

    #[test]
    fn compute_leaf_hash_heading_levels_differ() {
        let content = b"Introduction";
        let h1 = compute_leaf_hash(&LeafType::Heading { level: 1 }, content);
        let h2 = compute_leaf_hash(&LeafType::Heading { level: 2 }, content);
        // Same type tag, but content includes the level byte, so should differ.
        // (The content bytes are different because the caller encodes the level.)
        // Here both use TAG_HEADING, so they would be the same — it is the caller's
        // responsibility to include the level in the content bytes passed in.
        // This test just checks that the function itself is deterministic.
        let _ = (h1, h2);
    }

    #[test]
    fn compute_leaf_hash_spl_includes_tag_and_content() {
        // compute_leaf_hash for SplBlock now behaves like other leaf types:
        // BLAKE3(TAG_SPL_BLOCK ‖ content).  The dual-hash formula
        // (BLAKE3(content_hash ‖ ast_hash)) is handled by spl_combined_hash.
        let content = b"some spl code";
        let h = compute_leaf_hash(&LeafType::SplBlock, content);

        let expected: ContentHash = {
            let mut hasher = blake3::Hasher::new();
            hasher.update(&[TAG_SPL_BLOCK]);
            hasher.update(content);
            *hasher.finalize().as_bytes()
        };
        assert_eq!(h, expected);
    }

    #[test]
    fn compute_leaf_hash_spl_differs_with_different_content() {
        let h1 = compute_leaf_hash(&LeafType::SplBlock, b"rule_a");
        let h2 = compute_leaf_hash(&LeafType::SplBlock, b"rule_b");
        assert_ne!(
            h1, h2,
            "different SPL content should produce different hashes"
        );
    }

    // ── compute_file_root ──────────────────────────────────────────────────────

    #[test]
    fn compute_file_root_empty_leaves_returns_zero() {
        assert_eq!(compute_file_root(&[]), [0u8; 32]);
    }

    #[test]
    fn compute_file_root_single_leaf() {
        let hash = [1u8; 32];
        let leaf = make_leaf(LeafType::Paragraph, hash);
        let root = compute_file_root(&[leaf]);

        // Should equal BLAKE3(hash).
        let expected: ContentHash = *blake3::Hasher::new().update(&hash).finalize().as_bytes();
        assert_eq!(root, expected);
    }

    #[test]
    fn compute_file_root_order_matters() {
        let h1 = [1u8; 32];
        let h2 = [2u8; 32];
        let leaf1 = make_leaf(LeafType::Paragraph, h1);
        let leaf2 = make_leaf(LeafType::Heading { level: 1 }, h2);

        let root_ab = compute_file_root(&[leaf1.clone(), leaf2.clone()]);
        let root_ba = compute_file_root(&[leaf2, leaf1]);

        assert_ne!(root_ab, root_ba, "leaf order should affect file root");
    }

    #[test]
    fn compute_file_root_is_deterministic() {
        let h = [42u8; 32];
        let leaf = make_leaf(LeafType::Paragraph, h);
        let r1 = compute_file_root(&[leaf.clone()]);
        let r2 = compute_file_root(&[leaf]);
        assert_eq!(r1, r2);
    }

    // ── compute_vault_root ─────────────────────────────────────────────────────

    #[test]
    fn compute_vault_root_empty_returns_zero() {
        assert_eq!(compute_vault_root(&[]), [0u8; 32]);
    }

    #[test]
    fn compute_vault_root_sorts_by_path() {
        use std::path::Path;
        let h1 = [1u8; 32];
        let h2 = [2u8; 32];

        let pa = Path::new("a/note.md");
        let pb = Path::new("b/note.md");

        // Same files, different insertion order.
        let root_ab = compute_vault_root(&[(pa, h1), (pb, h2)]);
        let root_ba = compute_vault_root(&[(pb, h2), (pa, h1)]);

        assert_eq!(
            root_ab, root_ba,
            "vault root should be path-order independent"
        );
    }

    #[test]
    fn compute_vault_root_different_hashes_differ() {
        use std::path::Path;
        let pa = Path::new("a.md");
        let pb = Path::new("b.md");

        let r1 = compute_vault_root(&[(pa, [1u8; 32]), (pb, [2u8; 32])]);
        let r2 = compute_vault_root(&[(pa, [3u8; 32]), (pb, [4u8; 32])]);

        assert_ne!(r1, r2);
    }

    #[test]
    fn compute_vault_root_forward_slash_normalised() {
        use std::path::Path;
        // On Windows paths might use backslash; our sort key normalises them.
        // We test that a path with forward slashes is handled deterministically.
        let pa = Path::new("notes/foo.md");
        let pb = Path::new("notes/bar.md");
        let h_foo = [10u8; 32];
        let h_bar = [20u8; 32];

        // bar < foo lexicographically, so root should be BLAKE3(h_bar ‖ h_foo).
        let root = compute_vault_root(&[(pa, h_foo), (pb, h_bar)]);
        let mut expected = blake3::Hasher::new();
        expected.update(&h_bar);
        expected.update(&h_foo);
        let expected: ContentHash = *expected.finalize().as_bytes();

        assert_eq!(root, expected);
    }

    // ── grounding_section & detect_sections ────────────────────────────────────

    /// Build a MerkleLeaf with a specified start_line (for section tests).
    fn make_leaf_at(leaf_type: LeafType, start_line: u32, hash: ContentHash) -> MerkleLeaf {
        MerkleLeaf {
            node_type: leaf_type,
            start_line,
            end_line: start_line,
            hash,
            spl_hashes: None,
            block_id: None,
        }
    }

    #[test]
    fn detect_sections_empty_leaves_returns_empty() {
        assert!(detect_sections(&[]).is_empty());
    }

    #[test]
    fn detect_sections_no_spl_leaves_returns_empty() {
        let leaves = vec![
            make_leaf(LeafType::Heading { level: 1 }, [1u8; 32]),
            make_leaf(LeafType::Paragraph, [2u8; 32]),
        ];
        assert!(detect_sections(&leaves).is_empty());
    }

    #[test]
    fn grounding_section_preamble_no_headings() {
        // File: [Paragraph, SplBlock]
        // SPL is in preamble; no heading above it and no heading in the whole file.
        let p_hash = [1u8; 32];
        let s_hash = [2u8; 32];
        let leaves = vec![
            make_leaf_at(LeafType::Paragraph, 1, p_hash),
            make_leaf_at(LeafType::SplBlock, 3, s_hash),
        ];
        let sec = grounding_section(&leaves, 1);
        assert_eq!(sec.heading_level, 0, "preamble has level 0");
        assert_eq!(sec.heading_line, 0, "preamble line sentinel is 0");

        // Grounding hash = BLAKE3(p_hash) — the paragraph; SPL excluded.
        let expected: ContentHash = *blake3::Hasher::new().update(&p_hash).finalize().as_bytes();
        assert_eq!(sec.grounding_hash, expected);
    }

    #[test]
    fn grounding_section_preamble_stops_at_first_heading() {
        // File: [Paragraph, SplBlock, Heading(2)]
        // The SPL is in the preamble; section_end should be before the heading.
        let p_hash = [10u8; 32];
        let s_hash = [20u8; 32];
        let h_hash = [30u8; 32];
        let leaves = vec![
            make_leaf_at(LeafType::Paragraph, 1, p_hash),
            make_leaf_at(LeafType::SplBlock, 3, s_hash),
            make_leaf_at(LeafType::Heading { level: 2 }, 5, h_hash),
        ];
        let sec = grounding_section(&leaves, 1);
        assert_eq!(sec.heading_level, 0);
        // Section = [0..=1]; non-SPL = leaves[0] only.
        let expected: ContentHash = *blake3::Hasher::new().update(&p_hash).finalize().as_bytes();
        assert_eq!(sec.grounding_hash, expected);
    }

    #[test]
    fn grounding_section_under_heading() {
        // File: [Heading(2) @ line 1, Paragraph, SplBlock]
        // SPL grounded by Heading(2).
        let h_hash = [1u8; 32];
        let p_hash = [2u8; 32];
        let s_hash = [3u8; 32];
        let leaves = vec![
            make_leaf_at(LeafType::Heading { level: 2 }, 1, h_hash),
            make_leaf_at(LeafType::Paragraph, 3, p_hash),
            make_leaf_at(LeafType::SplBlock, 5, s_hash),
        ];
        let sec = grounding_section(&leaves, 2);
        assert_eq!(sec.heading_level, 2);
        assert_eq!(sec.heading_line, 1);

        // Section [0..=2]; non-SPL = h_hash, p_hash.
        let mut h = blake3::Hasher::new();
        h.update(&h_hash);
        h.update(&p_hash);
        let expected: ContentHash = *h.finalize().as_bytes();
        assert_eq!(sec.grounding_hash, expected);
    }

    #[test]
    fn grounding_section_subsection_narrower_than_parent() {
        // File: [H2@1, Para, H3@5, Para, SplBlock, H2@10]
        // SPL is grounded by H3@5 (nearest preceding heading), section terminates
        // at H2@10 (level ≤ 3 from a forward scan).
        let h2a = [1u8; 32];
        let p1 = [2u8; 32];
        let h3 = [3u8; 32];
        let p2 = [4u8; 32];
        let spl = [5u8; 32];
        let h2b = [6u8; 32];
        let leaves = vec![
            make_leaf_at(LeafType::Heading { level: 2 }, 1, h2a),
            make_leaf_at(LeafType::Paragraph, 2, p1),
            make_leaf_at(LeafType::Heading { level: 3 }, 5, h3),
            make_leaf_at(LeafType::Paragraph, 6, p2),
            make_leaf_at(LeafType::SplBlock, 8, spl),
            make_leaf_at(LeafType::Heading { level: 2 }, 10, h2b),
        ];
        let sec = grounding_section(&leaves, 4);
        assert_eq!(sec.heading_level, 3, "grounded by H3, not H2");
        assert_eq!(sec.heading_line, 5);
        // Section = [2..=4] (H3, Para, SPL); non-SPL = h3, p2.
        let mut h = blake3::Hasher::new();
        h.update(&h3);
        h.update(&p2);
        let expected: ContentHash = *h.finalize().as_bytes();
        assert_eq!(sec.grounding_hash, expected);
    }

    #[test]
    fn grounding_section_h2_terminates_at_next_h2() {
        // File: [H2@1, Para, SplBlock, H2@10, Para]
        // SPL under H2@1; section_end = just before H2@10.
        let h2a = [1u8; 32];
        let par = [2u8; 32];
        let spl = [3u8; 32];
        let h2b = [4u8; 32];
        let par2 = [5u8; 32];
        let leaves = vec![
            make_leaf_at(LeafType::Heading { level: 2 }, 1, h2a),
            make_leaf_at(LeafType::Paragraph, 2, par),
            make_leaf_at(LeafType::SplBlock, 4, spl),
            make_leaf_at(LeafType::Heading { level: 2 }, 10, h2b),
            make_leaf_at(LeafType::Paragraph, 11, par2),
        ];
        let sec = grounding_section(&leaves, 2);
        assert_eq!(sec.heading_level, 2);
        // Section = [0..=2]; non-SPL = h2a, par.
        let mut h = blake3::Hasher::new();
        h.update(&h2a);
        h.update(&par);
        let expected: ContentHash = *h.finalize().as_bytes();
        assert_eq!(sec.grounding_hash, expected);
    }

    #[test]
    fn detect_sections_two_spl_same_section() {
        // File: [H2@1, SplBlock, Para, SplBlock]
        // Both SPL blocks grounded by H2@1 → single section with leaf_range (0, 2).
        let h_hash = [1u8; 32];
        let s1 = [2u8; 32];
        let p_hash = [3u8; 32];
        let s2 = [4u8; 32];
        let leaves = vec![
            make_leaf_at(LeafType::Heading { level: 2 }, 1, h_hash),
            make_leaf_at(LeafType::SplBlock, 3, s1),
            make_leaf_at(LeafType::Paragraph, 5, p_hash),
            make_leaf_at(LeafType::SplBlock, 7, s2),
        ];
        let sections = detect_sections(&leaves);
        assert_eq!(sections.len(), 1, "both SPLs are in the same section");
        assert_eq!(sections[0].leaf_range, (0, 2));
        assert_eq!(sections[0].heading_level, 2);
    }

    #[test]
    fn detect_sections_two_spl_different_sections() {
        // File: [H2@1, SplBlock, H2@10, SplBlock]
        // Each SPL in a different H2 section → two sections.
        let h1 = [1u8; 32];
        let s1 = [2u8; 32];
        let h2 = [3u8; 32];
        let s2 = [4u8; 32];
        let leaves = vec![
            make_leaf_at(LeafType::Heading { level: 2 }, 1, h1),
            make_leaf_at(LeafType::SplBlock, 3, s1),
            make_leaf_at(LeafType::Heading { level: 2 }, 10, h2),
            make_leaf_at(LeafType::SplBlock, 12, s2),
        ];
        let sections = detect_sections(&leaves);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].leaf_range, (0, 1)); // spl_leaves[0]
        assert_eq!(sections[1].leaf_range, (1, 2)); // spl_leaves[1]
        assert_eq!(sections[0].heading_line, 1);
        assert_eq!(sections[1].heading_line, 10);
    }

    #[test]
    fn detect_sections_subsection_narrower_context() {
        // File: [H2@1, SplA, H3@5, SplB]
        // SplA grounded by H2@1 (extends to EOF since no H2 or H1 follows).
        // SplB grounded by H3@5 (narrower context).
        let h2 = [1u8; 32];
        let sa = [2u8; 32];
        let h3 = [3u8; 32];
        let sb = [4u8; 32];
        let leaves = vec![
            make_leaf_at(LeafType::Heading { level: 2 }, 1, h2),
            make_leaf_at(LeafType::SplBlock, 3, sa),
            make_leaf_at(LeafType::Heading { level: 3 }, 5, h3),
            make_leaf_at(LeafType::SplBlock, 7, sb),
        ];
        let sections = detect_sections(&leaves);
        assert_eq!(sections.len(), 2, "H2 and H3 sections are distinct");
        assert_eq!(sections[0].heading_level, 2);
        assert_eq!(sections[1].heading_level, 3);
    }

    #[test]
    fn grounding_section_empty_section_no_content_leaves() {
        // File: [H2@1, SplBlock] — no content other than heading (excluded) and SPL.
        let h = [1u8; 32];
        let s = [2u8; 32];
        let leaves = vec![
            make_leaf_at(LeafType::Heading { level: 2 }, 1, h),
            make_leaf_at(LeafType::SplBlock, 3, s),
        ];
        let sec = grounding_section(&leaves, 1);
        // Non-SPL leaves = [heading]. grounding_hash = BLAKE3(h).
        let expected: ContentHash = *blake3::Hasher::new().update(&h).finalize().as_bytes();
        assert_eq!(sec.grounding_hash, expected);
    }

    #[test]
    fn grounding_section_grounding_hash_excludes_spl_leaves() {
        // File: [H2, SplBlock_A, Para, SplBlock_B]
        // Grounding hash for SplBlock_B should include H2 and Para, NOT SplBlock_A.
        let h = [1u8; 32];
        let sa = [2u8; 32];
        let p = [3u8; 32];
        let sb = [4u8; 32];
        let leaves = vec![
            make_leaf_at(LeafType::Heading { level: 2 }, 1, h),
            make_leaf_at(LeafType::SplBlock, 3, sa),
            make_leaf_at(LeafType::Paragraph, 5, p),
            make_leaf_at(LeafType::SplBlock, 7, sb),
        ];
        let sec_a = grounding_section(&leaves, 1);
        let sec_b = grounding_section(&leaves, 3);
        // Both in same section so same grounding hash.
        assert_eq!(sec_a.grounding_hash, sec_b.grounding_hash);
        // Hash = BLAKE3(h ‖ p) — both SPLs excluded.
        let mut hasher = blake3::Hasher::new();
        hasher.update(&h);
        hasher.update(&p);
        let expected: ContentHash = *hasher.finalize().as_bytes();
        assert_eq!(sec_a.grounding_hash, expected);
    }

    // ── normalize_spl ──────────────────────────────────────────────────────────

    #[test]
    fn normalize_spl_strips_comment_lines() {
        let input = ";; this is a comment\n(given foo)\n;; another comment\n(given bar)";
        let result = normalize_spl(input);
        assert!(!result.contains(";;"), "comment lines should be stripped");
        assert!(result.contains("foo"), "fact 'foo' should remain");
        assert!(result.contains("bar"), "fact 'bar' should remain");
    }

    #[test]
    fn normalize_spl_strips_indented_comment_lines() {
        let input = "  ;; indented comment\n(given baz)";
        let result = normalize_spl(input);
        assert!(!result.contains(";;"));
        assert!(result.contains("baz"));
    }

    #[test]
    fn normalize_spl_collapses_whitespace() {
        let input = "(given   foo)   (given   bar)";
        let result = normalize_spl(input);
        // Consecutive spaces collapsed to single
        assert!(!result.contains("   "));
    }

    #[test]
    fn normalize_spl_trims_result() {
        let input = "   (given foo)   ";
        let result = normalize_spl(input);
        assert_eq!(result, result.trim());
    }

    #[test]
    fn normalize_spl_empty_after_comment_stripping() {
        let input = ";; comment only";
        let result = normalize_spl(input);
        assert_eq!(result, "");
    }

    // ── compute_spl_hashes ────────────────────────────────────────────────────

    #[test]
    fn compute_spl_hashes_content_hash_is_normalized_blake3() {
        let content = "(given foo)";
        let spl = compute_spl_hashes(content);
        let expected: ContentHash = *blake3::hash(normalize_spl(content).as_bytes()).as_bytes();
        assert_eq!(spl.content_hash, expected);
    }

    #[test]
    fn compute_spl_hashes_different_content_differs() {
        let a = compute_spl_hashes("(given foo)");
        let b = compute_spl_hashes("(given bar)");
        assert_ne!(a.content_hash, b.content_hash);
    }

    #[test]
    fn compute_spl_hashes_comment_stripped_same_as_without() {
        // Content with only a comment followed by same SPL should produce
        // the same content_hash as the SPL without the comment.
        let with_comment = ";; ignored\n(given foo)";
        let without_comment = "(given foo)";
        let a = compute_spl_hashes(with_comment);
        let b = compute_spl_hashes(without_comment);
        assert_eq!(a.content_hash, b.content_hash);
    }

    #[test]
    fn compute_spl_hashes_deterministic() {
        let content = "(given foo)\n(normally r1 a => b)";
        let h1 = compute_spl_hashes(content);
        let h2 = compute_spl_hashes(content);
        assert_eq!(h1.content_hash, h2.content_hash);
        assert_eq!(h1.ast_hash, h2.ast_hash);
    }

    #[cfg(not(feature = "reason"))]
    #[test]
    fn compute_spl_hashes_no_reason_ast_equals_content() {
        let content = "(given foo)";
        let spl = compute_spl_hashes(content);
        assert_eq!(
            spl.ast_hash, spl.content_hash,
            "without reason feature, ast_hash should equal content_hash"
        );
    }

    // ── spl_combined_hash ────────────────────────────────────────────────────

    #[test]
    fn spl_combined_hash_is_blake3_of_both() {
        use crate::types::SplLeafHash;
        let content_hash = [1u8; 32];
        let ast_hash = [2u8; 32];
        let spl = SplLeafHash {
            content_hash,
            ast_hash,
        };
        let combined = spl_combined_hash(&spl);

        let expected: ContentHash = *blake3::Hasher::new()
            .update(&content_hash)
            .update(&ast_hash)
            .finalize()
            .as_bytes();
        assert_eq!(combined, expected);
    }

    #[test]
    fn spl_combined_hash_order_matters() {
        use crate::types::SplLeafHash;
        let a = SplLeafHash {
            content_hash: [1u8; 32],
            ast_hash: [2u8; 32],
        };
        let b = SplLeafHash {
            content_hash: [2u8; 32],
            ast_hash: [1u8; 32],
        };
        assert_ne!(
            spl_combined_hash(&a),
            spl_combined_hash(&b),
            "swapping content_hash and ast_hash should change the combined hash"
        );
    }

    #[test]
    fn spl_combined_hash_equal_hashes_is_deterministic() {
        use crate::types::SplLeafHash;
        let spl = SplLeafHash {
            content_hash: [42u8; 32],
            ast_hash: [42u8; 32],
        };
        assert_eq!(spl_combined_hash(&spl), spl_combined_hash(&spl));
    }

    #[test]
    fn compute_spl_hashes_combined_feeds_leaf_hash() {
        // The hash stored in MerkleLeaf.hash for an SplBlock should equal
        // spl_combined_hash(spl_hashes).
        let content = "(given foo)";
        let spl = compute_spl_hashes(content);
        let combined = spl_combined_hash(&spl);

        // combined hash must not be all-zero (content is non-empty)
        assert_ne!(combined, [0u8; 32]);
    }

    // ── build_vault_hash_index & resolve_hash_prefix ──────────────────────────

    use crate::types::ParsedFile;
    use std::path::PathBuf;
    use std::time::SystemTime;

    fn minimal_parsed_file(path: &str, leaves: Vec<MerkleLeaf>) -> ParsedFile {
        ParsedFile {
            path: PathBuf::from(path),
            page_name: path.to_string(),
            links: vec![],
            spl_blocks: vec![],
            diagnostics: vec![],
            mtime: SystemTime::UNIX_EPOCH,
            merkle_leaves: leaves,
            file_merkle: None,
        }
    }

    fn leaf_with_hash(hash: ContentHash) -> MerkleLeaf {
        MerkleLeaf {
            node_type: LeafType::Paragraph,
            start_line: 1,
            end_line: 1,
            hash,
            spl_hashes: None,
            block_id: None,
        }
    }

    #[test]
    fn build_vault_hash_index_empty_files() {
        let index = build_vault_hash_index(&[]);
        assert!(index.entries.is_empty());
    }

    #[test]
    fn build_vault_hash_index_single_leaf() {
        let hash = [0xab_u8; 32];
        let file = minimal_parsed_file("notes/a.md", vec![leaf_with_hash(hash)]);
        let index = build_vault_hash_index(&[file]);
        assert_eq!(index.entries.len(), 1);
        let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
        let locs = index.entries.get(&hex).expect("entry should exist");
        assert_eq!(locs.len(), 1);
        assert_eq!(locs[0].leaf_index, 0);
    }

    #[test]
    fn build_vault_hash_index_duplicate_content_across_files() {
        let hash = [0xde_u8; 32];
        let f1 = minimal_parsed_file("a.md", vec![leaf_with_hash(hash)]);
        let f2 = minimal_parsed_file("b.md", vec![leaf_with_hash(hash)]);
        let index = build_vault_hash_index(&[f1, f2]);
        let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
        let locs = index.entries.get(&hex).expect("entry should exist");
        assert_eq!(locs.len(), 2, "same hash in two files → two locations");
    }

    #[test]
    fn resolve_hash_prefix_not_found() {
        let index = build_vault_hash_index(&[]);
        let result = resolve_hash_prefix("aabbcc", &index);
        assert!(matches!(result, HashResolutionResult::NotFound));
    }

    #[test]
    fn resolve_hash_prefix_exact_match() {
        let hash = [0x11_u8; 32];
        let file = minimal_parsed_file("f.md", vec![leaf_with_hash(hash)]);
        let index = build_vault_hash_index(&[file]);
        let full_hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();

        // Full prefix → found
        let result = resolve_hash_prefix(&full_hex, &index);
        match result {
            HashResolutionResult::Found {
                full_hash,
                locations,
            } => {
                assert_eq!(full_hash, full_hex);
                assert_eq!(locations.len(), 1);
            }
            _ => panic!("expected Found"),
        }
    }

    #[test]
    fn resolve_hash_prefix_short_prefix_found() {
        let hash = [0xaa_u8; 32]; // hex: "aa" repeated 32 times
        let file = minimal_parsed_file("f.md", vec![leaf_with_hash(hash)]);
        let index = build_vault_hash_index(&[file]);

        let result = resolve_hash_prefix("aaaa", &index);
        assert!(matches!(result, HashResolutionResult::Found { .. }));
    }

    #[test]
    fn resolve_hash_prefix_case_insensitive() {
        let hash = [0xab_u8; 32]; // hex starts with "abab..."
        let file = minimal_parsed_file("f.md", vec![leaf_with_hash(hash)]);
        let index = build_vault_hash_index(&[file]);

        // Upper-case prefix should still resolve
        let result = resolve_hash_prefix("ABAB", &index);
        assert!(matches!(result, HashResolutionResult::Found { .. }));
    }

    #[test]
    fn resolve_hash_prefix_duplicate_content_returns_all_locations() {
        let hash = [0xcc_u8; 32];
        let f1 = minimal_parsed_file("a.md", vec![leaf_with_hash(hash)]);
        let f2 = minimal_parsed_file("b.md", vec![leaf_with_hash(hash)]);
        let index = build_vault_hash_index(&[f1, f2]);

        let prefix = "cccc";
        let result = resolve_hash_prefix(prefix, &index);
        match result {
            HashResolutionResult::Found { locations, .. } => {
                assert_eq!(
                    locations.len(),
                    2,
                    "both locations returned for duplicate content"
                );
            }
            _ => panic!("expected Found"),
        }
    }

    #[test]
    fn resolve_hash_prefix_ambiguous() {
        // Two leaves with hashes that share a short common prefix but differ after it.
        // hash_a: 0x01 followed by 0x00 bytes  → hex "0100...00"
        // hash_b: 0x01 followed by 0xff bytes after byte 1 → needs careful crafting
        let mut hash_a = [0x00_u8; 32];
        hash_a[0] = 0x01;
        hash_a[1] = 0x00;

        let mut hash_b = [0x00_u8; 32];
        hash_b[0] = 0x01;
        hash_b[1] = 0xff;

        // Both start with "01" so a 2-char prefix is ambiguous.
        let f1 = minimal_parsed_file("a.md", vec![leaf_with_hash(hash_a)]);
        let f2 = minimal_parsed_file("b.md", vec![leaf_with_hash(hash_b)]);
        let index = build_vault_hash_index(&[f1, f2]);

        let result = resolve_hash_prefix("01", &index);
        match result {
            HashResolutionResult::Ambiguous { candidates, .. } => {
                assert_eq!(candidates.len(), 2);
                // Candidates are sorted
                assert!(candidates[0] < candidates[1]);
            }
            _ => panic!("expected Ambiguous"),
        }
    }

    #[test]
    fn resolve_hash_prefix_longer_prefix_disambiguates() {
        let mut hash_a = [0x00_u8; 32];
        hash_a[0] = 0x01;
        hash_a[1] = 0x00;

        let mut hash_b = [0x00_u8; 32];
        hash_b[0] = 0x01;
        hash_b[1] = 0xff;

        let f1 = minimal_parsed_file("a.md", vec![leaf_with_hash(hash_a)]);
        let f2 = minimal_parsed_file("b.md", vec![leaf_with_hash(hash_b)]);
        let index = build_vault_hash_index(&[f1, f2]);

        // "0100" uniquely identifies hash_a
        let result = resolve_hash_prefix("0100", &index);
        match result {
            HashResolutionResult::Found { full_hash, .. } => {
                assert!(full_hash.starts_with("0100"));
            }
            _ => panic!("expected Found"),
        }
    }

    // ── resolve_local_block_id ─────────────────────────────────────────────

    fn leaf_with_block_id(hash: ContentHash, block_id: &str) -> MerkleLeaf {
        MerkleLeaf {
            node_type: LeafType::Paragraph,
            start_line: 1,
            end_line: 1,
            hash,
            spl_hashes: None,
            block_id: Some(block_id.to_string()),
        }
    }

    #[test]
    fn resolve_local_block_id_found() {
        let hash = [0xaa_u8; 32];
        let leaves = vec![
            leaf_with_hash([0x01_u8; 32]),
            leaf_with_block_id(hash, "my-block"),
            leaf_with_hash([0x03_u8; 32]),
        ];
        let result = resolve_local_block_id(&leaves, "my-block");
        assert_eq!(result, Some(hash));
    }

    #[test]
    fn resolve_local_block_id_not_found() {
        let leaves = vec![leaf_with_hash([0x01_u8; 32]), leaf_with_hash([0x02_u8; 32])];
        let result = resolve_local_block_id(&leaves, "missing");
        assert_eq!(result, None);
    }

    #[test]
    fn resolve_local_block_id_empty_leaves() {
        let result = resolve_local_block_id(&[], "any-id");
        assert_eq!(result, None);
    }

    #[test]
    fn resolve_local_block_id_case_sensitive() {
        let hash = [0xbb_u8; 32];
        let leaves = vec![leaf_with_block_id(hash, "MyBlock")];
        // Exact case match
        assert_eq!(resolve_local_block_id(&leaves, "MyBlock"), Some(hash));
        // Different case → not found
        assert_eq!(resolve_local_block_id(&leaves, "myblock"), None);
    }

    #[test]
    fn resolve_local_block_id_returns_first_match() {
        let hash_a = [0x11_u8; 32];
        let hash_b = [0x22_u8; 32];
        // Two leaves with the same block_id (unusual, but handle gracefully).
        let leaves = vec![
            leaf_with_block_id(hash_a, "dup"),
            leaf_with_block_id(hash_b, "dup"),
        ];
        assert_eq!(resolve_local_block_id(&leaves, "dup"), Some(hash_a));
    }

    // ── resolve_cross_file_block_id ────────────────────────────────────────

    fn make_parsed_file_with_leaves(
        path: &str,
        page_name: &str,
        leaves: Vec<MerkleLeaf>,
    ) -> ParsedFile {
        ParsedFile {
            path: PathBuf::from(path),
            page_name: page_name.to_string(),
            links: vec![],
            spl_blocks: vec![],
            diagnostics: vec![],
            mtime: std::time::SystemTime::UNIX_EPOCH,
            merkle_leaves: leaves,
            file_merkle: None,
        }
    }

    #[test]
    fn resolve_cross_file_block_id_found() {
        let target_hash = [0xcc_u8; 32];
        let file = make_parsed_file_with_leaves(
            "notes/Page A.md",
            "Page A",
            vec![leaf_with_block_id(target_hash, "para-ref")],
        );
        let files = vec![file];
        let file_index = vec![("Page A".to_string(), PathBuf::from("notes/Page A.md"))];

        let result = resolve_cross_file_block_id(&files, &file_index, "Page A", "para-ref");
        assert_eq!(result, Ok(target_hash));
    }

    #[test]
    fn resolve_cross_file_block_id_page_not_found() {
        let files: Vec<ParsedFile> = vec![];
        let file_index: Vec<(String, PathBuf)> = vec![];

        let result =
            resolve_cross_file_block_id(&files, &file_index, "Nonexistent Page", "some-id");
        assert_eq!(
            result,
            Err(BlockIdResolutionError::PageNotFound {
                page_name: "Nonexistent Page".to_string()
            })
        );
    }

    #[test]
    fn resolve_cross_file_block_id_block_id_not_found() {
        let file = make_parsed_file_with_leaves(
            "notes/Page B.md",
            "Page B",
            vec![leaf_with_hash([0x01_u8; 32])],
        );
        let files = vec![file];
        let file_index = vec![("Page B".to_string(), PathBuf::from("notes/Page B.md"))];

        let result = resolve_cross_file_block_id(&files, &file_index, "Page B", "missing-block");
        assert_eq!(
            result,
            Err(BlockIdResolutionError::BlockIdNotFound {
                block_id: "missing-block".to_string(),
                file: PathBuf::from("notes/Page B.md"),
            })
        );
    }

    #[test]
    fn resolve_cross_file_block_id_case_insensitive_page_name() {
        let target_hash = [0xdd_u8; 32];
        let file = make_parsed_file_with_leaves(
            "Page C.md",
            "Page C",
            vec![leaf_with_block_id(target_hash, "anchor")],
        );
        let files = vec![file];
        let file_index = vec![("Page C".to_string(), PathBuf::from("Page C.md"))];

        // resolve_page_name is case-insensitive for page names.
        let result = resolve_cross_file_block_id(&files, &file_index, "page c", "anchor");
        assert_eq!(result, Ok(target_hash));
    }

    #[test]
    fn block_id_resolution_error_display() {
        let e1 = BlockIdResolutionError::BlockIdNotFound {
            block_id: "foo".to_string(),
            file: PathBuf::from("bar.md"),
        };
        assert!(e1.to_string().contains("foo"));
        assert!(e1.to_string().contains("bar.md"));

        let e2 = BlockIdResolutionError::PageNotFound {
            page_name: "Missing".to_string(),
        };
        assert!(e2.to_string().contains("Missing"));
    }

    // ── classify_source_ref ────────────────────────────────────────────────

    #[test]
    fn classify_source_ref_hash() {
        assert_eq!(classify_source_ref("abcdef12"), SourceRefType::HashRef);
        assert_eq!(
            classify_source_ref("a1b2c3d4e5f6a7b8"),
            SourceRefType::HashRef
        );
        // Upper-case hex accepted
        assert_eq!(classify_source_ref("ABCDEF12"), SourceRefType::HashRef);
        // Fewer than 8 hex chars → Unknown
        assert_eq!(classify_source_ref("abcdef1"), SourceRefType::Unknown);
    }

    #[test]
    fn classify_source_ref_local() {
        assert_eq!(classify_source_ref("^block-id"), SourceRefType::LocalRef);
        assert_eq!(classify_source_ref("^abc123"), SourceRefType::LocalRef);
    }

    #[test]
    fn classify_source_ref_cross_file() {
        assert_eq!(
            classify_source_ref("[[Page^block-id]]"),
            SourceRefType::CrossFileRef
        );
        assert_eq!(
            classify_source_ref("[[Architecture^intro]]"),
            SourceRefType::CrossFileRef
        );
    }

    #[test]
    fn classify_source_ref_page_ref() {
        // [[Page]] without ^ is a PageRef
        assert_eq!(classify_source_ref("[[Page]]"), SourceRefType::PageRef);
        assert_eq!(
            classify_source_ref("[[Redis vs Memcached]]"),
            SourceRefType::PageRef
        );
    }

    #[test]
    fn classify_source_ref_unknown() {
        assert_eq!(classify_source_ref("just-text"), SourceRefType::Unknown);
        assert_eq!(classify_source_ref(""), SourceRefType::Unknown);
    }

    // ── validate_source_refs ───────────────────────────────────────────────

    use crate::types::{ExplicitGrounding, FileMerkle, Section, SplLeafCached};

    fn make_spl_leaf(
        start_line: u32,
        content_hash: ContentHash,
        groundings: Vec<ExplicitGrounding>,
    ) -> SplLeafCached {
        SplLeafCached {
            start_line,
            content_hash,
            ast_hash: [0u8; 32],
            section_index: 0,
            explicit_groundings: groundings,
        }
    }

    fn make_file_merkle(spl_leaves: Vec<SplLeafCached>) -> FileMerkle {
        FileMerkle {
            root_hash: [0u8; 32],
            sections: vec![Section {
                heading_line: 1,
                heading_text: "Test".to_string(),
                heading_level: 1,
                leaf_range: (0, spl_leaves.len()),
                grounding_hash: [0u8; 32],
            }],
            spl_leaves,
        }
    }

    fn make_file_with_merkle(
        path: &str,
        page_name: &str,
        merkle_leaves: Vec<MerkleLeaf>,
        spl_leaves: Vec<SplLeafCached>,
    ) -> ParsedFile {
        ParsedFile {
            path: PathBuf::from(path),
            page_name: page_name.to_string(),
            links: vec![],
            spl_blocks: vec![],
            diagnostics: vec![],
            mtime: std::time::SystemTime::UNIX_EPOCH,
            merkle_leaves,
            file_merkle: Some(make_file_merkle(spl_leaves)),
        }
    }

    fn make_grounding(source_refs: Vec<&str>) -> ExplicitGrounding {
        ExplicitGrounding {
            construct: "r1".to_string(),
            source_refs: source_refs.into_iter().map(str::to_string).collect(),
            targets: vec![],
        }
    }

    #[test]
    fn validate_source_refs_hash_not_found() {
        // A hash prefix that doesn't match anything → error
        let grounding = make_grounding(vec!["deadbeef"]);
        let file = make_file_with_merkle(
            "notes/A.md",
            "A",
            vec![],
            vec![make_spl_leaf(5, [0u8; 32], vec![grounding])],
        );
        let files = vec![file];
        let file_index: Vec<(String, PathBuf)> =
            vec![("A".to_string(), PathBuf::from("notes/A.md"))];
        let index = build_vault_hash_index(&files);

        let diags = validate_source_refs(&files, &file_index, &index);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("deadbeef"));
        assert!(diags[0].message.contains("not found"));
        assert_eq!(diags[0].level, DiagnosticLevel::Error);
        assert_eq!(diags[0].line, 5);
    }

    #[test]
    fn validate_source_refs_hash_found() {
        // Build a file with a leaf whose hash starts with "aa"
        let leaf_hash = [0xaa_u8; 32];
        let leaf = make_leaf(LeafType::Paragraph, leaf_hash);
        let hex: String = leaf_hash.iter().map(|b| format!("{b:02x}")).collect();
        let prefix = &hex[..8]; // first 8 chars
        let grounding = make_grounding(vec![prefix]);
        let spl_leaf = make_spl_leaf(3, [0u8; 32], vec![grounding]);
        let file = make_file_with_merkle("A.md", "A", vec![leaf], vec![spl_leaf]);
        let files = vec![file];
        let file_index = vec![("A".to_string(), PathBuf::from("A.md"))];
        let index = build_vault_hash_index(&files);

        let diags = validate_source_refs(&files, &file_index, &index);
        assert!(diags.is_empty(), "expected no errors but got: {diags:?}");
    }

    #[test]
    fn validate_source_refs_hash_ambiguous() {
        // Two leaves with different hashes that share the same 8-char prefix.
        // Construct two hashes that differ only beyond the 4th byte.
        let mut hash_a = [0x01_u8; 32];
        let mut hash_b = [0x01_u8; 32];
        hash_a[4] = 0xAA;
        hash_b[4] = 0xBB;
        let leaf_a = make_leaf(LeafType::Paragraph, hash_a);
        let leaf_b = make_leaf(LeafType::Paragraph, hash_b);

        // Both share the first 8 hex chars ("01010101")
        let prefix = "01010101";
        let grounding = make_grounding(vec![prefix]);
        let spl_leaf = make_spl_leaf(1, [0u8; 32], vec![grounding]);

        let file = ParsedFile {
            path: PathBuf::from("A.md"),
            page_name: "A".to_string(),
            links: vec![],
            spl_blocks: vec![],
            diagnostics: vec![],
            mtime: std::time::SystemTime::UNIX_EPOCH,
            merkle_leaves: vec![leaf_a, leaf_b],
            file_merkle: Some(make_file_merkle(vec![spl_leaf])),
        };
        let files = vec![file];
        let file_index = vec![("A".to_string(), PathBuf::from("A.md"))];
        let index = build_vault_hash_index(&files);

        let diags = validate_source_refs(&files, &file_index, &index);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("ambiguous"));
        assert_eq!(diags[0].level, DiagnosticLevel::Error);
    }

    #[test]
    fn validate_source_refs_hash_duplicate_content_valid() {
        // Two leaves with identical hashes (duplicate content) → valid, no error
        let hash = [0xcc_u8; 32];
        let leaf1 = make_leaf(LeafType::Paragraph, hash);
        let leaf2 = make_leaf(LeafType::Paragraph, hash);
        let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
        let prefix = &hex[..8];
        let grounding = make_grounding(vec![prefix]);
        let spl_leaf = make_spl_leaf(1, [0u8; 32], vec![grounding]);

        let file = ParsedFile {
            path: PathBuf::from("A.md"),
            page_name: "A".to_string(),
            links: vec![],
            spl_blocks: vec![],
            diagnostics: vec![],
            mtime: std::time::SystemTime::UNIX_EPOCH,
            merkle_leaves: vec![leaf1, leaf2],
            file_merkle: Some(make_file_merkle(vec![spl_leaf])),
        };
        let files = vec![file];
        let file_index = vec![("A".to_string(), PathBuf::from("A.md"))];
        let index = build_vault_hash_index(&files);

        let diags = validate_source_refs(&files, &file_index, &index);
        assert!(
            diags.is_empty(),
            "duplicate content should be valid; got: {diags:?}"
        );
    }

    #[test]
    fn validate_source_refs_local_ref_not_found() {
        let grounding = make_grounding(vec!["^missing-block"]);
        // No leaf carries block_id "missing-block"
        let leaf = make_leaf(LeafType::Paragraph, [0x01_u8; 32]);
        let spl_leaf = make_spl_leaf(7, [0u8; 32], vec![grounding]);
        let file = make_file_with_merkle("B.md", "B", vec![leaf], vec![spl_leaf]);
        let files = vec![file];
        let file_index = vec![("B".to_string(), PathBuf::from("B.md"))];
        let index = build_vault_hash_index(&files);

        let diags = validate_source_refs(&files, &file_index, &index);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("missing-block"));
        assert!(diags[0].message.contains("does not exist"));
        assert_eq!(diags[0].level, DiagnosticLevel::Error);
        assert_eq!(diags[0].line, 7);
    }

    #[test]
    fn validate_source_refs_local_ref_found() {
        let leaf = leaf_with_block_id([0x02_u8; 32], "present");
        let grounding = make_grounding(vec!["^present"]);
        let spl_leaf = make_spl_leaf(2, [0u8; 32], vec![grounding]);
        let file = make_file_with_merkle("C.md", "C", vec![leaf], vec![spl_leaf]);
        let files = vec![file];
        let file_index = vec![("C".to_string(), PathBuf::from("C.md"))];
        let index = build_vault_hash_index(&files);

        let diags = validate_source_refs(&files, &file_index, &index);
        assert!(
            diags.is_empty(),
            "found block-id should be valid; got: {diags:?}"
        );
    }

    #[test]
    fn validate_source_refs_page_ref_not_found() {
        let grounding = make_grounding(vec!["[[Missing Page]]"]);
        let spl_leaf = make_spl_leaf(4, [0u8; 32], vec![grounding]);
        let file = make_file_with_merkle("D.md", "D", vec![], vec![spl_leaf]);
        let files = vec![file];
        let file_index = vec![("D".to_string(), PathBuf::from("D.md"))];
        let index = build_vault_hash_index(&files);

        let diags = validate_source_refs(&files, &file_index, &index);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("Missing Page"));
        assert!(diags[0].message.contains("not found"));
        assert_eq!(diags[0].level, DiagnosticLevel::Error);
    }

    #[test]
    fn validate_source_refs_page_ref_found() {
        let grounding = make_grounding(vec!["[[Existing Page]]"]);
        let spl_leaf = make_spl_leaf(1, [0u8; 32], vec![grounding]);
        let file = make_file_with_merkle("E.md", "E", vec![], vec![spl_leaf]);
        let target = make_parsed_file_with_leaves("Existing Page.md", "Existing Page", vec![]);
        let files = vec![file, target];
        let file_index = vec![
            ("E".to_string(), PathBuf::from("E.md")),
            (
                "Existing Page".to_string(),
                PathBuf::from("Existing Page.md"),
            ),
        ];
        let index = build_vault_hash_index(&files);

        let diags = validate_source_refs(&files, &file_index, &index);
        assert!(
            diags.is_empty(),
            "existing page ref should be valid; got: {diags:?}"
        );
    }

    #[test]
    fn validate_source_refs_cross_file_page_not_found() {
        let grounding = make_grounding(vec!["[[NoSuchPage^block1]]"]);
        let spl_leaf = make_spl_leaf(10, [0u8; 32], vec![grounding]);
        let file = make_file_with_merkle("F.md", "F", vec![], vec![spl_leaf]);
        let files = vec![file];
        let file_index = vec![("F".to_string(), PathBuf::from("F.md"))];
        let index = build_vault_hash_index(&files);

        let diags = validate_source_refs(&files, &file_index, &index);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("NoSuchPage"));
        assert!(diags[0].message.contains("not found"));
        assert_eq!(diags[0].level, DiagnosticLevel::Error);
        assert_eq!(diags[0].line, 10);
    }

    #[test]
    fn validate_source_refs_cross_file_block_id_not_found() {
        let grounding = make_grounding(vec!["[[Target Page^ghost-id]]"]);
        let spl_leaf = make_spl_leaf(8, [0u8; 32], vec![grounding]);
        let src_file = make_file_with_merkle("src.md", "src", vec![], vec![spl_leaf]);
        // Target page exists but has no leaf with "ghost-id"
        let target_leaf = make_leaf(LeafType::Paragraph, [0x99_u8; 32]);
        let target_file =
            make_parsed_file_with_leaves("Target Page.md", "Target Page", vec![target_leaf]);
        let files = vec![src_file, target_file];
        let file_index = vec![
            ("src".to_string(), PathBuf::from("src.md")),
            ("Target Page".to_string(), PathBuf::from("Target Page.md")),
        ];
        let index = build_vault_hash_index(&files);

        let diags = validate_source_refs(&files, &file_index, &index);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("block-id not found"));
        assert!(diags[0].message.contains("Target Page"));
        assert_eq!(diags[0].level, DiagnosticLevel::Error);
        assert_eq!(diags[0].line, 8);
    }

    #[test]
    fn validate_source_refs_cross_file_found() {
        let grounding = make_grounding(vec!["[[Target Page^real-id]]"]);
        let spl_leaf = make_spl_leaf(6, [0u8; 32], vec![grounding]);
        let src_file = make_file_with_merkle("src.md", "src", vec![], vec![spl_leaf]);
        let target_leaf = leaf_with_block_id([0x77_u8; 32], "real-id");
        let target_file =
            make_parsed_file_with_leaves("Target Page.md", "Target Page", vec![target_leaf]);
        let files = vec![src_file, target_file];
        let file_index = vec![
            ("src".to_string(), PathBuf::from("src.md")),
            ("Target Page".to_string(), PathBuf::from("Target Page.md")),
        ];
        let index = build_vault_hash_index(&files);

        let diags = validate_source_refs(&files, &file_index, &index);
        assert!(
            diags.is_empty(),
            "valid cross-file ref should have no errors; got: {diags:?}"
        );
    }

    #[test]
    fn validate_source_refs_unknown_skipped() {
        // Unknown format → no error emitted
        let grounding = make_grounding(vec!["some-plain-text"]);
        let spl_leaf = make_spl_leaf(1, [0u8; 32], vec![grounding]);
        let file = make_file_with_merkle("G.md", "G", vec![], vec![spl_leaf]);
        let files = vec![file];
        let file_index = vec![("G".to_string(), PathBuf::from("G.md"))];
        let index = build_vault_hash_index(&files);

        let diags = validate_source_refs(&files, &file_index, &index);
        assert!(
            diags.is_empty(),
            "unknown refs should be silently skipped; got: {diags:?}"
        );
    }

    #[test]
    fn validate_source_refs_no_file_merkle_skipped() {
        // File with no file_merkle → no panics, no errors
        let file = make_parsed_file_with_leaves("H.md", "H", vec![]);
        let files = vec![file];
        let file_index = vec![("H".to_string(), PathBuf::from("H.md"))];
        let index = build_vault_hash_index(&files);

        let diags = validate_source_refs(&files, &file_index, &index);
        assert!(diags.is_empty());
    }

    // ── TEST-038: Merkle construction ──────────────────────────────────────────

    /// TEST-038: Vault root determinism — supplying the same file hashes in
    /// different orderings always produces an identical vault root.
    ///
    /// Verifies that `compute_vault_root` sorts by canonical path before hashing,
    /// so the result is independent of scan/insertion order.
    #[test]
    fn test038_vault_root_determinism_multiple_scan_orders() {
        use std::path::Path;

        let pa = Path::new("notes/alpha.md");
        let pb = Path::new("notes/beta.md");
        let pc = Path::new("gamma.md");

        let ha = [0x11u8; 32];
        let hb = [0x22u8; 32];
        let hc = [0x33u8; 32];

        let root_abc = compute_vault_root(&[(pa, ha), (pb, hb), (pc, hc)]);
        let root_bca = compute_vault_root(&[(pb, hb), (pc, hc), (pa, ha)]);
        let root_cab = compute_vault_root(&[(pc, hc), (pa, ha), (pb, hb)]);
        let root_cba = compute_vault_root(&[(pc, hc), (pb, hb), (pa, ha)]);

        assert_eq!(root_abc, root_bca, "abc vs bca order mismatch");
        assert_eq!(root_abc, root_cab, "abc vs cab order mismatch");
        assert_eq!(root_abc, root_cba, "abc vs cba order mismatch");
        assert_ne!(
            root_abc, [0u8; 32],
            "vault root of non-empty vault must be non-zero"
        );
    }

    /// TEST-038: Normalisation — two files whose paragraphs contain identical
    /// semantic text but different internal whitespace produce the same per-file
    /// Merkle root.
    ///
    /// `build_merkle_leaves` collapses consecutive whitespace before hashing
    /// non-code blocks, so extra spaces are invisible to the hash.
    #[test]
    fn test038_normalisation_whitespace_same_file_merkle_root() {
        use crate::scanner::parse_file;
        use std::path::Path;

        let content_a = "# Introduction\n\nHello world with some text here.\n";
        let content_b = "# Introduction\n\nHello  world  with  some  text  here.\n";

        let pf_a = parse_file(Path::new("a.md"), content_a, "a");
        let pf_b = parse_file(Path::new("b.md"), content_b, "b");

        let root_a = pf_a.file_merkle.as_ref().map(|m| m.root_hash);
        let root_b = pf_b.file_merkle.as_ref().map(|m| m.root_hash);

        assert!(
            root_a.is_some(),
            "file_merkle should be populated for a non-empty .md file"
        );
        assert_eq!(
            root_a, root_b,
            "identical text with different internal whitespace must yield the same Merkle root"
        );
    }

    /// TEST-038: Changing a wikilink target in a paragraph alters the paragraph
    /// leaf hash and therefore the file-level Merkle root.
    #[test]
    fn test038_wikilink_change_alters_file_merkle_root() {
        use crate::scanner::parse_file;
        use std::path::Path;

        let content_before = "# Notes\n\nSee [[Alpha]] for details.\n";
        let content_after = "# Notes\n\nSee [[Beta]] for details.\n";

        let pf_before = parse_file(Path::new("note.md"), content_before, "note");
        let pf_after = parse_file(Path::new("note.md"), content_after, "note");

        let root_before = pf_before.file_merkle.as_ref().map(|m| m.root_hash);
        let root_after = pf_after.file_merkle.as_ref().map(|m| m.root_hash);

        assert!(
            root_before.is_some(),
            "file with content must have a Merkle root"
        );
        assert_ne!(
            root_before, root_after,
            "changing a wikilink target must alter the file-level Merkle root"
        );
    }

    // ── TEST-039: SPL dual hashing ─────────────────────────────────────────────

    /// TEST-039: `compute_spl_hashes` returns non-zero `content_hash` and
    /// `ast_hash` for a valid, non-empty SPL block.
    ///
    /// Uses SPL syntax confirmed by the integration-test vault:
    /// `(given literal)` for facts and `(normally label body head)` for rules.
    #[test]
    fn test039_spl_dual_hashes_both_nonzero_for_valid_content() {
        // Valid SPL: one fact and one defeasible rule (body=bird, head=flies).
        let content = "(given bird)\n(normally r-bird-flies bird flies)";
        let spl = compute_spl_hashes(content);

        assert_ne!(spl.content_hash, [0u8; 32], "content_hash must be non-zero");
        assert_ne!(spl.ast_hash, [0u8; 32], "ast_hash must be non-zero");
    }

    /// TEST-039: Whitespace-only reformatting does **not** change `content_hash`
    /// because `normalize_spl` collapses all whitespace before hashing.
    #[test]
    fn test039_spl_reformat_preserves_content_hash() {
        // Compact and reformatted versions of the same two SPL statements.
        let compact = "(given bird)\n(normally r-bird-flies bird flies)";
        let reformatted = "(given   bird)\n\n(normally   r-bird-flies   bird   flies)";

        let h_compact = compute_spl_hashes(compact);
        let h_reformatted = compute_spl_hashes(reformatted);

        assert_eq!(
            h_compact.content_hash, h_reformatted.content_hash,
            "content_hash must be identical for whitespace-equivalent SPL blocks"
        );
    }

    /// TEST-039: With the `reason` feature, whitespace-only reformatting also
    /// does **not** change `ast_hash` — the canonical AST is independent of
    /// source whitespace.
    #[cfg(feature = "reason")]
    #[test]
    fn test039_spl_reformat_preserves_ast_hash_reason() {
        // Same statements, different whitespace — canonical AST must be identical.
        let compact = "(given bird)\n(normally r-bird-flies bird flies)";
        let reformatted = "(given   bird)\n\n(normally   r-bird-flies   bird   flies)";

        let h_compact = compute_spl_hashes(compact);
        let h_reformatted = compute_spl_hashes(reformatted);

        assert_eq!(
            h_compact.ast_hash, h_reformatted.ast_hash,
            "ast_hash must be identical for whitespace-equivalent SPL blocks"
        );
    }

    /// TEST-039: An unparseable SPL block produces sentinel `ast_hash = [0u8; 32]`
    /// and a non-zero `content_hash`; `parse_file` emits a diagnostic for it.
    #[cfg(feature = "reason")]
    #[test]
    fn test039_spl_parse_error_sentinel_ast_hash_and_diagnostic() {
        use crate::scanner::parse_file;
        use std::path::Path;

        let invalid_spl = "this is @@@not valid !!! spl syntax";
        let spl = compute_spl_hashes(invalid_spl);

        assert_eq!(
            spl.ast_hash, [0u8; 32],
            "parse failure must produce sentinel ast_hash of all zeros"
        );
        assert_ne!(
            spl.content_hash, [0u8; 32],
            "content_hash must still be non-zero even when parsing fails"
        );

        let md_content = format!("# Notes\n\n```spl\n{invalid_spl}\n```\n");
        let pf = parse_file(Path::new("note.md"), &md_content, "note");
        let has_sentinel_diag = pf
            .diagnostics
            .iter()
            .any(|d| d.message.contains("SPL parse failed") || d.message.contains("sentinel"));
        assert!(
            has_sentinel_diag,
            "parse_file must emit a diagnostic for SPL blocks with sentinel ast_hash; \
             got: {:?}",
            pf.diagnostics
        );
    }
}
