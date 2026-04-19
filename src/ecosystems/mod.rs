//! Ecosystem adapter layer (SPEC-033).
//!
//! SPEC-033 introduces a first-class notion of a **plugin ecosystem** —
//! Pandoc filters, mdBook preprocessors, remark plugins — that zetl
//! bridges to via the SPEC-032 hook runtime. This module owns the
//! registry that enumerates those ecosystems and, as downstream tasks
//! land, will also own the per-ecosystem [`EcosystemAdapter`]
//! (CON-3302) implementations.
//!
//! Today this crate scopes to the registry alone (task-eco-registry);
//! adapter traits and concrete adapters arrive with task-adapter-trait
//! and the per-ecosystem tasks (task-eco-pandoc, task-eco-mdbook,
//! task-eco-remark).

pub mod registry;

pub use registry::{Ecosystem, EcosystemEntry, RuntimeDep, all, by_id};
