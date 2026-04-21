//! Cohort-to-pages assignment (SPEC-034 §2.1, REQ-3403).
//!
//! Given:
//!
//! 1. The cohort declarations from `recipients.toml` — each cohort
//!    may carry an optional `pages` glob that scopes which vault
//!    pages the cohort encrypts.
//! 2. A flat list of vault pages (each a slug plus any
//!    frontmatter-declared cohort memberships).
//!
//! Compute the bidirectional mapping:
//!
//! - `pages_in(cohort_id)` → the ordered list of page slugs that
//!   cohort will encrypt.
//! - `cohorts_of(slug)` → the ordered list of cohort ids that
//!   encrypt a given page.
//!
//! Assignment rules:
//!
//! - A page lists explicit cohorts in frontmatter → it belongs to
//!   **exactly** that set, regardless of cohort glob patterns.
//!   Explicit wins.
//! - A page lists no cohorts → it belongs to every cohort whose
//!   `pages` glob matches the slug (or whose glob is `None` / `*`,
//!   which match every page).
//! - A page that matches no cohort (no frontmatter + no glob hit) is
//!   valid — it just won't appear in any encrypted output. The index
//!   does not error; scoping decisions that render a page unreachable
//!   are the operator's responsibility.
//!
//! This module is pure: no I/O, no globals. The caller compiles the
//! globs (via `globset::GlobSet`) once at build start and feeds them
//! in.

use std::collections::{BTreeMap, BTreeSet};

use globset::{Glob, GlobSet, GlobSetBuilder};

/// One cohort as it appears to the scoping layer. Glob compilation
/// lives outside this module so callers can reuse a compiled
/// `GlobSet` across many pages without paying re-compile cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CohortScope {
    pub id: String,
    /// If `None`, the cohort matches every page that does not opt in
    /// explicitly elsewhere. If `Some`, only matching pages are in.
    pub pages_glob: Option<String>,
}

/// One vault page as it appears to the scoping layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRef {
    pub slug: String,
    /// Cohorts explicitly named by the page's frontmatter. Empty
    /// means "no explicit declaration; fall back to glob matching."
    pub explicit_cohorts: Vec<String>,
}

/// Errors raised while building a cohort index.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IndexError {
    #[error("duplicate cohort id {0:?} in declarations")]
    DuplicateCohortId(String),
    #[error("duplicate page slug {0:?}")]
    DuplicateSlug(String),
    #[error("page {slug:?} names unknown cohort {cohort:?} in frontmatter")]
    UnknownCohort { slug: String, cohort: String },
    #[error("cohort {cohort_id:?} has malformed glob {glob:?}: {err}")]
    BadGlob {
        cohort_id: String,
        glob: String,
        err: String,
    },
}

/// The bidirectional assignment. Stored as `BTreeMap` so iteration
/// order is stable (deterministic build output — NFR-3402 / REQ-3420).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CohortIndex {
    cohort_to_pages: BTreeMap<String, Vec<String>>,
    page_to_cohorts: BTreeMap<String, Vec<String>>,
}

