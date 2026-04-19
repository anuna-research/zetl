//! Hook-authoring CLI surface (SPEC-032 REQ-3225 / TEST-3225).
//!
//! Implements the four subcommands that close the author loop for
//! render-pipeline hooks:
//!
//! - [`scaffold`] — `zetl hook new <stage> <name>`: writes a persistent-mode
//!   skeleton, a sidecar manifest with sensible defaults, and a minimal
//!   test fixture whose golden is seeded so `hook test` passes immediately.
//! - [`run_test`] — `zetl hook test <name> [--update]`: runs the hook
//!   against its fixture and diffs the output; `--update` regenerates
//!   the golden instead.
//! - [`capture_fixture`] — `zetl hook fixture --from <page> --hook <name>`:
//!   copies a vault page into the fixture directory and seeds the golden
//!   from the hook's current output.
//! - [`watch`] — `zetl hook watch <name>`: file-watches the hook's source
//!   and restarts the persistent subprocess on change, streaming stderr.
//!
//! The runner owns the one piece of stage-specific logic the CLI has to
//! understand: payload shape at the stage boundary. `pre-parse` exchanges
//! raw markdown strings, `transform` exchanges `Document` JSON, and
//! `post-render` exchanges HTML-fragment strings. The golden file
//! extension (`.md` / `.json` / `.html`) follows the payload shape.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;

use crate::cli::{AuthoringStage, HookEcosystem, HookLang};
use crate::hooks::ast::parse_markdown;
use crate::hooks::persistent::{HookMessage, PersistentHook, DEFAULT_DEADLINE_MS};
use crate::hooks::pipeline::Stage;

/// Directory under the vault where fixtures live. SPEC-032 REQ-3225.
pub const FIXTURES_DIR: &str = "tests/hook-fixtures";

fn stage_to_pipeline(stage: &AuthoringStage) -> Stage {
    match stage {
        AuthoringStage::PreParse => Stage::PreParse,
        AuthoringStage::Transform => Stage::Transform,
        AuthoringStage::PostRender => Stage::PostRender,
    }
}

#[cfg(test)]
fn stage_from_pipeline(stage: Stage) -> AuthoringStage {
    match stage {
        Stage::PreParse => AuthoringStage::PreParse,
        Stage::Transform => AuthoringStage::Transform,
        Stage::PostRender => AuthoringStage::PostRender,
    }
}

/// File extension of the fixture golden for each stage — follows the
/// payload shape exchanged with the hook at the stage boundary.
fn golden_filename(stage: Stage) -> &'static str {
    match stage {
        Stage::PreParse => "expected.md",
        Stage::Transform => "expected.json",
        Stage::PostRender => "expected.html",
    }
}

// ── Scaffolding (`zetl hook new`) ──────────────────────────────────────────

/// Result of a successful [`scaffold`] call — the four paths created so
/// the CLI can print a tidy listing.
pub struct ScaffoldPaths {
    pub hook_file: PathBuf,
    pub manifest_file: PathBuf,
    pub fixture_input: PathBuf,
    pub fixture_expected: PathBuf,
}

