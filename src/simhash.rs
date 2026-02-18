use serde::Serialize;

/// Fingerprint index for fuzzy page name search via Hamming distance.
///
/// Uses a word-level Bloom filter approach: each word, word stem, and
/// character trigram sets specific bit positions in a 64-bit fingerprint.
/// Pages sharing words produce overlapping bit patterns, resulting in
/// low Hamming distance — enabling "poor person's semantic search".
pub struct SimHashIndex {
    entries: Vec<SimHashEntry>,
}

struct SimHashEntry {
    page_name: String,
    path: String,
    fingerprint: u64,
}

#[derive(Debug, Serialize)]
pub struct SimilarResult {
    pub page: String,
    pub distance: u32,
    pub path: String,
}

impl SimHashIndex {
    /// Build a fingerprint index from page names and paths
    pub fn build(pages: &[(String, String)]) -> Self {
        let entries = pages
            .iter()
            .map(|(name, path)| SimHashEntry {
                fingerprint: compute_simhash(name),
                page_name: name.clone(),
                path: path.clone(),
            })
            .collect();
        SimHashIndex { entries }
    }

    /// Search for pages similar to the query
    pub fn search(&self, query: &str, threshold: u32, limit: usize) -> Vec<SimilarResult> {
        let query_hash = compute_simhash(query);
        let mut results: Vec<SimilarResult> = self
            .entries
            .iter()
            .filter_map(|entry| {
                let distance = hamming_distance(query_hash, entry.fingerprint);
                if distance <= threshold {
                    Some(SimilarResult {
                        page: entry.page_name.clone(),
                        distance,
                        path: entry.path.clone(),
                    })
                } else {
                    None
                }
            })
            .collect();
        results.sort_by_key(|r| r.distance);
        results.truncate(limit);
        results
    }
}

/// Compute a 64-bit fingerprint for a string using word-level features.
///
/// Each word and word-stem sets specific bit positions (Bloom filter style).
/// Character trigrams within each word provide typo tolerance.
/// Pages sharing words will share bit positions → low Hamming distance.
pub fn compute_simhash(text: &str) -> u64 {
    let normalized = normalize(text);

    if normalized.is_empty() {
        return 0;
    }

    let mut fingerprint: u64 = 0;
    let words: Vec<&str> = normalized.split_whitespace().collect();

    for word in &words {
        // Whole word → 3 bit positions (primary semantic signal)
        set_feature_bits(&mut fingerprint, &format!("w:{word}"), 3);

        // 4-char stem → 2 bit positions (morphological matching)
        if word.len() >= 4 {
            let stem: String = word.chars().take(4).collect();
            set_feature_bits(&mut fingerprint, &format!("s:{stem}"), 2);
        }

        // Character trigrams → 1 bit position each (typo tolerance)
        let chars: Vec<char> = word.chars().collect();
        for w in chars.windows(3) {
            let trigram: String = w.iter().collect();
            set_feature_bits(&mut fingerprint, &format!("t:{trigram}"), 1);
        }
    }

    fingerprint
}

/// Set `num_bits` bit positions in the fingerprint for a feature.
/// Uses different byte-segments of the hash for each bit position.
fn set_feature_bits(fingerprint: &mut u64, feature: &str, num_bits: usize) {
    let hash = fnv1a_hash(feature);
    for i in 0..num_bits {
        let bit_pos = (hash >> (i * 8)) & 63;
        *fingerprint |= 1 << bit_pos;
    }
}