impl CohortIndex {
    /// Build the index from a cohort-declaration list + page list.
    pub fn build(cohorts: &[CohortScope], pages: &[PageRef]) -> Result<Self, IndexError> {
        // 1. Unique cohort ids.
        let mut seen_cohorts: BTreeSet<&str> = BTreeSet::new();
        for c in cohorts {
            if !seen_cohorts.insert(c.id.as_str()) {
                return Err(IndexError::DuplicateCohortId(c.id.clone()));
            }
        }

        // 2. Unique page slugs.
        let mut seen_slugs: BTreeSet<&str> = BTreeSet::new();
        for p in pages {
            if !seen_slugs.insert(p.slug.as_str()) {
                return Err(IndexError::DuplicateSlug(p.slug.clone()));
            }
        }

        // 3. Explicit-frontmatter cohorts must exist in the cohort
        // set; otherwise we silently drop pages, which is the exact
        // footgun this validator is meant to catch.
        for p in pages {
            for want in &p.explicit_cohorts {
                if !seen_cohorts.contains(want.as_str()) {
                    return Err(IndexError::UnknownCohort {
                        slug: p.slug.clone(),
                        cohort: want.clone(),
                    });
                }
            }
        }

        // 4. Compile one GlobSet per cohort (NB: each cohort has at
        // most one pattern, but `GlobSet` gives us standard
        // glob-semantics matching without hand-rolling anything).
        // Cohorts with `pages_glob = None` have no GlobSet; we treat
        // them as "match every non-explicit page."
        let mut cohort_glob: BTreeMap<&str, Option<GlobSet>> = BTreeMap::new();
        for c in cohorts {
            let set = match &c.pages_glob {
                None => None,
                Some(pat) => Some(compile_glob(&c.id, pat)?),
            };
            cohort_glob.insert(c.id.as_str(), set);
        }

        // 5. Assign.
        let mut cohort_to_pages: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut page_to_cohorts: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for c in cohorts {
            cohort_to_pages.entry(c.id.clone()).or_default();
        }
        for p in pages {
            page_to_cohorts.entry(p.slug.clone()).or_default();

            if !p.explicit_cohorts.is_empty() {
                // Explicit declaration wins — every match, no globs.
                for c in &p.explicit_cohorts {
                    push_unique(cohort_to_pages.get_mut(c).unwrap(), &p.slug);
                    push_unique(page_to_cohorts.get_mut(&p.slug).unwrap(), c);
                }
                continue;
            }

            // Glob-based assignment — cohort-declaration order drives
            // the output order so the index is fully deterministic.
            for c in cohorts {
                let hit = match cohort_glob.get(c.id.as_str()) {
                    Some(None) => true,
                    Some(Some(gs)) => gs.is_match(&p.slug),
                    None => unreachable!("compiled every cohort above"),
                };
                if hit {
                    push_unique(cohort_to_pages.get_mut(&c.id).unwrap(), &p.slug);
                    push_unique(page_to_cohorts.get_mut(&p.slug).unwrap(), &c.id);
                }
            }
        }

        Ok(Self {
            cohort_to_pages,
            page_to_cohorts,
        })
    }

    /// Slugs encrypted by `cohort_id`. Empty slice if the cohort has
    /// no pages (valid) or does not exist (also valid — caller's
    /// lookup is best-effort).
    pub fn pages_in(&self, cohort_id: &str) -> &[String] {
        self.cohort_to_pages
            .get(cohort_id)
            .map_or(&[], |v| v.as_slice())
    }

    /// Cohort ids that will encrypt `slug`. Empty slice if the slug
    /// is unknown or the page is outside every cohort's scope.
    pub fn cohorts_of(&self, slug: &str) -> &[String] {
        self.page_to_cohorts.get(slug).map_or(&[], |v| v.as_slice())
    }

    /// Iterate all cohort → slugs mappings in cohort-id-sorted order.
    pub fn iter_cohorts(&self) -> impl Iterator<Item = (&str, &[String])> {
        self.cohort_to_pages
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_slice()))
    }
}

fn push_unique(v: &mut Vec<String>, s: &str) {
    if !v.iter().any(|x| x == s) {
        v.push(s.to_string());
    }
}

