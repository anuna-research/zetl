//! `zetl feed forget` per REQ-3834 + CON-3814 + T22.
//!
//! Pure-core: planning + tombstone records. Effectful shell:
//! filesystem deletion, tombstone-file appending.

use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};

/// Pattern shape per REQ-3834.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForgetPattern {
    /// Glob over slugs (e.g. `foo/**`).
    SlugGlob(String),
    /// Prefix match against item GUID.
    GuidPrefix(String),
    /// Prefix match against content_hash.
    ContentHashPrefix(String),
}

impl ForgetPattern {
    /// Best-effort detection of pattern shape from a string. Heuristic:
    ///
    ///   * Looks like a hex prefix (`[0-9a-f]+`) of length 4..64 ->
    ///     `ContentHashPrefix`
    ///   * Starts with `urn:` or contains a colon followed by year
    ///     digits -> `GuidPrefix`
    ///   * Otherwise treated as a glob slug pattern.
    pub fn detect(input: &str) -> ForgetPattern {
        if input.starts_with("urn:") || input.starts_with("tag:") {
            return ForgetPattern::GuidPrefix(input.to_string());
        }
        if input.len() >= 4
            && input.len() <= 64
            && input.chars().all(|c| c.is_ascii_hexdigit())
        {
            return ForgetPattern::ContentHashPrefix(input.to_string());
        }
        ForgetPattern::SlugGlob(input.to_string())
    }
}

/// One inbox / archive item under `forget` review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForgetCandidate {
    pub path: String,
    pub slug: String,
    pub guid: String,
    pub content_hash: Option<String>,
    /// True if the item lives under `archived/` rather than `inbox/`.
    pub in_archive: bool,
}

/// Tombstone record per REQ-3834. Persisted to
/// `.zetl/feeds/<sub-id>/tombstones.jsonl`; consulted on every
/// subsequent fetch (T22 mitigation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tombstone {
    pub guid: Option<String>,
    pub link: Option<String>,
    pub content_hash: Option<String>,
    pub erased_at: String,
    pub reason: Option<String>,
}

/// Plan a forget pass. Pure: takes the candidate set + parameters,
/// returns the set of items to remove + tombstones to append.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForgetPlan {
    pub remove: Vec<ForgetCandidate>,
    pub tombstones: Vec<Tombstone>,
    /// Items that match but lie under archived/ when --include-archive
    /// was *not* set; surfaced for warning but not removed.
    pub skipped_archive: Vec<ForgetCandidate>,
}

/// Build a forget plan.
pub fn plan_forget(
    candidates: &[ForgetCandidate],
    pattern: &ForgetPattern,
    include_archive: bool,
    reason: Option<&str>,
    erased_at: &str,
) -> Result<ForgetPlan, ForgetError> {
    let matcher = build_matcher(pattern)?;
    let mut remove = Vec::new();
    let mut tombstones = Vec::new();
    let mut skipped_archive = Vec::new();
    for cand in candidates {
        if !matches(&matcher, cand) {
            continue;
        }
        if cand.in_archive && !include_archive {
            skipped_archive.push(cand.clone());
            continue;
        }
        tombstones.push(Tombstone {
            guid: Some(cand.guid.clone()),
            link: None,
            content_hash: cand.content_hash.clone(),
            erased_at: erased_at.to_string(),
            reason: reason.map(|s| s.to_string()),
        });
        remove.push(cand.clone());
    }
    Ok(ForgetPlan {
        remove,
        tombstones,
        skipped_archive,
    })
}

enum Matcher {
    Glob(GlobSet),
    GuidPrefix(String),
    HashPrefix(String),
}

fn build_matcher(p: &ForgetPattern) -> Result<Matcher, ForgetError> {
    match p {
        ForgetPattern::SlugGlob(g) => {
            let mut b = GlobSetBuilder::new();
            b.add(Glob::new(g).map_err(|e| ForgetError::Pattern(e.to_string()))?);
            Ok(Matcher::Glob(b.build().map_err(|e| ForgetError::Pattern(e.to_string()))?))
        }
        ForgetPattern::GuidPrefix(g) => Ok(Matcher::GuidPrefix(g.clone())),
        ForgetPattern::ContentHashPrefix(g) => Ok(Matcher::HashPrefix(g.clone())),
    }
}

fn matches(m: &Matcher, c: &ForgetCandidate) -> bool {
    match m {
        Matcher::Glob(set) => set.is_match(&c.slug),
        Matcher::GuidPrefix(p) => c.guid.starts_with(p.as_str()),
        Matcher::HashPrefix(p) => c
            .content_hash
            .as_deref()
            .map(|h| h.starts_with(p.as_str()))
            .unwrap_or(false),
    }
}

/// Verify a fetched item against an existing tombstone log per
/// REQ-3834. Returns `true` iff at least one signal matches a stored
/// tombstone (T22: refuse re-import).
pub fn is_tombstoned(tombstones: &[Tombstone], guid: Option<&str>, link: Option<&str>, hash: Option<&str>) -> bool {
    for t in tombstones {
        if matches_signal(&t.guid, guid)
            || matches_signal(&t.link, link)
            || matches_signal(&t.content_hash, hash)
        {
            return true;
        }
    }
    false
}

