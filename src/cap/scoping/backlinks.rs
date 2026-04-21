//! Cohort-scoped backlinks filtering (SPEC-034 REQ-3415).
//!
//! Given the set of incoming links to a page (the vault-wide
//! backlinks the core `graph::LinkGraph` would surface in non-
//! capability mode) plus the cohort assignment for every slug, this
//! module filters to only those sources that share at least one
//! cohort with the target.
//!
//! Pure core: no I/O, no clock, and the cohort assignment is threaded
//! in by the caller ([`CohortIndex`] from `cohort_index`).  The
//! caller is responsible for re-running this filter for each
//! (cohort, target-slug) pair it encrypts, because in capability
//! mode the _same_ target slug renders with a different backlinks
//! panel per cohort.

use std::collections::BTreeSet;

use super::cohort_index::CohortIndex;

/// One raw backlink as the caller sees it before scoping. The struct
/// is intentionally minimal — whatever extra per-edge metadata the
/// non-capability renderer carries (line numbers, aliases, embed
/// flag, …) is opaque to this filter.  Keep the identifying handle
/// (`source_slug`) as the sole field this module reads; attach the
/// rest via the generic payload so call sites can round-trip their
/// existing types without a conversion step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawBacklink<T> {
    pub source_slug: String,
    pub payload: T,
}

/// Filter `raw` down to the in-cohort subset per REQ-3415.
///
/// A backlink survives iff the target page and the source page share
/// at least one cohort in `index`.  Targets with no cohort (a page
/// that no cohort covers) produce an empty result — the caller
/// should not be rendering that page in capability mode anyway, but
/// the filter is defensive.
///
/// Order of the output mirrors the order of the input so callers
/// that already rank by line number / source slug see stable output.
/// No deduplication — a source that links twice appears twice, the
/// same as the non-capability renderer.
pub fn scope_backlinks_for_target<T: Clone>(
    target_slug: &str,
    raw: &[RawBacklink<T>],
    index: &CohortIndex,
) -> Vec<RawBacklink<T>> {
    let target_cohorts: BTreeSet<&str> = index
        .cohorts_of(target_slug)
        .iter()
        .map(String::as_str)
        .collect();
    if target_cohorts.is_empty() {
        return Vec::new();
    }
    raw.iter()
        .filter(|b| {
            let src_cohorts = index.cohorts_of(&b.source_slug);
            src_cohorts
                .iter()
                .any(|c| target_cohorts.contains(c.as_str()))
        })
        .cloned()
        .collect()
}