fn compile_glob(cohort_id: &str, pattern: &str) -> Result<GlobSet, IndexError> {
    let mut b = GlobSetBuilder::new();
    let g = Glob::new(pattern).map_err(|e| IndexError::BadGlob {
        cohort_id: cohort_id.to_string(),
        glob: pattern.to_string(),
        err: e.to_string(),
    })?;
    b.add(g);
    b.build().map_err(|e| IndexError::BadGlob {
        cohort_id: cohort_id.to_string(),
        glob: pattern.to_string(),
        err: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn default_cohort_matches_all_non_explicit_pages() {
        let c = vec![cohort("all", None)];
        let p = vec![page("a", &[]), page("b/nested", &[])];
        let ix = CohortIndex::build(&c, &p).unwrap();
        assert_eq!(
            ix.pages_in("all"),
            &["a".to_string(), "b/nested".to_string()]
        );
        assert_eq!(ix.cohorts_of("a"), &["all".to_string()]);
    }

    #[test]
    fn glob_filters_pages() {
        let c = vec![cohort("eng", Some("eng/**")), cohort("ops", Some("ops/**"))];
        let p = vec![
            page("eng/intro", &[]),
            page("ops/runbook", &[]),
            page("readme", &[]),
        ];
        let ix = CohortIndex::build(&c, &p).unwrap();
        assert_eq!(ix.pages_in("eng"), &["eng/intro".to_string()]);
        assert_eq!(ix.pages_in("ops"), &["ops/runbook".to_string()]);
        assert_eq!(ix.cohorts_of("readme"), &[] as &[String]);
    }

    #[test]
    fn explicit_frontmatter_overrides_glob() {
        // Even though the "eng" glob matches "shared/page", the
        // explicit declaration names "ops" — "ops" wins, "eng" does
        // not appear.
        let c = vec![cohort("eng", Some("**")), cohort("ops", None)];
        let p = vec![page("shared/page", &["ops"])];
        let ix = CohortIndex::build(&c, &p).unwrap();
        assert_eq!(ix.pages_in("ops"), &["shared/page".to_string()]);
        assert_eq!(ix.pages_in("eng"), &[] as &[String]);
    }

    #[test]
    fn explicit_can_name_multiple_cohorts() {
        let c = vec![cohort("eng", None), cohort("ops", None)];
        let p = vec![page("page", &["eng", "ops"])];
        let ix = CohortIndex::build(&c, &p).unwrap();
        assert_eq!(
            ix.cohorts_of("page"),
            &["eng".to_string(), "ops".to_string()]
        );
    }

    #[test]
    fn duplicate_cohort_fails() {
        let c = vec![cohort("eng", None), cohort("eng", None)];
        let p: Vec<PageRef> = vec![];
        assert!(matches!(
            CohortIndex::build(&c, &p),
            Err(IndexError::DuplicateCohortId(_))
        ));
    }

    #[test]
    fn duplicate_slug_fails() {
        let c = vec![cohort("eng", None)];
        let p = vec![page("a", &[]), page("a", &[])];
        assert!(matches!(
            CohortIndex::build(&c, &p),
            Err(IndexError::DuplicateSlug(_))
        ));
    }

    #[test]
    fn unknown_explicit_cohort_fails() {
        let c = vec![cohort("eng", None)];
        let p = vec![page("a", &["ghost"])];
        assert!(matches!(
            CohortIndex::build(&c, &p),
            Err(IndexError::UnknownCohort { .. })
        ));
    }

    #[test]
    fn star_glob_matches_all_pages() {
        let c = vec![cohort("all", Some("**"))];
        let p = vec![page("a", &[]), page("deep/nested/page", &[])];
        let ix = CohortIndex::build(&c, &p).unwrap();
        assert_eq!(ix.pages_in("all").len(), 2);
    }

    #[test]
    fn bad_glob_fails() {
        let c = vec![cohort("eng", Some("[invalid"))];
        let p: Vec<PageRef> = vec![];
        assert!(matches!(
            CohortIndex::build(&c, &p),
            Err(IndexError::BadGlob { .. })
        ));
    }

    #[test]
    fn cohort_declaration_order_drives_output_order() {
        // When a page is hit by two cohorts' globs, the output
        // cohort-list honours the declaration order so downstream
        // build steps (age encrypt w/ recipients = union) are
        // deterministic. REQ-3420.
        let c = vec![cohort("b", None), cohort("a", None)];
        let p = vec![page("x", &[])];
        let ix = CohortIndex::build(&c, &p).unwrap();
        assert_eq!(ix.cohorts_of("x"), &["b".to_string(), "a".to_string()]);
    }

    #[test]
    fn empty_inputs_produce_empty_index() {
        let ix = CohortIndex::build(&[], &[]).unwrap();
        assert!(ix.iter_cohorts().next().is_none());
    }

    #[test]
    fn iter_cohorts_is_sorted_by_id() {
        let c = vec![cohort("z", None), cohort("a", None), cohort("m", None)];
        let p = vec![page("x", &[])];
        let ix = CohortIndex::build(&c, &p).unwrap();
        let ids: Vec<&str> = ix.iter_cohorts().map(|(id, _)| id).collect();
        assert_eq!(ids, vec!["a", "m", "z"]);
    }
}
