//! Ecosystem adapter layer (SPEC-033).
//!
//! SPEC-033 introduces a first-class notion of a **plugin ecosystem** —
//! Pandoc filters, mdBook preprocessors, remark plugins — that zetl
//! bridges to via the SPEC-032 hook runtime. This module owns:
//!
//! - the [`registry`] that enumerates the ecosystems (REQ-3301 / CON-3301)
//!   with their runtime dependencies, default stages, and feature flags;
//! - the [`adapter`] trait that every concrete adapter implements
//!   (REQ-3302 / CON-3302) — probe, translate, invoke — plus a
//!   parameterised [`adapter::run_conformance`] harness the per-ecosystem
//!   tasks use to satisfy TEST-3302;
//! - a [`adapter::MockEcosystemAdapter`] identity adapter used by tests
//!   and as a placeholder in registry entries until each ecosystem's
//!   real adapter lands (task-pandoc-adapter, task-mdbook-adapter,
//!   task-remark-harness).

pub mod adapter;
pub mod check;
pub mod detection;
pub mod registry;

pub use adapter::{
    default_fixtures, mock_adapter_ctor, run_conformance, CheckOutcome, ConformanceFixture,
    ConformanceReport, Diagnostic, EcosystemAdapter, HookContext, MockEcosystemAdapter,
    PluginManifest, PluginResponse, RuntimeStatus, StageInput, StageOutput,
};
pub use check::{
    assemble_report, run_ecosystem_check, EcosystemCheckEntry, EcosystemCheckReport,
    EcosystemCheckStatus,
};
pub use detection::{
    detect_all_ecosystems, diagnostic_for, enforce_required, format_log_line, install_hint_for,
    parse_version, probe_runtime_dep, upgrade_hint_for, EcosystemDetectionReport, PROBE_TIMEOUT,
};
pub use registry::{all, by_id, Ecosystem, EcosystemEntry, RuntimeDep};