/// Create the skeleton hook, its sidecar manifest, and the starter fixture.
///
/// `stage` determines the `<stage>.d/` directory, the fixture golden
/// filename, and the payload-echo body of the skeleton. `lang` picks the
/// language + shebang. `ecosystem` layers on an ecosystem stub (currently
/// a comment pointer — full adapter wiring is owned by Phase E/F/G
/// tasks in IMPL-032-033).
pub fn scaffold(
    vault_root: &Path,
    stage: &AuthoringStage,
    name: &str,
    lang: &HookLang,
    ecosystem: Option<&HookEcosystem>,
    force: bool,
) -> Result<ScaffoldPaths> {
    validate_name(name)?;

    let pipeline_stage = stage_to_pipeline(stage);
    let stage_dir = vault_root
        .join(".zetl")
        .join("hooks")
        .join(format!("{}.d", stage.as_str()));
    std::fs::create_dir_all(&stage_dir)
        .with_context(|| format!("creating hooks stage dir {}", stage_dir.display()))?;

    let hook_file = stage_dir.join(format!("{}.{}", name, lang.ext()));
    let manifest_file = stage_dir.join(format!("{name}.toml"));
    let fixture_dir = vault_root.join(FIXTURES_DIR).join(name);
    std::fs::create_dir_all(&fixture_dir)
        .with_context(|| format!("creating fixture dir {}", fixture_dir.display()))?;
    let fixture_input = fixture_dir.join("input.md");
    let fixture_expected = fixture_dir.join(golden_filename(pipeline_stage));

    for path in [&hook_file, &manifest_file, &fixture_input, &fixture_expected] {
        if path.exists() && !force {
            bail!(
                "refusing to overwrite existing file {} (pass --force to overwrite)",
                path.display()
            );
        }
    }

    let skeleton = render_skeleton(lang, name, pipeline_stage, ecosystem);
    std::fs::write(&hook_file, skeleton)
        .with_context(|| format!("writing hook skeleton {}", hook_file.display()))?;
    make_executable(&hook_file)?;

    let manifest = render_manifest(stage, name, lang, ecosystem);
    std::fs::write(&manifest_file, manifest)
        .with_context(|| format!("writing hook manifest {}", manifest_file.display()))?;

    let input = DEFAULT_FIXTURE_INPUT;
    std::fs::write(&fixture_input, input)
        .with_context(|| format!("writing fixture input {}", fixture_input.display()))?;

    // Seed the golden from the identity-transform output so `hook test`
    // passes immediately (SPEC-032 TEST-3225 row 1).
    let expected = identity_output_for_stage(pipeline_stage, input)?;
    std::fs::write(&fixture_expected, expected)
        .with_context(|| format!("writing fixture golden {}", fixture_expected.display()))?;

    Ok(ScaffoldPaths {
        hook_file,
        manifest_file,
        fixture_input,
        fixture_expected,
    })
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("hook name cannot be empty");
    }
    // Extension ids namespace template vars (REQ-3214) — keep them tame.
    for ch in name.chars() {
        if !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-') {
            bail!(
                "invalid hook name '{name}': only ascii alphanumerics, '_', and '-' are allowed"
            );
        }
    }
    Ok(())
}

const DEFAULT_FIXTURE_INPUT: &str = "# Hello

This is a minimal fixture for testing the hook. Edit input.md to
exercise the transformations your hook performs, then run
`zetl hook test <name>` (or `zetl hook test <name> --update` to
regenerate the golden).
";

fn render_manifest(
    stage: &AuthoringStage,
    name: &str,
    lang: &HookLang,
    ecosystem: Option<&HookEcosystem>,
) -> String {
    let ecosystem_note = match ecosystem {
        Some(e) => format!(
            "\n# Ecosystem adapter: {} (SPEC-033). Wire the companion plugin\n\
             # under .zetl/hooks/{}.d/ before enabling in builds.\n",
            e.as_str(),
            stage.as_str()
        ),
        None => String::new(),
    };
    format!(
        r#"# Manifest for hook `{name}` ({lang_ext}). Generated by `zetl hook new`.
# Full field reference:
#   https://zetl.codeberg.page/docs/hook-authoring/manifest-fields
{ecosystem_note}
stage = "{stage}"
mode = "persistent"
timeout_ms = 100
extension_id = "{name}"

[select]
include = ["**/*.md"]
exclude = []

[contract]
preserves = []
idempotent = true
may_restructure = false
"#,
        stage = stage.as_str(),
        name = name,
        ecosystem_note = ecosystem_note,
        lang_ext = lang.ext(),
    )
}

fn render_skeleton(
    lang: &HookLang,
    name: &str,
    stage: Stage,
    ecosystem: Option<&HookEcosystem>,
) -> String {
    let eco_comment = match ecosystem {
        Some(e) => format!(
            "# Scaffolded with --ecosystem {}. The companion adapter lives\n# under SPEC-033 and is not yet bound in this skeleton.\n",
            e.as_str()
        ),
        None => String::new(),
    };
    match lang {
        HookLang::Py => render_py_skeleton(name, stage, &eco_comment),
        HookLang::Js => render_js_skeleton(name, stage, &eco_comment),
        HookLang::Sh => render_sh_skeleton(name, stage, &eco_comment),
    }
}

