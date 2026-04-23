//! `ztl ecosystem check` aggregator (SPEC-033 REQ-3310 / CON-3310).
//!
//! Probes each registered ecosystem's runtime ([`detection::detect_all_ecosystems`]),
//! counts how many of the vault's composed hooks declare each ecosystem, and
//! scans for reachable plugins per ecosystem so the user sees, at a glance,
//! which runtimes are present and which adapters are wired up for this vault.
//!
//! ## Scope
//!
//! This module is the *pure* aggregator — it:
//!
//! - invokes [`crate::ecosystems::detection::detect_all_ecosystems`] once
//!   (NFR-3301 session-scoped cache);
//! - walks an input list of [`ComposedHook`]s, grouping by the manifest's
//!   top-level `ecosystem = "..."` field;
//! - scans the host's `PATH` for mdbook preprocessors (binaries matching
//!   `mdbook-*`) since that's how mdBook itself discovers them;
//! - lists remark plugins resolvable from `./node_modules` when the
//!   vault has a Node-side package tree;
//! - for Pandoc: enumerates the `executable` paths declared by
//!   configured filter hooks — the adapter side has no host-wide
//!   "installed filters" concept.
//!
//! Rendering (table and JSON) is owned by the CLI dispatch site; this
//! module hands back a serialisable [`EcosystemCheckReport`] so tests and
//! callers can assert on structured data.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ecosystems::adapter::RuntimeStatus;
use crate::ecosystems::detection::{detect_all_ecosystems, EcosystemDetectionReport};
use crate::ecosystems::registry::{EcosystemEntry, ECOSYSTEMS};
use crate::hooks::composition::ComposedHook;

/// Compact per-ecosystem status tag surfaced in table + JSON output.
///
/// SPEC-033 REQ-3310 mandates three states: `detected`, `missing`,
/// `wrong-version`. `probe-failed` is folded into `missing` for the
/// user-facing report (the detail still lives in
/// [`EcosystemCheckEntry::hint`]), since the remediation — install a
/// working runtime — is the same for both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EcosystemCheckStatus {
    Detected,
    Missing,
    WrongVersion,
}

impl EcosystemCheckStatus {
    /// Fold a [`RuntimeStatus`] into the three CON-3310 axes. `Missing`
    /// and `ProbeFailed` both map to [`Self::Missing`] because the
    /// remediation (install or repair the runtime) is the same.
    pub fn from_runtime(status: &RuntimeStatus) -> Self {
        match status {
            RuntimeStatus::Available { .. } => EcosystemCheckStatus::Detected,
            RuntimeStatus::OutdatedVersion { .. } => EcosystemCheckStatus::WrongVersion,
            RuntimeStatus::Missing { .. } | RuntimeStatus::ProbeFailed { .. } => {
                EcosystemCheckStatus::Missing
            }
        }
    }

    /// Human-readable tag for the table's `STATUS` column.
    pub fn as_str(self) -> &'static str {
        match self {
            EcosystemCheckStatus::Detected => "detected",
            EcosystemCheckStatus::Missing => "missing",
            EcosystemCheckStatus::WrongVersion => "wrong-version",
        }
    }
}

/// One row of the `ztl ecosystem check` report — matches CON-3310's
/// JSON shape (`id`, `status`, `version`, `configured`, `available_plugins`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EcosystemCheckEntry {
    /// Registry id — `"pandoc"`, `"mdbook"`, or `"remark"`.
    pub id: String,
    /// Detection outcome, folded from [`RuntimeStatus`] per CON-3310.
    pub status: EcosystemCheckStatus,
    /// Version string as the runtime itself reported it. `None` when the
    /// runtime is missing or the probe failed before a version could be
    /// read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Resolved binary path (if the probe found one). Informational;
    /// empty for intrinsic or missing runtimes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable: Option<PathBuf>,
    /// Count of composed hooks in the vault whose manifest declared this
    /// ecosystem via `ecosystem = "..."`. Zero when the vault hasn't
    /// wired any hooks against this ecosystem yet.
    pub configured: usize,
    /// Plugins reachable on the host. Semantics per ecosystem:
    ///
    /// - **mdbook**: binaries named `mdbook-*` found on `PATH`.
    /// - **pandoc**: the set of configured filter executables that exist
    ///   on disk — pandoc has no host-wide "installed filter" concept,
    ///   so reachable ≡ resolvable filter paths.
    /// - **remark**: package names under `./node_modules` that start with
    ///   `remark-`.
    pub available_plugins: Vec<String>,
    /// Actionable hint when the runtime isn't cleanly available. Mirrors
    /// [`RuntimeStatus::Missing.hint`] / [`RuntimeStatus::OutdatedVersion.hint`];
    /// `None` when the ecosystem is detected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// Aggregate `ztl ecosystem check` output (CON-3310).
