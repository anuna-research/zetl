//! `[access.search]` / `[access.backlinks]` config surface (SPEC-034
//! REQ-3415).
//!
//! Search and backlinks in capability-mode distribution are scoped to
//! one cohort at a time — a build that enabled global cross-cohort
//! search or cross-cohort backlinks would leak page titles, slugs,
//! and link structure across cohorts that were otherwise protected
//! by per-cohort path-caps and per-cohort ciphertexts.
//!
//! The opt-ins this module accepts:
//!
//! - `[access.search] mode = "off"`   (default; no search UI emitted).
//! - `[access.search] mode = "per-cohort"` (operator opts into a
//!   per-cohort encrypted search index).  The index format itself is
//!   explicitly deferred per BUG-019; v1 builds still refuse to emit
//!   any index bytes — the mode is plumbed so operators can declare
//!   intent and CI can surface the "pending" state, but the feature
//!   only _ships_ once a later task lands the index format.
//! - `[access.backlinks] mode = "scoped"`   (default; backlinks panel
//!   filtered to in-cohort sources).
//!
//! The opt-outs this module rejects at build start (never at a later
//! point where a partial dist tree would already be on disk):
//!
//! - `[access.search] mode = "global"` — global cross-cohort search.
//! - `[access.backlinks] mode = "global"` — global cross-cohort
//!   backlinks.
//!
//! Any other string in either mode field is a parse error so typos
//! cannot silently weaken scoping.

use serde::{Deserialize, Serialize};

/// Combined access-scoping config. Either sub-section is optional
/// in TOML; all fall back to documented defaults when absent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessConfig {
    #[serde(default)]
    pub search: SearchConfig,
    #[serde(default)]
    pub backlinks: BacklinksConfig,
    #[serde(default)]
    pub cache: CacheConfig,
}

/// `[access.search]` table. The `mode` field is the only tunable
/// knob in v1; additional fields (`index_format`, `analyzer`, ...)
/// will be added once BUG-019 resolves the index format question.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchConfig {
    #[serde(default)]
    pub mode: SearchMode,
}

/// `[access.backlinks]` table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BacklinksConfig {
    #[serde(default)]
    pub mode: BacklinksMode,
}

/// Search-UI emission mode. Default `Off` — no search surface is
/// emitted and no index bytes are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SearchMode {
    /// No search UI, no index. REQ-3415 default.
    Off,
    /// One encrypted index per cohort. Index format pending BUG-019;
    /// v1 builds refuse to emit bytes but accept the declaration so
    /// operators can signal intent and CI can report the deferred-
    /// feature state.
    PerCohort,
    /// Global cross-cohort search. Rejected at build start — emitting
    /// a single index that referenced pages from every cohort would
    /// surface page titles and slugs to every reader regardless of
    /// their grants.
    Global,
}

impl Default for SearchMode {
    fn default() -> Self {
        SearchMode::Off
    }
}

/// Backlinks-panel scoping. Default `Scoped` — the backlinks panel on
/// each decrypted page lists only sources that share at least one
/// cohort with the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BacklinksMode {
    /// Filter to in-cohort sources only. REQ-3415 default.
    Scoped,
    /// Show every backlink regardless of cohort. Rejected at build
    /// start — a reader with one cohort's key could otherwise learn
    /// that a page in a _different_ cohort linked into theirs, which
    /// leaks slug + link-structure metadata that path-caps exist to
    /// hide.
    Global,
}

impl Default for BacklinksMode {
    fn default() -> Self {
        BacklinksMode::Scoped
    }
}

/// `[access.cache]` table. Operator-facing knob controlling the
/// `Cache-Control: max-age` emitted on `/c/*` ciphertext responses
/// via the deploy-headers recipes (SPEC-034 REQ-3407 / CON-3406).
///
/// Hard bounds `[60, 3600]` (NFR-3409 revocation-latency envelope);
/// the `validate` method rejects anything outside so a misconfigured
/// value short-circuits the build before any recipe is written.
///
/// The immutable cache on `/assets/shim.js` (`max-age=31536000`) and
/// the `Clear-Site-Data` header on `/enroll.html` + `/logout` are
/// NOT operator-tunable — both are fixed by spec.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheConfig {
    /// Seconds. Default 300 (5 min). Bounds [60, 3600].
    #[serde(default = "default_cap_max_age")]
    pub max_age: u32,
}