fn render_py_skeleton(name: &str, _stage: Stage, eco_comment: &str) -> String {
    format!(
        r#"#!/usr/bin/env python3
# -*- coding: utf-8 -*-
# `{name}` render-pipeline hook.
#
# SPEC-032 CON-3201 persistent-mode protocol. Reads JSON-lines from stdin,
# writes JSON-lines to stdout. See:
#   https://zetl.codeberg.page/docs/hook-authoring
{eco_comment}
import json, sys

# Handshake.
sys.stdout.write(json.dumps({{
    "zetl_ast": 1,
    "hook": "{name}",
    "version": "0.1.0",
    "ready": True,
}}) + "\n")
sys.stdout.flush()

for line in sys.stdin:
    try:
        msg = json.loads(line)
    except Exception as e:
        sys.stderr.write(f"[{name}] malformed request: {{e}}\n")
        continue
    t = msg.get("type")
    if t == "shutdown":
        break
    # Identity transform. Replace this with your transformation.
    payload = msg.get("payload", None)
    response = {{
        "type": "result",
        "payload": payload,
        "diagnostics": [],
        "template_vars": {{}},
    }}
    sys.stdout.write(json.dumps(response) + "\n")
    sys.stdout.flush()
"#
    )
}

fn render_js_skeleton(name: &str, _stage: Stage, eco_comment: &str) -> String {
    format!(
        r#"#!/usr/bin/env node
// `{name}` render-pipeline hook.
//
// SPEC-032 CON-3201 persistent-mode protocol. Reads JSON-lines from stdin,
// writes JSON-lines to stdout. See:
//   https://zetl.codeberg.page/docs/hook-authoring
{eco_comment}
'use strict';

const readline = require('readline');
const rl = readline.createInterface({{ input: process.stdin }});

process.stdout.write(JSON.stringify({{
    zetl_ast: 1,
    hook: '{name}',
    version: '0.1.0',
    ready: true,
}}) + '\n');

rl.on('line', (line) => {{
    let msg;
    try {{
        msg = JSON.parse(line);
    }} catch (e) {{
        process.stderr.write(`[{name}] malformed request: ${{e}}\n`);
        return;
    }}
    if (msg.type === 'shutdown') {{
        process.exit(0);
    }}
    // Identity transform. Replace this with your transformation.
    const response = {{
        type: 'result',
        payload: msg.payload ?? null,
        diagnostics: [],
        template_vars: {{}},
    }};
    process.stdout.write(JSON.stringify(response) + '\n');
}});
"#
    )
}

fn render_sh_skeleton(name: &str, _stage: Stage, eco_comment: &str) -> String {
    // sh skeleton uses python3 under the hood — `sh` is a delivery
    // vehicle for wrapping an existing binary. Pure-shell JSON handling
    // is error-prone and we intentionally avoid it.
    format!(
        r#"#!/usr/bin/env sh
# `{name}` render-pipeline hook.
#
# SPEC-032 CON-3201 persistent-mode protocol. This shell skeleton
# shells out to python3 for JSON handling; replace with your own
# language once it's worth the churn.
{eco_comment}
exec python3 - <<'PY'
import json, sys

sys.stdout.write(json.dumps({{
    "zetl_ast": 1, "hook": "{name}", "version": "0.1.0", "ready": True,
}}) + "\n")
sys.stdout.flush()

for line in sys.stdin:
    try:
        msg = json.loads(line)
    except Exception:
        continue
    if msg.get("type") == "shutdown":
        break
    response = {{
        "type": "result",
        "payload": msg.get("payload"),
        "diagnostics": [],
        "template_vars": {{}},
    }}
    sys.stdout.write(json.dumps(response) + "\n")
    sys.stdout.flush()
PY
"#
    )
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    // Windows: no-op. `zetl hook test` and `watch` use the file path as
    // the command; the shell extension map resolves `.py` / `.js` / `.sh`
    // via the user's PATHEXT, which is a user concern on that platform.
    Ok(())
}

fn identity_output_for_stage(stage: Stage, input: &str) -> Result<String> {
    match stage {
        Stage::PreParse => Ok(input.to_string()),
        Stage::Transform => {
            let doc = parse_markdown(input);
            let value = serde_json::to_value(&doc).context("serialising parsed AST")?;
            Ok(serde_json::to_string_pretty(&value)? + "\n")
        }
        Stage::PostRender => Ok(render_markdown_to_html(input)),
    }
}

