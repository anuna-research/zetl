use serde::Serialize;

/// SimHash index for fuzzy page name matching
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
    /// Build a SimHash index from page names and paths
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

/// Compute a 64-bit SimHash fingerprint for a string
pub fn compute_simhash(text: &str) -> u64 {
    let normalized = normalize(text);
    let trigrams = char_trigrams(&normalized);

    if trigrams.is_empty() {
        return 0;
    }

    let mut weights = [0i32; 64];

    for trigram in &trigrams {
        let hash = fnv1a_hash(trigram);
        for i in 0..64 {
            if (hash >> i) & 1 == 1 {
                weights[i] += 1;
            } else {
                weights[i] -= 1;
            }
        }
    }

    let mut fingerprint: u64 = 0;
    for i in 0..64 {
        if weights[i] > 0 {
            fingerprint |= 1 << i;
        }
    }
    fingerprint
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

/// Extract character trigrams from text
fn char_trigrams(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() < 3 {
        return vec![text.to_string()];
    }
    chars.windows(3).map(|w| w.iter().collect()).collect()
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
    fn test_similar_strings_low_distance() {
        let h1 = compute_simhash("zettelkasten");
        let h2 = compute_simhash("zettelkasen");
        // SimHash with character trigrams may produce moderate distances
        // for short strings with small edits. The key property is that
        // similar strings produce lower distance than dissimilar ones.
        let similar_dist = hamming_distance(h1, h2);
        let dissimilar_dist = hamming_distance(h1, compute_simhash("rust programming"));
        assert!(
            similar_dist < dissimilar_dist,
            "similar strings should have lower distance ({}) than dissimilar ones ({})",
            similar_dist,
            dissimilar_dist
        );
    }

    #[test]
    fn test_different_strings_high_distance() {
        let h1 = compute_simhash("Zettelkasten Method");
        let h2 = compute_simhash("Rust Programming");
        assert!(hamming_distance(h1, h2) > 5);
    }
}
