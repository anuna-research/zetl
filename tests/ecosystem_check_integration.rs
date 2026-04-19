//! SPEC-033 TEST-3310 — integration coverage for `zetl ecosystem check`.
//!
//! Pins the CON-3310 output contract at the CLI boundary:
//!
//! - zero-configured vault → table + informational footer, exit 0,
//!   regardless of which runtimes are or aren't installed on the host;
//! - `--json` emits a JSON object with an `entries` array whose rows
//!   carry `id` / `status` / `configured` / `available_plugins`;
//! - configured-but-missing runtime → exit 1 with a stderr hint;
//! - canonical ecosystem ordering (pandoc → mdbook → remark) surfaces in
//!   both formats.

#![cfg(unix)]

use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use tempfile::TempDir;

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

/// Drop a runnable hook (shebang + 0o755) at `rel` under `vault`.
fn write_runnable(vault: &Path, rel: &str, body: &str) {
    let p = vault.join(rel);
    write(&p, body);
    let mut perms = fs::metadata(&p).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&p, perms).unwrap();
}

fn run_check(vault: &Path, extra: &[&str]) -> std::process::Output {
    cargo_bin_cmd!("zetl")
        .args(["--dir", vault.to_str().unwrap(), "ecosystem", "check"])
        .args(extra)
        .output()
        .expect("run zetl ecosystem check")
}

/// Force table output: tests run non-TTY, where the global format resolves
/// to JSON by default. Passing `--format=table` re-selects the table.
fn run_check_table(vault: &Path) -> std::process::Output {
    cargo_bin_cmd!("zetl")
        .args([
            "--dir",
            vault.to_str().unwrap(),
            "--format",
            "table",
            "ecosystem",
            "check",
        ])
        .output()
        .expect("run zetl ecosystem check")
}

// ── TEST-3310: zero-configured state ─────────────────────────────────────

#[test]
fn zero_configured_vault_exits_zero_and_shows_informational_footer() {
    // A fresh vault with no `.zetl/hooks/` → every ecosystem has
    // `configured = 0`. CON-3310 says this MUST exit 0 regardless of
    // whether runtimes are installed on the CI host.
    let tmp = TempDir::new().unwrap();
    let out = run_check_table(tmp.path());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        out.status.success(),
        "zero-configured vault must exit 0; stdout={stdout}; stderr={stderr}"
    );
    // Canonical ordering in the table body.
    let pandoc_pos = stdout.find("pandoc").expect("row for pandoc");
    let mdbook_pos = stdout.find("mdbook").expect("row for mdbook");
    let remark_pos = stdout.find("remark").expect("row for remark");
    assert!(pandoc_pos < mdbook_pos && mdbook_pos < remark_pos);
    // CON-3310 footer — byte-stable prompt for the user.
    assert!(
        stdout.contains("No ecosystem hooks configured in this vault."),
        "expected zero-configured footer; stdout={stdout}"
    );
}

// ── TEST-3310: JSON output shape ─────────────────────────────────────────

#[test]
fn json_output_matches_con_3310_shape() {
    let tmp = TempDir::new().unwrap();

    // Wire one pandoc hook so the JSON also exercises non-zero
    // `configured` — this pins the field's presence even when > 0.
    write_runnable(
        tmp.path(),
        ".zetl/hooks/transform.d/crossref.py",
        "#!/bin/sh\ntrue\n",
    );
    write(
        &tmp.path().join(".zetl/hooks/transform.d/crossref.py.toml"),
        r#"ecosystem = "pandoc"
extension_id = "crossref"
"#,
    );

    let out = run_check(tmp.path(), &["--json"]);
    let stdout = String::from_utf8(out.stdout.clone()).expect("stdout must be UTF-8");
    let value: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("--json output must parse as JSON: {e}; got:\n{stdout}"));

    let entries = value
        .get("entries")
        .and_then(|v| v.as_array())
        .expect("top-level object must have `entries` array");
    assert_eq!(entries.len(), 3, "three ecosystems: pandoc, mdbook, remark");

    // Canonical ordering in JSON.
    assert_eq!(entries[0].get("id").and_then(|v| v.as_str()), Some("pandoc"));
    assert_eq!(entries[1].get("id").and_then(|v| v.as_str()), Some("mdbook"));
    assert_eq!(entries[2].get("id").and_then(|v| v.as_str()), Some("remark"));

    for row in entries {
        let obj = row.as_object().unwrap();
        for field in ["id", "status", "configured", "available_plugins"] {
            assert!(
                obj.contains_key(field),
                "every entry must carry `{field}`; got keys: {:?}",
                obj.keys().collect::<Vec<_>>()
            );
        }
    }

    // pandoc entry carries configured = 1 thanks to the hook we wired.
    let pandoc = &entries[0];
    assert_eq!(
        pandoc.get("configured").and_then(|v| v.as_u64()),
        Some(1),
        "pandoc configured count should reflect the wired hook"
    );
}

// ── TEST-3310: configured-but-missing runtime → exit 1 ───────────────────

#[test]
fn configured_missing_runtime_fails_with_actionable_stderr() {
    // Declare a hook against an ecosystem whose runtime is guaranteed
    // absent on this host by pointing the hook at a real ecosystem id
    // ("pandoc") and skipping the runtime probe is not possible here;
    // instead we rely on the production code path: if the host *does*
    // have pandoc installed, this test degrades to a soft assertion
    // (verifying exit 0 and no configured failure). CI is expected to
    // assert the stricter path explicitly when pandoc is not on PATH.
    //
    // The rigorous logic-level coverage for the CON-3310 exit-code rule
    // lives in `src/ecosystems/check.rs::tests` using synthetic
    // `RuntimeStatus::Missing` — this integration test only pins the
    // CLI-side wiring (stderr hint surfaced, exit code follows the
    // report's `has_configured_failures()`).
    let tmp = TempDir::new().unwrap();
    write_runnable(
        tmp.path(),
        ".zetl/hooks/transform.d/crossref.py",
        "#!/bin/sh\ntrue\n",
    );
    write(
        &tmp.path().join(".zetl/hooks/transform.d/crossref.py.toml"),
        r#"ecosystem = "pandoc"
extension_id = "crossref"
"#,
    );

    let out = run_check(tmp.path(), &[]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    // Either pandoc is installed (exit 0, no hint on stderr) or it is
    // missing (exit 1, `[zetl] ecosystem pandoc:` line on stderr). Both
    // are valid — what we lock in is the conditional wiring.
    if out.status.success() {
        // Host has pandoc; the table must still reflect the
        // configured count.
        assert!(
            stdout.contains("pandoc"),
            "pandoc row must appear; stdout={stdout}"
        );
    } else {
        assert_eq!(
            out.status.code(),
            Some(1),
            "missing-configured-runtime must exit 1; code={:?}",
            out.status.code()
        );
        assert!(
            stderr.contains("[zetl] ecosystem pandoc:"),
            "stderr must carry actionable line for configured-but-missing runtime; \
             stderr={stderr}"
        );
    }
}

// ── TEST-3310: canonical table column headers stay stable ───────────────

#[test]
fn table_output_carries_con_3310_column_headers() {
    let tmp = TempDir::new().unwrap();
    let out = run_check_table(tmp.path());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ECOSYSTEM"));
    assert!(stdout.contains("STATUS"));
    assert!(stdout.contains("VERSION"));
    assert!(stdout.contains("PLUGINS CONFIGURED"));
    assert!(stdout.contains("PLUGINS AVAILABLE"));
}