fn render_markdown_to_html(content: &str) -> String {
    // Empty slug_map + root_path: wikilinks render with `link-error`
    // class, matching the zero-context fixture harness used by
    // `zetl ast sample --stage post-render`. Matching behaviour avoids
    // surprising drift between the two commands' outputs.
    crate::web::markdown::render_to_html(content, &HashMap::new(), "", "index.html")
}

// ── Hook discovery on disk ─────────────────────────────────────────────────

/// A hook discovered inside a vault's `.zetl/hooks/<stage>.d/` tree, plus
/// any sidecar manifest. Used by `hook test`, `hook fixture`, and `hook
/// watch` to locate the author's work.
pub struct ScaffoldedHook {
    pub stage: Stage,
    pub name: String,
    pub path: PathBuf,
    pub manifest_path: Option<PathBuf>,
}

/// Find a hook by extension id (`name`) under any of the three stage
/// directories. If multiple files share the same stem across languages,
/// the first one wins — authors are expected to keep one skeleton per
/// extension id.
pub fn find_scaffolded_hook(vault_root: &Path, name: &str) -> Result<ScaffoldedHook> {
    for stage in Stage::all() {
        let stage_dir = vault_root
            .join(".zetl")
            .join("hooks")
            .join(format!("{}.d", stage.as_str()));
        if !stage_dir.is_dir() {
            continue;
        }
        let entries = std::fs::read_dir(&stage_dir)
            .with_context(|| format!("reading stage dir {}", stage_dir.display()))?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
                continue;
            };
            if ext == "toml" {
                continue;
            }
            if stem != name {
                continue;
            }
            let manifest_path = path.with_extension("toml");
            let manifest_path = manifest_path.exists().then_some(manifest_path);
            return Ok(ScaffoldedHook {
                stage,
                name: name.to_string(),
                path,
                manifest_path,
            });
        }
    }
    Err(anyhow!(
        "no hook named '{name}' under .zetl/hooks/{{pre-parse,transform,post-render}}.d/",
    ))
}

// ── Hook test runner (`zetl hook test`) ───────────────────────────────────

/// Outcome of a single [`run_test`] invocation.
pub enum TestOutcome {
    /// Actual output matched the golden byte-for-byte.
    Match,
    /// `--update` rewrote the golden.
    Updated { path: PathBuf },
    /// Actual and golden diverged. `diff` is a pre-rendered unified
    /// text-diff for terminal display.
    Mismatch {
        actual: String,
        expected: String,
        diff: String,
    },
}

/// Run a scaffolded hook against its fixture and compare output to the
/// stored golden.
///
/// `update = true` overwrites the golden with the fresh output instead of
/// diffing. On first scaffold this is a no-op (the golden is already
/// identity-seeded); after an author edits the hook it's the knob they
/// reach for to accept a new baseline.
pub fn run_test(vault_root: &Path, name: &str, update: bool) -> Result<TestOutcome> {
    let hook = find_scaffolded_hook(vault_root, name)?;
    let fixture_dir = vault_root.join(FIXTURES_DIR).join(name);
    if !fixture_dir.is_dir() {
        bail!(
            "fixture directory missing: {} (run `zetl hook new` or \
             `zetl hook fixture --from <page> --hook {name}`)",
            fixture_dir.display()
        );
    }
    let input_path = fixture_dir.join("input.md");
    let input = std::fs::read_to_string(&input_path)
        .with_context(|| format!("reading fixture input {}", input_path.display()))?;

    let golden_path = fixture_dir.join(golden_filename(hook.stage));
    let actual = run_hook_once(&hook, &input)?;

    if update {
        std::fs::write(&golden_path, &actual)
            .with_context(|| format!("writing golden {}", golden_path.display()))?;
        return Ok(TestOutcome::Updated { path: golden_path });
    }

    let expected = std::fs::read_to_string(&golden_path)
        .with_context(|| format!("reading golden {}", golden_path.display()))?;
    if actual == expected {
        Ok(TestOutcome::Match)
    } else {
        let diff = render_line_diff(&expected, &actual);
        Ok(TestOutcome::Mismatch {
            actual,
            expected,
            diff,
        })
    }
}

