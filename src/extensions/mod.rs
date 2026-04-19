//! First-party canonical extensions + their shared test infrastructure.
//!
//! The canonical extensions (`callouts`, `tasks`, `admonition`) themselves
//! ship downstream per SPEC-032 REQ-3212 as thin theme stubs backed by
//! ecosystem plugins. This module houses their shared test surface:
//!
//! - [`golden`]: the per-fixture golden-HTML harness (CON-3212). One set
//!   of `input.md` / `expected.html` files per extension plus a pure
//!   comparator drives the CI gate; `cargo xtask update-golden` writes
//!   regenerated expecteds for review.

pub mod golden;
