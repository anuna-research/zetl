//! Repo-local dev tasks.
//!
//! # `cargo xtask update-golden [<fixture>]`
//!
//! Regenerate `expected.html` for every canonical-extension fixture under
//! `tests/extension-fixtures/` (SPEC-032 CON-3212). Pass a fixture name to
//! scope the update to a single directory.
//!
//! The runner set here **must** stay identical to the one the integration
//! test uses — otherwise `update-golden` could write expecteds the gate
//! would immediately reject. They live in
//! [`runners`] and are exported for test-side consumption too.
//!
//! Invoked via the workspace's `.cargo/config.toml` alias so
//! `cargo xtask <cmd>` is the canonical spelling.

use std::env;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};

mod runners;

fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let cmd = args
        .next()
        .ok_or_else(|| anyhow!("usage: cargo xtask <command>\n  commands: update-golden"))?;

    match cmd.as_str() {
        "update-golden" => {
            let target = args.next();
            update_golden(target.as_deref())
        }
        other => bail!("unknown xtask command: {other}\n  commands: update-golden"),
    }
}

fn update_golden(only: Option<&str>) -> Result<()> {
    let root = repo_root()?;
    let fixtures_dir = root.join("tests/extension-fixtures");
    let fixtures = zetl::extensions::golden::load_fixtures(&fixtures_dir)
        .with_context(|| format!("loading fixtures under {}", fixtures_dir.display()))?;

    if fixtures.is_empty() {
        eprintln!(
            "xtask update-golden: no fixtures under {}; nothing to do.",
            fixtures_dir.display()
        );
        return Ok(());
    }

    let mut updated = 0usize;
    let mut skipped = 0usize;
    let mut unmatched: Vec<String> = Vec::new();
    for fx in &fixtures {
        if let Some(name) = only {
            if fx.name != name {
                continue;
            }
        }
        let runner = runners::resolve(&fx.name).ok_or_else(|| {
            anyhow!(
                "fixture '{}' has no registered runner — add it to tools/xtask/src/runners.rs",
                fx.name
            )
        })?;
        let input = fx
            .read_input()
            .with_context(|| format!("reading input.md for fixture '{}'", fx.name))?;
        let html = runner(&input);
        let previous = fx.read_expected().unwrap_or(None);
        if previous.as_deref() == Some(&html) {
            skipped += 1;
            println!("unchanged: {}", fx.name);
        } else {
            fx.write_expected(&html)
                .with_context(|| format!("writing expected.html for fixture '{}'", fx.name))?;
            updated += 1;
            println!("updated:   {}", fx.name);
        }
    }

    if let Some(name) = only {
        if !fixtures.iter().any(|f| f.name == name) {
            unmatched.push(name.to_string());
        }
    }
    if !unmatched.is_empty() {
        bail!(
            "update-golden: no fixture matched {:?} under {}",
            unmatched,
            fixtures_dir.display()
        );
    }

    eprintln!(
        "xtask update-golden: {updated} updated, {skipped} unchanged (out of {} fixture{}).",
        fixtures.len(),
        if fixtures.len() == 1 { "" } else { "s" }
    );
    Ok(())
}

fn repo_root() -> Result<PathBuf> {
    let start = env::current_dir().context("getting current dir")?;
    let mut cur: &Path = &start;
    loop {
        if cur.join("Cargo.toml").is_file() && cur.join("tests/extension-fixtures").is_dir() {
            return Ok(cur.to_path_buf());
        }
        match cur.parent() {
            Some(p) => cur = p,
            None => bail!(
                "could not locate repo root from {}; run xtask from within the repo",
                start.display()
            ),
        }
    }
}