/// Spawn the hook via the persistent protocol, send one init + one run,
/// then shut down. Returns the actual stage-output string that should
/// match the golden.
fn run_hook_once(hook: &ScaffoldedHook, input: &str) -> Result<String> {
    let payload = payload_for_input(hook.stage, input)?;

    let mut cmd = Command::new(&hook.path);
    cmd.current_dir(hook.path.parent().unwrap_or(Path::new(".")));
    let mut child = PersistentHook::spawn(cmd, hook.name.clone(), hook.stage)
        .with_context(|| format!("spawning persistent hook {}", hook.path.display()))?;

    let _ = child
        .init(Value::Object(Default::default()), DEFAULT_DEADLINE_MS)
        .context("init exchange with hook")?;

    let page_slug = "fixture-input";
    let frontmatter = Value::Object(Default::default());
    let response = child
        .run(page_slug, frontmatter, payload, DEFAULT_DEADLINE_MS)
        .context("run exchange with hook")?;

    let payload = match response {
        HookMessage::Result { payload, .. } => payload,
        HookMessage::Error { reason, detail } => {
            bail!("hook reported error: {reason} ({detail})");
        }
    };

    // Shutdown cleanly (idempotent; Drop also handles this but explicit
    // call lets us surface errors).
    child.shutdown().ok();

    render_payload_for_stage(hook.stage, payload)
}

fn payload_for_input(stage: Stage, input: &str) -> Result<Value> {
    match stage {
        Stage::PreParse => Ok(Value::String(input.to_string())),
        Stage::Transform => {
            let doc = parse_markdown(input);
            serde_json::to_value(&doc).context("serialising AST for transform stage")
        }
        Stage::PostRender => Ok(Value::String(render_markdown_to_html(input))),
    }
}

fn render_payload_for_stage(stage: Stage, payload: Value) -> Result<String> {
    match stage {
        Stage::PreParse | Stage::PostRender => match payload {
            Value::String(s) => Ok(s),
            other => bail!(
                "expected string payload from {} hook, got: {}",
                stage.as_str(),
                other
            ),
        },
        Stage::Transform => Ok(serde_json::to_string_pretty(&payload)? + "\n"),
    }
}