///
/// `entries` is in canonical ecosystem order (pandoc → mdbook → remark);
/// `hooks_configured_total` is the grand total across every ecosystem —
/// zero means the zero-configured state and triggers the "No ecosystem
/// hooks configured" informational footer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EcosystemCheckReport {
    pub entries: Vec<EcosystemCheckEntry>,
    pub hooks_configured_total: usize,
}

impl EcosystemCheckReport {
    /// `true` when any *configured* ecosystem (one the vault has hooks
    /// against) is not [`EcosystemCheckStatus::Detected`]. This is the
    /// CON-3310 exit-code rule — unavailable runtimes only fail the check
    /// when the vault actually relies on them.
    pub fn has_configured_failures(&self) -> bool {
        self.entries
            .iter()
            .any(|e| e.configured > 0 && e.status != EcosystemCheckStatus::Detected)
    }

    /// Look up a single entry by id.
    pub fn entry(&self, id: &str) -> Option<&EcosystemCheckEntry> {
        self.entries.iter().find(|e| e.id == id)
    }
}

/// Build a [`EcosystemCheckReport`] from scratch: probe every runtime,
/// fold the composed hooks by ecosystem, and list reachable plugins.
///
/// `vault_root` is used to scope the remark plugin scan to the vault's
/// own `./node_modules`; the mdbook preprocessor scan uses `PATH`.
pub fn run_ecosystem_check(vault_root: &Path, hooks: &[ComposedHook]) -> EcosystemCheckReport {
    let detection = detect_all_ecosystems();
    assemble_report(&detection, hooks, vault_root)
}

/// Same as [`run_ecosystem_check`] but takes a pre-built
/// [`EcosystemDetectionReport`]. Exposed for tests that inject a
/// synthetic detection state and for hosts that already ran detection at
/// session start.
pub fn assemble_report(
    detection: &EcosystemDetectionReport,
    hooks: &[ComposedHook],
    vault_root: &Path,
) -> EcosystemCheckReport {
    let mut entries = Vec::with_capacity(ECOSYSTEMS.len());
    let mut total_configured = 0usize;

    for entry in ECOSYSTEMS {
        let configured_hooks: Vec<&ComposedHook> = hooks
            .iter()
            .filter(|h| h.ecosystem.as_deref() == Some(entry.id))
            .collect();
        let configured = configured_hooks.len();
        total_configured += configured;

        let runtime = detection.status_for(entry.id);
        let (status, version, executable, hint) = match runtime {
            Some(RuntimeStatus::Available {
                executable,
                version,
            }) => (
                EcosystemCheckStatus::Detected,
                Some(version.clone()),
                executable.clone(),
                None,
            ),
            Some(RuntimeStatus::OutdatedVersion {
                executable,
                found,
                hint,
                ..
            }) => (
                EcosystemCheckStatus::WrongVersion,
                Some(found.clone()),
                Some(executable.clone()),
                Some(hint.clone()),
            ),
            Some(RuntimeStatus::Missing { hint, .. }) => (
                EcosystemCheckStatus::Missing,
                None,
                None,
                Some(hint.clone()),
            ),
            Some(RuntimeStatus::ProbeFailed { detail }) => (
                EcosystemCheckStatus::Missing,
                None,
                None,
                Some(detail.clone()),
            ),
            None => (EcosystemCheckStatus::Missing, None, None, None),
        };

        let available_plugins = list_available_plugins(entry, vault_root, &configured_hooks);

        entries.push(EcosystemCheckEntry {
            id: entry.id.to_string(),
            status,
            version,
            executable,
            configured,
            available_plugins,
            hint,
        });
    }

    EcosystemCheckReport {
        entries,
        hooks_configured_total: total_configured,
    }
}