/// Cohort-scoped backlinks keyed by cohort. The same target slug may
/// render in multiple cohorts; each cohort rendering sees only the
/// backlinks whose source is _also_ in that cohort, even if another
/// cohort's rendering of the same target would include more.
///
/// This is the shape the build driver consumes: for a page P in
/// cohorts {A, B}, the returned map has two entries keyed by `"A"`
/// and `"B"`.
pub fn scope_backlinks_per_cohort<T: Clone>(
    target_slug: &str,
    raw: &[RawBacklink<T>],
    index: &CohortIndex,
) -> std::collections::BTreeMap<String, Vec<RawBacklink<T>>> {
    let mut out: std::collections::BTreeMap<String, Vec<RawBacklink<T>>> =
        std::collections::BTreeMap::new();
    for cohort_id in index.cohorts_of(target_slug) {
        let filtered: Vec<RawBacklink<T>> = raw
            .iter()
            .filter(|b| {
                index
                    .cohorts_of(&b.source_slug)
                    .iter()
                    .any(|c| c == cohort_id)
            })
            .cloned()
            .collect();
        out.insert(cohort_id.clone(), filtered);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::cap::scoping::cohort_index::{CohortScope, PageRef};

    fn cohort(id: &str, glob: Option<&str>) -> CohortScope {
        CohortScope {
            id: id.to_string(),
            pages_glob: glob.map(String::from),
        }
    }

    fn page(slug: &str, cohorts: &[&str]) -> PageRef {
        PageRef {
            slug: slug.to_string(),
            explicit_cohorts: cohorts.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn bl(source: &str) -> RawBacklink<()> {
        RawBacklink {
            source_slug: source.to_string(),
            payload: (),
        }
    }

    #[test]
    fn in_cohort_sources_survive_cross_cohort_sources_drop() {
        // "readme" lives in cohort "eng"; "ops/runbook" lives in
        // cohort "ops".  Both link to "shared".  "shared" is
        // declared as member of _only_ "eng" via frontmatter.
        // Filter should keep the readme→shared backlink and drop
        // the ops/runbook→shared one.
        let c = vec![cohort("eng", None), cohort("ops", None)];
        let p = vec![
            page("readme", &["eng"]),
            page("ops/runbook", &["ops"]),
            page("shared", &["eng"]),
        ];
        let ix = CohortIndex::build(&c, &p).unwrap();
        let raw = vec![bl("readme"), bl("ops/runbook")];
        let scoped = scope_backlinks_for_target("shared", &raw, &ix);
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].source_slug, "readme");
    }

    #[test]
    fn target_in_multiple_cohorts_accepts_source_from_any() {
        // "shared" is in both "eng" and "ops".  A backlink from
        // anything in either cohort is retained.
        let c = vec![cohort("eng", None), cohort("ops", None)];
        let p = vec![
            page("a", &["eng"]),
            page("b", &["ops"]),
            page("shared", &["eng", "ops"]),
        ];
        let ix = CohortIndex::build(&c, &p).unwrap();
        let raw = vec![bl("a"), bl("b")];
        let scoped = scope_backlinks_for_target("shared", &raw, &ix);
        assert_eq!(scoped.len(), 2);
    }

    #[test]
    fn source_outside_all_cohorts_drops() {
        // "outsider" matches no cohort (no frontmatter, no glob
        // hit).  Its backlinks never appear in any scoped render.
        let c = vec![cohort("eng", Some("eng/**"))];
        let p = vec![page("eng/home", &[]), page("outsider", &[])];
        let ix = CohortIndex::build(&c, &p).unwrap();
        let raw = vec![bl("outsider"), bl("eng/home")];
        let scoped = scope_backlinks_for_target("eng/home", &raw, &ix);
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].source_slug, "eng/home");
    }

    #[test]
    fn target_outside_all_cohorts_produces_empty() {
        let c = vec![cohort("eng", Some("eng/**"))];
        let p = vec![page("eng/home", &[]), page("orphan", &[])];
        let ix = CohortIndex::build(&c, &p).unwrap();
        let raw = vec![bl("eng/home")];
        let scoped = scope_backlinks_for_target("orphan", &raw, &ix);
        assert!(scoped.is_empty());
    }

    #[test]
    fn per_cohort_returns_one_entry_per_target_cohort() {
        // "shared" is in {eng, ops}.  Readme is in eng; runbook is
        // in ops.  The per-cohort render of "shared" for eng shows
        // only readme; for ops shows only runbook — even though
        // the vault-global backlinks list is the same.
        let c = vec![cohort("eng", None), cohort("ops", None)];
        let p = vec![
            page("readme", &["eng"]),
            page("ops/runbook", &["ops"]),
            page("shared", &["eng", "ops"]),
        ];
        let ix = CohortIndex::build(&c, &p).unwrap();
        let raw = vec![bl("readme"), bl("ops/runbook")];
        let per = scope_backlinks_per_cohort("shared", &raw, &ix);
        assert_eq!(per.len(), 2);
        assert_eq!(per.get("eng").unwrap().len(), 1);
        assert_eq!(per.get("eng").unwrap()[0].source_slug, "readme");
        assert_eq!(per.get("ops").unwrap().len(), 1);
        assert_eq!(per.get("ops").unwrap()[0].source_slug, "ops/runbook");
    }

    #[test]
    fn input_order_preserved_no_dedup() {
        let c = vec![cohort("all", None)];
        let p = vec![page("a", &[]), page("b", &[])];
        let ix = CohortIndex::build(&c, &p).unwrap();
        // `a` backlinks twice (e.g. two wikilinks in one source
        // document); both should come through, in order.
        let raw = vec![bl("a"), bl("a"), bl("b")];
        let scoped = scope_backlinks_for_target("a", &raw, &ix);
        assert_eq!(scoped.len(), 3);
        assert_eq!(scoped[0].source_slug, "a");
        assert_eq!(scoped[1].source_slug, "a");
        assert_eq!(scoped[2].source_slug, "b");
    }

    #[test]
    fn payload_round_trips_unchanged() {
        // The filter must not mutate whatever the caller hangs off
        // a backlink (line numbers, aliases, embed flags).
        let c = vec![cohort("all", None)];
        let p = vec![page("a", &[]), page("b", &[])];
        let ix = CohortIndex::build(&c, &p).unwrap();
        let raw = vec![
            RawBacklink {
                source_slug: "a".into(),
                payload: (42u32, "alias".to_string()),
            },
            RawBacklink {
                source_slug: "b".into(),
                payload: (7u32, "other".to_string()),
            },
        ];
        let scoped = scope_backlinks_for_target("b", &raw, &ix);
        assert_eq!(scoped.len(), 2);
        assert_eq!(scoped[0].payload, (42u32, "alias".to_string()));
        assert_eq!(scoped[1].payload, (7u32, "other".to_string()));
    }
}
