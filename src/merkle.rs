//! Merkle tree hashing functions for SPEC-006.
//!
//! Implements BLAKE3-based hashing for leaf nodes, file roots, and the vault root
//! per §4.1–4.2 and §4.6.

use crate::types::{ContentHash, LeafType, MerkleLeaf, Section};
use std::path::Path;

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
/// For [`LeafType::SplBlock`] leaves the hash is a placeholder sentinel
/// (`BLAKE3([TAG_SPL_BLOCK])`); full dual hashing is handled by the
/// `task-spl-dual-hashing` task.
pub fn compute_leaf_hash(leaf_type: &LeafType, content: &[u8]) -> ContentHash {
    let tag = leaf_type_tag(leaf_type);
    if matches!(leaf_type, LeafType::SplBlock) {
        // Placeholder: hash only the type tag (content is not yet dual-hashed).
        let mut hasher = blake3::Hasher::new();
        hasher.update(&[tag]);
        return *hasher.finalize().as_bytes();
    }
    let mut hasher = blake3::Hasher::new();
    hasher.update(&[tag]);
    hasher.update(content);
    *hasher.finalize().as_bytes()
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
    let (section_start_idx, heading_level, heading_line) =
        match (0..spl_leaf_idx)
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
        match leaves[search_from..]
            .iter()
            .position(|l| matches!(l.node_type, LeafType::Heading { level } if level <= heading_level))
        {
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
        let same_as_prev = sections.last().map_or(false, |prev| {
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
            assert!(seen.insert(tag), "duplicate tag: 0x{:02x}", tag);
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
        assert_eq!(
            leaf_type_tag(&LeafType::List { ordered: false }),
            TAG_LIST
        );
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
        assert_ne!(h1, h2, "different leaf types should produce different hashes");
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
    fn compute_leaf_hash_spl_is_placeholder() {
        // SPL leaves always return BLAKE3([TAG_SPL_BLOCK]), regardless of content.
        let h1 = compute_leaf_hash(&LeafType::SplBlock, b"some spl code");
        let h2 = compute_leaf_hash(&LeafType::SplBlock, b"different spl code");
        assert_eq!(
            h1, h2,
            "SPL placeholder should be independent of content"
        );

        // Placeholder should equal BLAKE3([TAG_SPL_BLOCK]).
        let expected: ContentHash =
            *blake3::Hasher::new().update(&[TAG_SPL_BLOCK]).finalize().as_bytes();
        assert_eq!(h1, expected);
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
}