/// Enumerate the plugins that are *reachable* for this ecosystem,
/// following REQ-3310's per-ecosystem rules:
///
/// - mdbook: scan `$PATH` for `mdbook-*` executables.
/// - pandoc: list configured filter executables that exist on disk.
/// - remark: list `./node_modules/remark-*` package directories.
fn list_available_plugins(
    entry: &EcosystemEntry,
    vault_root: &Path,
    configured_hooks: &[&ComposedHook],
) -> Vec<String> {
    match entry.id {
        "pandoc" => reachable_pandoc_filters(configured_hooks),
        "mdbook" => mdbook_preprocessors_on_path(),
        "remark" => remark_plugins_in_node_modules(vault_root),
        _ => Vec::new(),
    }
}

/// For Pandoc, a filter is reachable when its configured executable path
/// exists on disk. Pandoc filters are arbitrary programs; there is no
/// host-wide registry to scan.
fn reachable_pandoc_filters(hooks: &[&ComposedHook]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for h in hooks {
        if h.path.is_file() {
            out.push(h.extension_id.clone());
        }
    }
    out.sort();
    out.dedup();
    out
}

/// mdBook discovers preprocessors by looking for `mdbook-*` binaries on
/// `PATH`. We mirror that so the report reflects what mdBook itself
/// would find.
fn mdbook_preprocessors_on_path() -> Vec<String> {
    let path_var = match std::env::var_os("PATH") {
        Some(v) => v,
        None => return Vec::new(),
    };
    let mut names: Vec<String> = Vec::new();
    for dir in std::env::split_paths(&path_var) {
        let read = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for e in read.flatten() {
            let file_name = match e.file_name().into_string() {
                Ok(s) => s,
                Err(_) => continue,
            };
            // Strip trailing `.exe` on Windows so the displayed name is
            // platform-stable (`mdbook-mermaid`, not `mdbook-mermaid.exe`).
            let stripped = file_name
                .strip_suffix(".exe")
                .unwrap_or(&file_name)
                .to_string();
            if !stripped.starts_with("mdbook-") {
                continue;
            }
            // Skip the `mdbook` binary itself; only preprocessors count.
            if stripped == "mdbook" {
                continue;
            }
            if !e.path().is_file() {
                continue;
            }
            names.push(stripped);
        }
    }
    names.sort();
    names.dedup();
    names
}

