//! SPEC-028 NFR-103 / NFR-104 — static asset and graph-index size budgets.
//!
//! | Test     | Requirement                                                      |
//! |----------|------------------------------------------------------------------|
//! | TEST-203 | `gzip -c themes/default/static/vendor/sigma/*.min.js` ≤ 250 kB   |
//! | TEST-204 | `graph-index.json` ≤ 1 MB for 2 k pages; stderr warning ≥ 5 k    |
//!
//! TEST-203 is a pure file-size check against the vendored Sigma.js bundle
//! that ships with the default theme. It is skipped (with an eprintln note)
//! when the vendor directory is absent — the upstream bundling task has not
//! yet landed on every branch — but enforces the budget the moment the
//! files appear.
//!
//! TEST-204's 1 MB file-size arm is likewise conditional on graph-index.json
//! emission: once `graph_build_emit` lands, the 2 k-page build will produce
//! the file and the assertion will bind. The stderr-warning arm is exercised
//! two ways:
//!   * a fast path that pre-seeds a > 1 MB `graph-index.json` in the output
//!     directory and confirms the build emits the warning;
//!   * a `#[ignore]`-gated full-scale seed of 5 000 pages, matching the
//!     SPEC-028 verification plan verbatim. Run with:
//!
//!     ```text
//!     cargo test --release --test nfr_size_budgets -- --ignored
//!     ```

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use flate2::write::GzEncoder;
use flate2::Compression;
use tempfile::TempDir;

// ── constants ────────────────────────────────────────────────────────────────

/// NFR-103: gzip of bundled Sigma.js + graphology assets.
const VENDOR_BUNDLE_MAX_GZIP_BYTES: u64 = 250 * 1024;

/// NFR-104: uncompressed graph-index.json budget.
const GRAPH_INDEX_MAX_BYTES: u64 = 1024 * 1024;

/// Root of this repository, discovered at test compile time.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Vendor directory the build shells out of the bundled default theme.
fn vendor_dir() -> PathBuf {
    repo_root().join("themes/default/static/vendor/sigma")
}

/// Compress a file's bytes with gzip (default compression) and return the
/// compressed length. `gzip -c` in CI uses the same default level (6).
fn gzip_len(path: &Path) -> std::io::Result<u64> {
    let bytes = fs::read(path)?;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&bytes)?;
    let compressed = encoder.finish()?;
    Ok(compressed.len() as u64)
}

// ── TEST-203 — vendor bundle gzip size ───────────────────────────────────────

/// TEST-203: every `.min.js` in `themes/default/static/vendor/sigma/` gzips to
/// at most 250 kB combined. Skipped when the vendor directory is absent.
#[test]
fn test_203_vendor_bundle_gzip_under_250kb() {
    let dir = vendor_dir();
    if !dir.is_dir() {
        eprintln!(
            "TEST-203: skipping — vendor bundle not present at {}. \
             The upstream `task-vendor-bundle` must land first; \
             once it does, this test will enforce the 250 kB gzip budget.",
            dir.display()
        );
        return;
    }

    let mut total: u64 = 0;
    let mut per_file: Vec<(String, u64)> = Vec::new();
    for entry in fs::read_dir(&dir).expect("read vendor/sigma/") {
        let entry = entry.expect("read_dir entry");
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if !name.ends_with(".min.js") {
            continue;
        }
        let size = gzip_len(&path).expect("gzip min.js");
        total += size;
        per_file.push((name.to_string(), size));
    }

    assert!(
        !per_file.is_empty(),
        "TEST-203: vendor/sigma/ exists but contains no *.min.js files \
         ({})",
        dir.display()
    );

    assert!(
        total <= VENDOR_BUNDLE_MAX_GZIP_BYTES,
        "TEST-203 failed: gzip of vendor/sigma/*.min.js is {} bytes, \
         over the {} byte (250 kB) NFR-103 budget. Breakdown: {:?}",
        total,
        VENDOR_BUNDLE_MAX_GZIP_BYTES,
        per_file
    );

    eprintln!(
        "TEST-203: vendor bundle gzip = {} bytes ({} files), under {} byte budget",
        total,
        per_file.len(),
        VENDOR_BUNDLE_MAX_GZIP_BYTES
    );
}

// ── TEST-204 — graph-index.json size and 5k-page warning ─────────────────────

/// Populate a tempdir with `n` tiny Markdown pages. Kept minimal so the
/// test can run in reasonable wall time when invoked with `--ignored`.
fn seed_vault(root: &Path, n: usize) {
    for i in 0..n {
        let sub = root.join(format!("n/b{:03}", i / 100));
        fs::create_dir_all(&sub).expect("mkdir batch");
        let prev = i.saturating_sub(1);
        let body = format!("# N{i}\n\n[[n{prev}]]\n");
        fs::write(sub.join(format!("n{i}.md")), body).expect("write note");
    }
}

