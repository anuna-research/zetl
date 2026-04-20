//! Capability-mode build orchestration (SPEC-034 REQ-3401 / REQ-3403 /
//! REQ-3418).
//!
//! The [`driver`] submodule walks a vault, sanitises each page, path-caps
//! it per cohort, age-encrypts the sanitised HTML to the cohort's
//! recipient set (plus padding), Ed25519-signs the ciphertext, and
//! writes the resulting envelope to `dist/c/<path-cap>/<slug>.html`.
//! Per OBS-3401 it prints a one-line summary to stderr so CI and `make
//! cap-build` can regex-check the counters without parsing any config.

pub mod driver;

pub use driver::{
    run_capability_build, BuildConfig, BuildError, BuildSummary, EncryptedPageStats, PageInput,
    Visibility,
};
