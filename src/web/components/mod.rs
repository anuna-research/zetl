//! SPEC-048 — Template Components & Templated Static Pages (v1 core).
//!
//! A [`Component`] is a directory `components/<name>/` holding a minijinja-fragment
//! template (`<name>.html`), a manifest (`<name>.toml`, [`manifest`]), and an optional
//! stylesheet (`<name>.css`). Components compile to minijinja **macros** — the
//! `{% component %}`/`{% slot %}` surface is lowered to native `{% call %}` +
//! `{% set %}` capture before parse ([`lower`]); minijinja gains no custom tag
//! (ADR-4803).
//!
//! This module is pure/library code: parsing, validation, resolution, lowering, the
//! token compiler, CSS collection, transclusion target handling, and nesting bounds.
//! Wiring into the render engine and the build pipeline lives in `engine.rs` /
//! `build.rs`.

pub mod css;
#[cfg(feature = "content-components")]
pub mod directive;
pub mod lower;
pub mod manifest;
pub mod nesting;
pub mod resolve;
#[cfg(feature = "content-components")]
pub mod sanitize;
pub mod tokens;
pub mod transclude;
pub mod validate;

use std::fmt;

/// A recognised SPEC-048 error carrying one of the spec's stable error codes
/// (`component-malformed`, `component-not-found`, `tokens-value-unsafe`, …). The code
/// is what observability (OBS-4803) and `--strict` builds key on; the message is for
/// humans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentError {
    /// Stable kebab-case error code from SPEC-048 (e.g. `component-malformed`).
    pub code: &'static str,
    /// Human-readable detail.
    pub message: String,
}

impl ComponentError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for ComponentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for ComponentError {}

/// Result alias for component operations.
pub type CResult<T> = Result<T, ComponentError>;