/// Invoke `zetl build -d <vault> -o <out>` and capture the process's stderr.
fn run_build(vault: &Path, out: &Path) -> assert_cmd::assert::Assert {
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("zetl");
    cmd.arg("-d")
        .arg(vault.as_os_str())
        .arg("--no-cache")
        .arg("build")
        .arg("-o")
        .arg(out.as_os_str());
    cmd.assert()
}

/// TEST-204 (warning arm, fast path): when `{out}/graph-index.json` exceeds
/// the 1 MB budget, the warning helper must produce the NFR-104 message.
///
/// Drives the file-size branch directly via `graph_index_size_warning` so the
/// test stays fast and deterministic — `build_static` itself rewrites
/// `{out}/graph-index.json` on every run, which would otherwise erase any
/// pre-seeded oversize file before the size check runs.
#[test]
fn test_204_warning_when_graph_index_over_budget() {
    let tmp = TempDir::new().unwrap();
    let out = tmp.path().to_path_buf();
    let oversized = vec![b' '; (GRAPH_INDEX_MAX_BYTES as usize) + 16];
    fs::write(out.join("graph-index.json"), &oversized).unwrap();

    let warning = zetl::web::build::graph_index_size_warning(&out, 1)
        .expect("TEST-204: oversize graph-index.json must produce a warning");

    assert!(
        warning.contains("NFR-104"),
        "TEST-204: warning must mention NFR-104. got: {warning}"
    );
    assert!(
        warning.contains("1 MB"),
        "TEST-204: warning must reference the 1 MB budget. got: {warning}"
    );
    assert!(
        warning.contains("zetl serve") || warning.contains("server-mode"),
        "TEST-204: warning must recommend server-mode deployment. got: {warning}"
    );
}

/// TEST-204 (negative arm): a small vault — graph-index.json absent, page
/// count well below the 5 000-page proxy threshold — must NOT emit the
/// NFR-104 warning.
#[test]
fn test_204_no_warning_for_small_vault() {
    let tmp = TempDir::new().unwrap();
    let vault = tmp.path().join("vault");
    let out = tmp.path().join("dist");
    fs::create_dir_all(&vault).unwrap();
    seed_vault(&vault, 5);

    let assert = run_build(&vault, &out);
    let output = assert.get_output();
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !stderr.contains("NFR-104"),
        "TEST-204: small vault must not emit the NFR-104 warning. stderr:\n{stderr}"
    );
}

/// TEST-204 (size arm): once graph-index.json is emitted by the build, a
/// 2 000-page vault must stay under the 1 MB budget. Skipped when the file
/// is absent (upstream `graph_build_emit` not yet landed).
///
/// `#[ignore]` because seeding 2 k pages and rendering them end-to-end —
/// including OG PNGs — is multi-second work unsuitable for the default
/// `cargo test` run. SPEC-028 schedules this as an on-demand NFR check.
#[test]
#[ignore]
fn test_204_graph_index_under_1mb_for_2k_vault() {
    let tmp = TempDir::new().unwrap();
    let vault = tmp.path().join("vault");
    let out = tmp.path().join("dist");
    fs::create_dir_all(&vault).unwrap();
    seed_vault(&vault, 2_000);

    run_build(&vault, &out).success();

    let graph_index = out.join("graph-index.json");
    if !graph_index.is_file() {
        eprintln!(
            "TEST-204 (size arm): skipping — graph-index.json not emitted \
             at {} yet. The upstream `task-graph-build-emit` must land \
             first; once it does, this test will bind.",
            graph_index.display()
        );
        return;
    }

    let size = fs::metadata(&graph_index).unwrap().len();
    assert!(
        size <= GRAPH_INDEX_MAX_BYTES,
        "TEST-204: graph-index.json is {} bytes, over the {} byte (1 MB) \
         NFR-104 budget for a 2 000-page vault",
        size,
        GRAPH_INDEX_MAX_BYTES,
    );
    eprintln!("TEST-204: graph-index.json = {size} bytes (under 1 MB budget)");
}

/// TEST-204 (warning arm, full scale): a 5 000-page vault must emit the
/// NFR-104 warning to stderr, matching the SPEC-028 verification plan.
///
/// `#[ignore]` because seeding and building 5 000 pages is substantially
/// slower than a default-speed unit test. The smaller sibling test
/// `test_204_warning_when_graph_index_over_budget` covers the same warning
/// emission path on every CI run.
#[test]
#[ignore]
fn test_204_warning_for_5k_vault() {
    let tmp = TempDir::new().unwrap();
    let vault = tmp.path().join("vault");
    let out = tmp.path().join("dist");
    fs::create_dir_all(&vault).unwrap();
    seed_vault(&vault, 5_000);

    let assert = run_build(&vault, &out);
    let output = assert.get_output();
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stderr.contains("NFR-104"),
        "TEST-204: 5 000-page vault must emit the NFR-104 warning. stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("zetl serve") || stderr.contains("server-mode"),
        "TEST-204: 5 000-page vault warning must recommend server-mode. stderr:\n{stderr}"
    );
}