/// Hamming distance between two 64-bit fingerprints
pub fn hamming_distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Normalize text: lowercase, collapse whitespace, strip punctuation
fn normalize(text: &str) -> String {
    text.chars()
        .filter_map(|c| {
            if c.is_alphanumeric() {
                Some(c.to_lowercase().next().unwrap_or(c))
            } else if c.is_whitespace() || c == '-' || c == '_' {
                Some(' ')
            } else {
                None
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// FNV-1a hash for a string, producing a 64-bit value
fn fnv1a_hash(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in s.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_strings_zero_distance() {
        let h1 = compute_simhash("Zettelkasten Method");
        let h2 = compute_simhash("Zettelkasten Method");
        assert_eq!(hamming_distance(h1, h2), 0);
    }

    #[test]
    fn test_shared_word_low_distance() {
        // Pages sharing a word should have low Hamming distance
        let notes = compute_simhash("notes");
        let atomic_notes = compute_simhash("Atomic Notes");
        let evergreen_notes = compute_simhash("Evergreen Notes");
        let zettelkasten = compute_simhash("Zettelkasten");

        let dist_atomic = hamming_distance(notes, atomic_notes);
        let dist_evergreen = hamming_distance(notes, evergreen_notes);
        let dist_unrelated = hamming_distance(notes, zettelkasten);

        assert!(
            dist_atomic < dist_unrelated,
            "shared word 'notes' should give lower distance: Atomic Notes={dist_atomic}, Zettelkasten={dist_unrelated}"
        );
        assert!(
            dist_evergreen < dist_unrelated,
            "shared word 'notes' should give lower distance: Evergreen Notes={dist_evergreen}, Zettelkasten={dist_unrelated}"
        );
    }

    #[test]
    fn test_typo_tolerance() {
        // Typo in a word should still have lower distance than unrelated
        let correct = compute_simhash("zettelkasten");
        let typo = compute_simhash("zettelkasen");
        let unrelated = compute_simhash("rust programming");

        let typo_dist = hamming_distance(correct, typo);
        let unrelated_dist = hamming_distance(correct, unrelated);

        assert!(
            typo_dist < unrelated_dist,
            "typo should have lower distance ({typo_dist}) than unrelated ({unrelated_dist})"
        );
    }

    #[test]
    fn test_word_search_finds_matching_pages() {
        // Simulate searching "notes" against a set of page names
        let pages = vec![
            ("Atomic Notes".to_string(), "a.md".to_string()),
            ("Literature Notes".to_string(), "b.md".to_string()),
            ("Evergreen Notes".to_string(), "c.md".to_string()),
            ("Zettelkasten".to_string(), "d.md".to_string()),
            ("Knowledge Graph".to_string(), "e.md".to_string()),
        ];
        let index = SimHashIndex::build(&pages);
        let results = index.search("notes", 12, 10);

        let found: Vec<&str> = results.iter().map(|r| r.page.as_str()).collect();
        assert!(found.contains(&"Atomic Notes"), "should find 'Atomic Notes', got: {found:?}");
        assert!(found.contains(&"Literature Notes"), "should find 'Literature Notes', got: {found:?}");
        assert!(found.contains(&"Evergreen Notes"), "should find 'Evergreen Notes', got: {found:?}");
        assert!(!found.contains(&"Knowledge Graph"), "should NOT find 'Knowledge Graph', got: {found:?}");
    }

    #[test]
    fn test_knowledge_search() {
        let pages = vec![
            ("Knowledge Management".to_string(), "a.md".to_string()),
            ("Knowledge Graph".to_string(), "b.md".to_string()),
            ("Atomic Notes".to_string(), "c.md".to_string()),
            ("Daily Note Practice".to_string(), "d.md".to_string()),
        ];
        let index = SimHashIndex::build(&pages);
        let results = index.search("knowledge", 10, 10);

        let found: Vec<&str> = results.iter().map(|r| r.page.as_str()).collect();
        assert!(found.contains(&"Knowledge Management"), "should find 'Knowledge Management', got: {found:?}");
        assert!(found.contains(&"Knowledge Graph"), "should find 'Knowledge Graph', got: {found:?}");
    }

    #[test]
    fn test_different_strings_high_distance() {
        let h1 = compute_simhash("Zettelkasten Method");
        let h2 = compute_simhash("Rust Programming");
        assert!(
            hamming_distance(h1, h2) > 5,
            "completely different strings should have high distance"
        );
    }
}