fn matches_signal(stored: &Option<String>, candidate: Option<&str>) -> bool {
    match (stored, candidate) {
        (Some(s), Some(c)) if !s.is_empty() && !c.is_empty() => s == c,
        _ => false,
    }
}

/// Serialise a tombstone log into JSONL.
pub fn tombstones_to_jsonl(tombstones: &[Tombstone]) -> Result<String, serde_json::Error> {
    let mut out = String::new();
    for t in tombstones {
        out.push_str(&serde_json::to_string(t)?);
        out.push('\n');
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ForgetError {
    #[error("invalid pattern: {0}")]
    Pattern(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(slug: &str, in_archive: bool) -> ForgetCandidate {
        ForgetCandidate {
            path: format!(
                ".zetl/feeds/x/{}/{slug}.md",
                if in_archive { "archived" } else { "inbox" }
            ),
            slug: slug.to_string(),
            guid: format!("urn:zetl:{slug}"),
            content_hash: Some(format!("hash-{slug}")),
            in_archive,
        }
    }

    #[test]
    fn slug_glob_matches() {
        let cands = vec![cand("foo/a", false), cand("bar/b", false)];
        let plan = plan_forget(
            &cands,
            &ForgetPattern::SlugGlob("foo/**".to_string()),
            false,
            None,
            "2026-05-08T00:00:00Z",
        )
        .unwrap();
        assert_eq!(plan.remove.len(), 1);
        assert_eq!(plan.remove[0].slug, "foo/a");
    }

    #[test]
    fn guid_prefix_matches() {
        let cands = vec![cand("a", false), cand("b", false)];
        let plan = plan_forget(
            &cands,
            &ForgetPattern::GuidPrefix("urn:zetl:a".to_string()),
            false,
            None,
            "2026-05-08T00:00:00Z",
        )
        .unwrap();
        assert_eq!(plan.remove.len(), 1);
    }

    #[test]
    fn content_hash_prefix_matches() {
        let cands = vec![cand("a", false)];
        let plan = plan_forget(
            &cands,
            &ForgetPattern::ContentHashPrefix("hash-a".to_string()),
            false,
            None,
            "2026-05-08T00:00:00Z",
        )
        .unwrap();
        assert_eq!(plan.remove.len(), 1);
    }

    #[test]
    fn archive_excluded_unless_include_archive_set() {
        let cands = vec![cand("foo", true), cand("bar", false)];
        let plan = plan_forget(
            &cands,
            &ForgetPattern::SlugGlob("**".to_string()),
            false,
            None,
            "2026-05-08T00:00:00Z",
        )
        .unwrap();
        assert_eq!(plan.remove.len(), 1);
        assert_eq!(plan.remove[0].slug, "bar");
        assert_eq!(plan.skipped_archive.len(), 1);

        let plan2 = plan_forget(
            &cands,
            &ForgetPattern::SlugGlob("**".to_string()),
            true,
            None,
            "2026-05-08T00:00:00Z",
        )
        .unwrap();
        assert_eq!(plan2.remove.len(), 2);
        assert!(plan2.skipped_archive.is_empty());
    }

    #[test]
    fn tombstone_records_carry_reason() {
        let cands = vec![cand("a", false)];
        let plan = plan_forget(
            &cands,
            &ForgetPattern::SlugGlob("a".to_string()),
            false,
            Some("operator request"),
            "2026-05-08T00:00:00Z",
        )
        .unwrap();
        assert_eq!(plan.tombstones.len(), 1);
        assert_eq!(plan.tombstones[0].reason.as_deref(), Some("operator request"));
        assert_eq!(plan.tombstones[0].erased_at, "2026-05-08T00:00:00Z");
    }

    #[test]
    fn is_tombstoned_blocks_reimport() {
        let tombs = vec![Tombstone {
            guid: Some("urn:x".to_string()),
            link: None,
            content_hash: Some("hashx".to_string()),
            erased_at: "2026-05-08T00:00:00Z".to_string(),
            reason: None,
        }];
        assert!(is_tombstoned(&tombs, Some("urn:x"), None, None));
        assert!(is_tombstoned(&tombs, None, None, Some("hashx")));
        assert!(!is_tombstoned(&tombs, Some("urn:fresh"), None, Some("hashfresh")));
    }

    #[test]
    fn forget_pattern_detection_heuristic() {
        assert_eq!(
            ForgetPattern::detect("urn:zetl:foo"),
            ForgetPattern::GuidPrefix("urn:zetl:foo".to_string())
        );
        assert_eq!(
            ForgetPattern::detect("deadbeef"),
            ForgetPattern::ContentHashPrefix("deadbeef".to_string())
        );
        assert_eq!(
            ForgetPattern::detect("notes/foo"),
            ForgetPattern::SlugGlob("notes/foo".to_string())
        );
    }

    #[test]
    fn jsonl_round_trip() {
        let tombs = vec![Tombstone {
            guid: Some("g".to_string()),
            link: None,
            content_hash: None,
            erased_at: "2026-05-08T00:00:00Z".to_string(),
            reason: Some("test".to_string()),
        }];
        let jsonl = tombstones_to_jsonl(&tombs).unwrap();
        let line = jsonl.lines().next().unwrap();
        let parsed: Tombstone = serde_json::from_str(line).unwrap();
        assert_eq!(parsed, tombs[0]);
    }
}
