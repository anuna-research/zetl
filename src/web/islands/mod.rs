//! SPEC-050 — Component Islands & Inter-Island Messaging (build-side).
//!
//! This module is the build-time half of SPEC-050: it recognises island manifest fields
//! (CON-5002), the topic-name grammar (CON-5001, [`topic`]), and the topic value-type
//! language (CON-5005, [`value_type`]); verifies inter-island wiring (REQ-5008/5009,
//! [`wiring`]); and computes the per-page Content-Security-Policy (REQ-5027, [`csp`]).
//! The browser-side runtime (the `window.zetl` bus, capability bridge, and controlled-
//! element renderer) ships as the static asset `src/web/assets/islands.js`.
//!
//! Errors reuse the components' [`ComponentError`](crate::web::components::ComponentError)
//! carrier with SPEC-050 `island-*` codes.

pub mod csp;
pub mod emit;
pub mod manifest;
pub mod theme;
pub mod topic;
pub mod value_type;
pub mod wiring;

/// SPEC-050 errors reuse the component error carrier (code + message).
pub use crate::web::components::ComponentError as IslandError;
/// Result alias for island operations.
pub type IResult<T> = Result<T, IslandError>;