/// remark plugins live under `./node_modules/remark-*`. We list the
/// package directory names so `ztl ecosystem check` surfaces the set a
/// configured hook can actually `import()`.
fn remark_plugins_in_node_modules(vault_root: &Path) -> Vec<String> {
    let dir = vault_root.join("node_modules");
    let read = match std::fs::read_dir(&dir) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let mut names: Vec<String> = Vec::new();
    for e in read.flatten() {
        let name = match e.file_name().into_string() {
            Ok(s) => s,
            Err(_) => continue,
        };
        if !name.starts_with("remark-") {
            continue;
        }
        if !e.path().is_dir() {
            continue;
        }
        names.push(name);
    }
    names.sort();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecosystems::registry::by_id;
    use crate::hooks::composition::{ComposedHook, CompositionSource, DisabledReason};
    use crate::hooks::pipeline::Stage;
    use crate::hooks::translators::AstType;
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    fn make_hook(ecosystem: Option<&str>, ext_id: &str) -> ComposedHook {
        ComposedHook {
            stage: Stage::Transform,
            filename: format!("{ext_id}.py"),
            extension_id: ext_id.to_string(),
            path: PathBuf::from("/nonexistent/hook"),
            manifest_path: None,
            source: CompositionSource::Vault,
            before: Vec::new(),
            after: Vec::new(),
            optional: false,
            ast_type: AstType::default(),
            ast_version: None,
            preserves: Vec::new(),
            ecosystem: ecosystem.map(str::to_string),
            disabled: None::<DisabledReason>,
        }
    }

    fn fake_detection(
        pandoc: RuntimeStatus,
        mdbook: RuntimeStatus,
        remark: RuntimeStatus,
    ) -> EcosystemDetectionReport {
        let mut entries: BTreeMap<&'static str, RuntimeStatus> = BTreeMap::new();
        entries.insert(by_id("pandoc").unwrap().id, pandoc);
        entries.insert(by_id("mdbook").unwrap().id, mdbook);
        entries.insert(by_id("remark").unwrap().id, remark);
        EcosystemDetectionReport { entries }
    }

    #[test]
    fn status_mapping_matches_con_3310_axis() {
        let a = RuntimeStatus::Available {
            executable: None,
            version: "3.1".into(),
        };
        let o = RuntimeStatus::OutdatedVersion {
            executable: PathBuf::from("/bin/x"),
            found: "0.1".into(),
            required: "1.0".into(),
            hint: "upgrade".into(),
        };
        let m = RuntimeStatus::Missing {
            binary: "x".into(),
            hint: "install".into(),
        };
        let f = RuntimeStatus::ProbeFailed {
            detail: "boom".into(),
        };
        assert_eq!(
            EcosystemCheckStatus::from_runtime(&a),
            EcosystemCheckStatus::Detected
        );
        assert_eq!(
            EcosystemCheckStatus::from_runtime(&o),
            EcosystemCheckStatus::WrongVersion
        );
        assert_eq!(
            EcosystemCheckStatus::from_runtime(&m),
            EcosystemCheckStatus::Missing
        );
        // probe-failed folds into missing — same remediation axis.
        assert_eq!(
            EcosystemCheckStatus::from_runtime(&f),
            EcosystemCheckStatus::Missing
        );
    }

    #[test]
    fn report_in_canonical_order_pandoc_mdbook_remark() {
        let det = fake_detection(
            RuntimeStatus::Available {
                executable: None,
                version: "3.1".into(),
            },
            RuntimeStatus::Missing {
                binary: "mdbook".into(),
                hint: "install".into(),
            },
            RuntimeStatus::Missing {
                binary: "node".into(),
                hint: "install".into(),
            },
        );
        let tmp = TempDir::new().unwrap();
        let report = assemble_report(&det, &[], tmp.path());
        let ids: Vec<&str> = report.entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["pandoc", "mdbook", "remark"]);
    }

    #[test]
    fn zero_configured_state_yields_zero_total_and_no_failures() {
        // CON-3310: `No ecosystem hooks configured in this vault.` — the
        // aggregate must signal total == 0 and never flag failures even
        // when runtimes are missing.
        let det = fake_detection(
            RuntimeStatus::Missing {
                binary: "pandoc".into(),
                hint: "install".into(),
            },
            RuntimeStatus::Missing {
                binary: "mdbook".into(),
                hint: "install".into(),
            },
            RuntimeStatus::Missing {
                binary: "node".into(),
                hint: "install".into(),
            },
        );
        let tmp = TempDir::new().unwrap();
        let report = assemble_report(&det, &[], tmp.path());
        assert_eq!(report.hooks_configured_total, 0);
        assert!(!report.has_configured_failures());
        for e in &report.entries {
            assert_eq!(e.configured, 0);
            assert_eq!(e.status, EcosystemCheckStatus::Missing);
        }
    }

    #[test]
    fn configured_count_folds_hooks_by_ecosystem() {
        let hooks = vec![
            make_hook(Some("pandoc"), "crossref"),
            make_hook(Some("pandoc"), "citeproc"),
            make_hook(Some("remark"), "gfm"),
            make_hook(None, "native-callouts"),
        ];
        let det = fake_detection(
            RuntimeStatus::Available {
                executable: None,
                version: "3.1".into(),
            },
            RuntimeStatus::Missing {
                binary: "mdbook".into(),
                hint: "install".into(),
            },
            RuntimeStatus::Available {
                executable: None,
                version: "v20".into(),
            },
        );
        let tmp = TempDir::new().unwrap();
        let report = assemble_report(&det, &hooks, tmp.path());
        assert_eq!(report.entry("pandoc").unwrap().configured, 2);
        assert_eq!(report.entry("mdbook").unwrap().configured, 0);
        assert_eq!(report.entry("remark").unwrap().configured, 1);
        assert_eq!(report.hooks_configured_total, 3);
    }

    #[test]
    fn configured_but_missing_runtime_flags_failure() {
        // CON-3310 exit-code rule: non-zero only when a *configured*
        // ecosystem is missing its runtime.
        let hooks = vec![make_hook(Some("pandoc"), "crossref")];
        let det = fake_detection(
            RuntimeStatus::Missing {
                binary: "pandoc".into(),
                hint: "install".into(),
            },
            RuntimeStatus::Available {
                executable: None,
                version: "0.4".into(),
            },
            RuntimeStatus::Available {
                executable: None,
                version: "v20".into(),
            },
        );
        let tmp = TempDir::new().unwrap();
        let report = assemble_report(&det, &hooks, tmp.path());
        assert!(report.has_configured_failures());
        assert_eq!(
            report.entry("pandoc").unwrap().status,
            EcosystemCheckStatus::Missing
        );
        assert!(report.entry("pandoc").unwrap().hint.is_some());
    }

    #[test]
    fn unconfigured_missing_runtime_does_not_flag_failure() {
        // Zero-configured side: runtime absence is informational only.
        let det = fake_detection(
            RuntimeStatus::Missing {
                binary: "pandoc".into(),
                hint: "install".into(),
            },
            RuntimeStatus::Available {
                executable: None,
                version: "0.4".into(),
            },
            RuntimeStatus::Missing {
                binary: "node".into(),
                hint: "install".into(),
            },
        );
        let tmp = TempDir::new().unwrap();
        let report = assemble_report(&det, &[], tmp.path());
        assert!(!report.has_configured_failures());
    }

    #[test]
    fn remark_plugin_scan_finds_remark_packages_in_node_modules() {
        let tmp = TempDir::new().unwrap();
        let node_modules = tmp.path().join("node_modules");
        std::fs::create_dir_all(node_modules.join("remark-gfm")).unwrap();
        std::fs::create_dir_all(node_modules.join("remark-math")).unwrap();
        // Unrelated package must be skipped.
        std::fs::create_dir_all(node_modules.join("chalk")).unwrap();
        // File-named entry must be skipped (only dirs are packages).
        std::fs::write(node_modules.join("remark-not-a-dir"), b"").unwrap();

        let got = remark_plugins_in_node_modules(tmp.path());
        assert_eq!(
            got,
            vec!["remark-gfm".to_string(), "remark-math".to_string()]
        );
    }

    #[test]
    fn remark_plugin_scan_empty_when_no_node_modules() {
        let tmp = TempDir::new().unwrap();
        assert!(remark_plugins_in_node_modules(tmp.path()).is_empty());
    }

    #[test]
    fn pandoc_filter_scan_lists_configured_filters_that_exist_on_disk() {
        // Configure two pandoc hooks; only one has an on-disk executable.
        let tmp = TempDir::new().unwrap();
        let real = tmp.path().join("pandoc-crossref");
        std::fs::write(&real, "#!/bin/sh\ntrue\n").unwrap();

        let mut hook_exists = make_hook(Some("pandoc"), "crossref");
        hook_exists.path = real;
        let hook_missing = make_hook(Some("pandoc"), "ghost");

        let hooks_ref: Vec<&ComposedHook> = vec![&hook_exists, &hook_missing];
        let got = reachable_pandoc_filters(&hooks_ref);
        assert_eq!(got, vec!["crossref".to_string()]);
    }

    #[test]
    fn entry_lookup_returns_none_for_unknown_id() {
        let det = fake_detection(
            RuntimeStatus::Available {
                executable: None,
                version: "3.1".into(),
            },
            RuntimeStatus::Available {
                executable: None,
                version: "0.4".into(),
            },
            RuntimeStatus::Available {
                executable: None,
                version: "v20".into(),
            },
        );
        let tmp = TempDir::new().unwrap();
        let report = assemble_report(&det, &[], tmp.path());
        assert!(report.entry("djot").is_none());
        assert!(report.entry("pandoc").is_some());
    }

    #[test]
    fn json_shape_matches_con_3310_fields() {
        // CON-3310: JSON is an array of objects with `id`, `status`,
        // `version`, `configured`, `available_plugins`. We serialise the
        // entries and assert those keys are present on each row.
        let det = fake_detection(
            RuntimeStatus::Available {
                executable: None,
                version: "3.1".into(),
            },
            RuntimeStatus::Missing {
                binary: "mdbook".into(),
                hint: "install".into(),
            },
            RuntimeStatus::Available {
                executable: None,
                version: "v20".into(),
            },
        );
        let tmp = TempDir::new().unwrap();
        let report = assemble_report(&det, &[], tmp.path());
        let json = serde_json::to_value(&report.entries).unwrap();
        let arr = json.as_array().expect("entries must serialise as array");
        assert_eq!(arr.len(), ECOSYSTEMS.len());
        for row in arr {
            let obj = row.as_object().unwrap();
            assert!(obj.contains_key("id"));
            assert!(obj.contains_key("status"));
            assert!(obj.contains_key("configured"));
            assert!(obj.contains_key("available_plugins"));
        }
    }
}
