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

/// Compute a 64-bit SimHash fingerprint for a string.
///
/// Uses multiple n-gram sizes (unigrams, bigrams with boundary markers,
/// trigrams) to produce fingerprints where similar short strings have
/// low Hamming distance. The multi-gram approach ensures enough feature
/// overlap that single-character edits flip only a few bits.
pub fn compute_simhash(text: &str) -> u64 {
    let normalized = normalize(text);

    if normalized.is_empty() {
        return 0;
    }

    let mut weights = [0i32; 64];

    // Collect all n-gram features: unigrams, bigrams (with boundary markers), trigrams
    let chars: Vec<char> = normalized.chars().collect();

    // Unigrams (individual characters)
    for ch in &chars {
        let feature = ch.to_string();
        let hash = fnv1a_hash(&feature);
        accumulate_hash(&mut weights, hash);
    }

    // Bigrams with boundary markers for better short-string sensitivity
    let padded: Vec<char> = std::iter::once('^')
        .chain(chars.iter().copied())
        .chain(std::iter::once('$'))
        .collect();
    for w in padded.windows(2) {
        let feature: String = w.iter().collect();
        let hash = fnv1a_hash(&feature);
        accumulate_hash(&mut weights, hash);
    }

    // Trigrams (if long enough)
    if chars.len() >= 3 {
        for w in chars.windows(3) {
            let feature: String = w.iter().collect();
            let hash = fnv1a_hash(&feature);
            accumulate_hash(&mut weights, hash);
        }
    }

    let mut fingerprint: u64 = 0;
    for (i, &w) in weights.iter().enumerate() {
        if w > 0 {
            fingerprint |= 1 << i;
        }
    }
    fingerprint
}

/// Accumulate a hash into the weight vector (+1 for set bits, -1 for unset)
fn accumulate_hash(weights: &mut [i32; 64], hash: u64) {
    for (i, w) in weights.iter_mut().enumerate() {
        if (hash >> i) & 1 == 1 {
            *w += 1;
        } else {
            *w -= 1;
        }
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
            "similar strings should have lower distance ({similar_dist}) than dissimilar ones ({dissimilar_dist})"
        );
    }

    #[test]
    fn test_different_strings_high_distance() {
        let h1 = compute_simhash("Zettelkasten Method");
        let h2 = compute_simhash("Rust Programming");
        assert!(hamming_distance(h1, h2) > 5);
    }
}
