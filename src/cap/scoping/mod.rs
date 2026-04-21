//! Cohort-to-page scoping (SPEC-034 §4 search & backlinks; §2.1 build
//! pipeline). Pure core: assigns each vault page to zero or more
//! cohorts given the cohort declarations and per-page metadata.
//!
//! Submodules:
//!
//! - [`cohort_index`]: bidirectional (cohort_id ↔ slug) assignment.
//! - [`access_config`]: `[access.search]` / `[access.backlinks]`
//!   TOML surface (REQ-3415); `validate()` rejects global modes at
//!   build start.
//! - [`backlinks`]: cohort-scoped backlink filter that uses the
//!   index to drop cross-cohort sources.

pub mod access_config;
pub mod backlinks;
pub mod cohort_index;