/// Default `/c/*` max-age in seconds (5 minutes).
pub const DEFAULT_CAP_MAX_AGE: u32 = 300;
/// Lower bound (NFR-3409 revocation-latency floor).
pub const MIN_CAP_MAX_AGE: u32 = 60;
/// Upper bound (NFR-3409 revocation-latency ceiling: 1 hour).
pub const MAX_CAP_MAX_AGE: u32 = 3600;

fn default_cap_max_age() -> u32 {
    DEFAULT_CAP_MAX_AGE
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_age: DEFAULT_CAP_MAX_AGE,
        }
    }
}

/// Validation error surfaced at the build-driver boundary so the
/// whole build short-circuits before any ciphertext is written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AccessConfigError {
    #[error(
        "[access.search] mode = \"global\" is not supported in capability-mode builds — global \
         cross-cohort search would leak page titles/slugs across cohorts; set mode = \"off\" \
         (default) or mode = \"per-cohort\" (SPEC-034 REQ-3415)"
    )]
    GlobalSearchRejected,
    #[error(
        "[access.backlinks] mode = \"global\" is not supported in capability-mode builds — \
         global cross-cohort backlinks would reveal that a page in one cohort links into \
         another; set mode = \"scoped\" (default) (SPEC-034 REQ-3415)"
    )]
    GlobalBacklinksRejected,
    #[error(
        "[access.cache] max_age = {got} is outside the permitted bounds [{min}, {max}] \
         (SPEC-034 REQ-3407 / NFR-3409 — revocation latency envelope)",
        min = MIN_CAP_MAX_AGE,
        max = MAX_CAP_MAX_AGE,
    )]
    CacheMaxAgeOutOfBounds { got: u32 },
}

impl AccessConfig {
    /// Check the parsed config for REQ-3415 rejection rules. Pure —
    /// callers must invoke this before consuming the config in the
    /// build driver.
    pub fn validate(&self) -> Result<(), AccessConfigError> {
        if matches!(self.search.mode, SearchMode::Global) {
            return Err(AccessConfigError::GlobalSearchRejected);
        }
        if matches!(self.backlinks.mode, BacklinksMode::Global) {
            return Err(AccessConfigError::GlobalBacklinksRejected);
        }
        if self.cache.max_age < MIN_CAP_MAX_AGE || self.cache.max_age > MAX_CAP_MAX_AGE {
            return Err(AccessConfigError::CacheMaxAgeOutOfBounds {
                got: self.cache.max_age,
            });
        }
        Ok(())
    }

    /// Whether the build should emit any search-UI / search-index
    /// artefacts. False for `Off`; **also** false for `PerCohort`
    /// in v1 because the index format is deferred (BUG-019). The
    /// public function name reads as intent, not as "is the knob
    /// flipped" — when BUG-019 lands this returns `true` for
    /// `PerCohort` and the driver starts emitting bytes.
    pub fn search_ui_enabled(&self) -> bool {
        // BUG-019: per-cohort index format is deferred. Keep the
        // operator's opt-in recorded (so `validate()` still rejects
        // global) but don't emit anything yet.
        false
    }