/// Render a minimal unified-style line diff of `expected` vs `actual`.
/// Intentionally tiny — no external crate — because the diff is purely
/// for human iteration, not machine parsing.
pub fn render_line_diff(expected: &str, actual: &str) -> String {
    let mut out = String::new();
    out.push_str("--- expected\n");
    out.push_str("+++ actual\n");
    let a: Vec<&str> = expected.split_inclusive('\n').collect();
    let b: Vec<&str> = actual.split_inclusive('\n').collect();
    // Walk in parallel; on first divergence dump remainder. A proper LCS
    // diff would be nicer but TEST-3225 only requires "diff shown".
    let mut diverged = false;
    let common = a.len().min(b.len());
    for i in 0..common {
        if a[i] == b[i] {
            if !diverged {
                // Show a few lines of context before the first diverge.
                out.push_str("  ");
                out.push_str(a[i]);
                if !a[i].ends_with('\n') {
                    out.push('\n');
                }
            }
        } else {
            diverged = true;
            out.push('-');
            out.push(' ');
            out.push_str(a[i]);
            if !a[i].ends_with('\n') {
                out.push('\n');
            }
            out.push('+');
            out.push(' ');
            out.push_str(b[i]);
            if !b[i].ends_with('\n') {
                out.push('\n');
            }
        }
    }
    for line in a.iter().skip(common) {
        out.push('-');
        out.push(' ');
        out.push_str(line);
        if !line.ends_with('\n') {
            out.push('\n');
        }
    }
    for line in b.iter().skip(common) {
        out.push('+');
        out.push(' ');
        out.push_str(line);
        if !line.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

// ── Fixture capture (`zetl hook fixture`) ─────────────────────────────────

/// Outcome of [`capture_fixture`].
pub struct CapturedFixture {
    pub input_path: PathBuf,
    pub expected_path: PathBuf,
}

/// Copy a vault page into the fixture directory and seed the golden from
/// the hook's current output. SPEC-032 REQ-3225 `zetl hook fixture`.
pub fn capture_fixture(
    vault_root: &Path,
    page: &str,
    hook_name: &str,
) -> Result<CapturedFixture> {
    let hook = find_scaffolded_hook(vault_root, hook_name)?;

    let page_path = resolve_page_path(vault_root, page)?;
    let content = std::fs::read_to_string(&page_path)
        .with_context(|| format!("reading vault page {}", page_path.display()))?;

    let fixture_dir = vault_root.join(FIXTURES_DIR).join(hook_name);
    std::fs::create_dir_all(&fixture_dir)
        .with_context(|| format!("creating fixture dir {}", fixture_dir.display()))?;
    let input_path = fixture_dir.join("input.md");
    std::fs::write(&input_path, &content)
        .with_context(|| format!("writing fixture input {}", input_path.display()))?;

    let expected = run_hook_once(&hook, &content)?;
    let expected_path = fixture_dir.join(golden_filename(hook.stage));
    std::fs::write(&expected_path, &expected)
        .with_context(|| format!("writing fixture golden {}", expected_path.display()))?;

    Ok(CapturedFixture {
        input_path,
        expected_path,
    })
}

fn resolve_page_path(vault_root: &Path, page: &str) -> Result<PathBuf> {
    // Accept three spellings: exact path, stem (add `.md`), or vault-relative.
    let candidates = [
        vault_root.join(page),
        vault_root.join(format!("{page}.md")),
        PathBuf::from(page),
    ];
    for candidate in candidates {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(anyhow!(
        "page '{page}' not found under {}",
        vault_root.display()
    ))
}

// ── Watch loop (`zetl hook watch`) ────────────────────────────────────────

/// Signal yielded to the caller by [`watch`] on each notable event. Used
/// by the CLI to print human-readable status lines; tests consume the
/// stream directly.
#[derive(Debug, PartialEq, Eq)]
pub enum WatchEvent {
    /// Hook process spawned (initial or post-restart).
    Spawned,
    /// File change detected; subprocess is being restarted.
    Restart,
    /// Stderr captured from the hook — free-form string.
    Stderr(String),
    /// Watcher is shutting down (Ctrl-C or channel closed).
    Shutdown,
}

/// Control knobs for [`watch`] — exposed so tests can override the
/// debounce / iteration count without waiting on real wall-clock time.
pub struct WatchOptions {
    /// Minimum gap between file-change events that should coalesce into a
    /// single restart. Defaults to 100 ms — faster than a human edit
    /// burst but slower than a file-system race.
    pub debounce: Duration,
    /// Maximum iterations before the loop exits on its own. `None`
    /// means "run until Ctrl-C"; `Some(n)` is used exclusively by tests
    /// for TEST-3225's "restart within 500 ms" assertion.
    pub max_events: Option<usize>,
}

impl Default for WatchOptions {
    fn default() -> Self {
        Self {
            debounce: Duration::from_millis(100),
            max_events: None,
        }
    }
}

/// Watch the hook source and restart the persistent child on change.
///
/// `on_event` is called synchronously on the watcher's thread for every
/// [`WatchEvent`]; the CLI uses this to stream status lines to the
/// terminal, tests use it to observe restart timing. Returns when the
/// file watcher shuts down (Ctrl-C on the CLI, `max_events` exhausted
/// in tests) or an unrecoverable error surfaces.
pub fn watch<F>(vault_root: &Path, hook_name: &str, opts: WatchOptions, mut on_event: F) -> Result<()>
where
    F: FnMut(&WatchEvent),
{
    use notify::{RecursiveMode, Watcher};
    use std::sync::mpsc as stdmpsc;

    let hook = find_scaffolded_hook(vault_root, hook_name)?;

    let (tx, rx) = stdmpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })
    .context("constructing file watcher")?;
    watcher
        .watch(&hook.path, RecursiveMode::NonRecursive)
        .with_context(|| format!("watching {}", hook.path.display()))?;

    let mut child = spawn_watched_hook(&hook)?;
    on_event(&WatchEvent::Spawned);

    let mut events_seen = 0usize;
    let mut last_restart = std::time::Instant::now()
        .checked_sub(opts.debounce)
        .unwrap_or_else(std::time::Instant::now);

    loop {
        if let Some(max) = opts.max_events {
            if events_seen >= max {
                break;
            }
        }

        // Bounded recv so we can also flush stderr periodically without
        // blocking on the watcher forever. 100 ms is small enough to
        // feel responsive, large enough to avoid burning CPU.
        let event = rx.recv_timeout(Duration::from_millis(100));

        // Flush any stderr the child produced since the last tick.
        let stderr = child.drain_stderr();
        if !stderr.is_empty() {
            on_event(&WatchEvent::Stderr(stderr));
        }

        match event {
            Ok(Ok(ev)) => {
                if !matches!(
                    ev.kind,
                    notify::EventKind::Modify(_)
                        | notify::EventKind::Create(_)
                        | notify::EventKind::Remove(_)
                ) {
                    continue;
                }
                if last_restart.elapsed() < opts.debounce {
                    continue;
                }
                last_restart = std::time::Instant::now();
                events_seen += 1;
                on_event(&WatchEvent::Restart);
                // Drop old child first so it exits before we respawn.
                drop(child);
                child = match spawn_watched_hook(&hook) {
                    Ok(c) => c,
                    Err(e) => {
                        on_event(&WatchEvent::Stderr(format!(
                            "[zetl] respawn failed: {e}\n"
                        )));
                        // Keep looping — the author may fix the error.
                        // A fresh Persistent process is required for
                        // further events, so spawn a placeholder that
                        // simply dies immediately. We just abort here
                        // instead: cleaner semantics, clearer error.
                        break;
                    }
                };
                on_event(&WatchEvent::Spawned);
            }
            Ok(Err(e)) => {
                on_event(&WatchEvent::Stderr(format!(
                    "[zetl] watcher error: {e}\n"
                )));
            }
            Err(stdmpsc::RecvTimeoutError::Timeout) => continue,
            Err(stdmpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    on_event(&WatchEvent::Shutdown);
    Ok(())
}

fn spawn_watched_hook(hook: &ScaffoldedHook) -> Result<PersistentHook> {
    let mut cmd = Command::new(&hook.path);
    cmd.current_dir(hook.path.parent().unwrap_or(Path::new(".")));
    PersistentHook::spawn(cmd, hook.name.clone(), hook.stage)
        .with_context(|| format!("spawning persistent hook {}", hook.path.display()))
}

// ── Unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_pipeline_conversion_roundtrips() {
        for s in [
            AuthoringStage::PreParse,
            AuthoringStage::Transform,
            AuthoringStage::PostRender,
        ] {
            let round = stage_from_pipeline(stage_to_pipeline(&s));
            assert_eq!(round, s);
        }
    }

    #[test]
    fn golden_filenames_are_stage_appropriate() {
        assert_eq!(golden_filename(Stage::PreParse), "expected.md");
        assert_eq!(golden_filename(Stage::Transform), "expected.json");
        assert_eq!(golden_filename(Stage::PostRender), "expected.html");
    }

    #[test]
    fn validate_name_accepts_reasonable_ids() {
        validate_name("callouts").unwrap();
        validate_name("my_hook-2").unwrap();
    }

    #[test]
    fn validate_name_rejects_path_separators() {
        assert!(validate_name("foo/bar").is_err());
        assert!(validate_name("foo bar").is_err());
        assert!(validate_name("").is_err());
    }

    #[test]
    fn identity_pre_parse_matches_input() {
        let input = "hello\n";
        let out = identity_output_for_stage(Stage::PreParse, input).unwrap();
        assert_eq!(out, input);
    }

    #[test]
    fn identity_transform_produces_valid_ast_json() {
        let out = identity_output_for_stage(Stage::Transform, "# Hi\n\ntext\n").unwrap();
        let v: Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(v["type"], "Document");
        assert_eq!(v["ast_version"], "1.0");
    }

    #[test]
    fn identity_post_render_emits_html() {
        let out = identity_output_for_stage(Stage::PostRender, "# Hi\n").unwrap();
        assert!(out.contains("<h1"));
    }

    #[test]
    fn render_line_diff_shows_plus_minus_on_divergence() {
        let diff = render_line_diff("a\nb\n", "a\nc\n");
        assert!(diff.contains("--- expected"));
        assert!(diff.contains("+++ actual"));
        assert!(diff.contains("- b"));
        assert!(diff.contains("+ c"));
    }
}