    /// Whether backlinks should be rendered at all. Always `true` in
    /// capability-mode — the panel is scoped not suppressed. Kept as
    /// an accessor so future `[access.backlinks] mode = "off"` can
    /// flip it without touching call sites.
    pub fn backlinks_enabled(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_off_and_scoped() {
        let cfg = AccessConfig::default();
        assert_eq!(cfg.search.mode, SearchMode::Off);
        assert_eq!(cfg.backlinks.mode, BacklinksMode::Scoped);
        assert!(cfg.validate().is_ok());
        assert!(!cfg.search_ui_enabled());
        assert!(cfg.backlinks_enabled());
    }

    #[test]
    fn per_cohort_search_validates_but_does_not_emit_in_v1() {
        let cfg = AccessConfig {
            search: SearchConfig {
                mode: SearchMode::PerCohort,
            },
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());
        // BUG-019: index format deferred, so v1 builds still do not
        // emit a search UI even when the operator opts in.
        assert!(!cfg.search_ui_enabled());
    }

    #[test]
    fn global_search_is_rejected() {
        let cfg = AccessConfig {
            search: SearchConfig {
                mode: SearchMode::Global,
            },
            ..Default::default()
        };
        assert_eq!(
            cfg.validate().unwrap_err(),
            AccessConfigError::GlobalSearchRejected
        );
    }

    #[test]
    fn global_backlinks_is_rejected() {
        let cfg = AccessConfig {
            backlinks: BacklinksConfig {
                mode: BacklinksMode::Global,
            },
            ..Default::default()
        };
        assert_eq!(
            cfg.validate().unwrap_err(),
            AccessConfigError::GlobalBacklinksRejected
        );
    }

    #[test]
    fn toml_round_trip_defaults() {
        let parsed: AccessConfig = toml::from_str("").unwrap();
        assert_eq!(parsed, AccessConfig::default());
    }

    #[test]
    fn toml_parses_per_cohort() {
        let src = r#"
            [search]
            mode = "per-cohort"
        "#;
        let parsed: AccessConfig = toml::from_str(src).unwrap();
        assert_eq!(parsed.search.mode, SearchMode::PerCohort);
        assert_eq!(parsed.backlinks.mode, BacklinksMode::Scoped);
    }

    #[test]
    fn toml_parses_global_and_validate_rejects() {
        let src = r#"
            [search]
            mode = "global"
        "#;
        let parsed: AccessConfig = toml::from_str(src).unwrap();
        assert_eq!(parsed.search.mode, SearchMode::Global);
        assert!(parsed.validate().is_err());
    }

    #[test]
    fn unknown_mode_is_a_parse_error() {
        // Typos must not silently become `Off` — they must be loud so
        // an operator who meant `per-cohort` doesn't accidentally
        // ship without search.
        let src = r#"
            [search]
            mode = "cohort-per"
        "#;
        let err = toml::from_str::<AccessConfig>(src).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("unknown variant")
                || msg.contains("did not match any variant")
                || msg.contains("unknown"),
            "expected serde unknown-variant error, got {msg:?}"
        );
    }

    #[test]
    fn unknown_field_is_a_parse_error() {
        // `deny_unknown_fields` protects against a future `mode`
        // rename from being accidentally accepted under the old key.
        let src = r#"
            [search]
            mode = "off"
            mode2 = "per-cohort"
        "#;
        assert!(toml::from_str::<AccessConfig>(src).is_err());
    }

    #[test]
    fn cache_defaults_to_300s() {
        let cfg = AccessConfig::default();
        assert_eq!(cfg.cache.max_age, DEFAULT_CAP_MAX_AGE);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn cache_toml_parses_override() {
        let src = r#"
            [cache]
            max_age = 600
        "#;
        let parsed: AccessConfig = toml::from_str(src).unwrap();
        assert_eq!(parsed.cache.max_age, 600);
        assert!(parsed.validate().is_ok());
    }

    #[test]
    fn cache_max_age_below_floor_rejected() {
        let cfg = AccessConfig {
            cache: CacheConfig { max_age: 30 },
            ..Default::default()
        };
        assert_eq!(
            cfg.validate().unwrap_err(),
            AccessConfigError::CacheMaxAgeOutOfBounds { got: 30 }
        );
    }

    #[test]
    fn cache_max_age_above_ceiling_rejected() {
        let cfg = AccessConfig {
            cache: CacheConfig { max_age: 7200 },
            ..Default::default()
        };
        assert_eq!(
            cfg.validate().unwrap_err(),
            AccessConfigError::CacheMaxAgeOutOfBounds { got: 7200 }
        );
    }

    #[test]
    fn cache_max_age_boundaries_accepted() {
        for value in [MIN_CAP_MAX_AGE, MAX_CAP_MAX_AGE] {
            let cfg = AccessConfig {
                cache: CacheConfig { max_age: value },
                ..Default::default()
            };
            assert!(
                cfg.validate().is_ok(),
                "boundary {value} should validate"
            );
        }
    }

    #[test]
    fn cache_unknown_field_rejected() {
        let src = r#"
            [cache]
            max_age = 300
            ttl = 999
        "#;
        assert!(toml::from_str::<AccessConfig>(src).is_err());
    }

    #[test]
    fn backlinks_scoped_opt_in_is_a_noop_but_accepted() {
        let src = r#"
            [backlinks]
            mode = "scoped"
        "#;
        let parsed: AccessConfig = toml::from_str(src).unwrap();
        assert!(parsed.validate().is_ok());
        assert_eq!(parsed.backlinks.mode, BacklinksMode::Scoped);
    }
}
