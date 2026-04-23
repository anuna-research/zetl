//! SPEC-032 REQ-3223 — `ztl build --safe-mode` end-to-end coverage.
//!
//! Exercises the CLI surface: a theme that ships hooks emits the
//! "ships <N> undeclared hook(s)" warning by default and the
//! `--safe-mode skipped <stage>/<id> from <source>` line under
//! `--safe-mode`. Library-side filter logic is unit-tested in
//! `src/hooks/safe_mode.rs`.

#![cfg(unix)]

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;

/// Drop a runnable hook (shebang + 0o755) at the given relative path.
fn write_runnable(root: &std::path::Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(&p, body).unwrap();
    let mut perms = fs::metadata(&p).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&p, perms).unwrap();
}

/// Build a vault with a single page plus a disk-installed theme that
/// ships one declared transform hook (`callouts`) and one undeclared
/// transform hook (`rogue`). Returns the tempdir so the caller controls
/// its lifetime.
fn setup_vault_with_theme(declare_callouts: bool) -> TempDir {
    let dir = TempDir::new().unwrap();
    let vault = dir.path();

    // One trivial page so the build pipeline has work to do.
    fs::write(vault.join("hello.md"), "# Hello\n\nbody\n").unwrap();

    // Disk-installed theme. The `template/` and `static/` dirs aren't
    // required for the build path we exercise here, but `theme.toml`
    // and `hooks/transform.d/*` are.
    let theme_dir = vault.join(".ztl/themes/fountain");
    fs::create_dir_all(&theme_dir).unwrap();

    // theme.toml — declare callouts, omit rogue.
    let mut toml = String::from(
        "[theme]\n\
         name = \"fountain\"\n\
         version = \"1.0.0\"\n",
    );
    if declare_callouts {
        toml.push_str(
            "\n[[theme.hooks]]\n\
             stage = \"transform\"\n\
             extension_id = \"callouts\"\n\
             summary = \"Render block callouts.\"\n",
        );
    }
    fs::write(theme_dir.join("theme.toml"), toml).unwrap();

    // The pipeline build path also wants base templates from a real
    // theme. Reuse the bundled `default` templates by symlinking — if
    // the bundled-theme machinery resolves the on-disk theme first, an
    // empty `templates/` triggers a parse error. Easiest workaround:
    // copy a couple of minimal placeholders.
    let tpl = theme_dir.join("templates");
    fs::create_dir_all(&tpl).unwrap();
    fs::write(
        tpl.join("base.html"),
        "<!doctype html><html><body>{% block content %}{% endblock %}</body></html>",
    )
    .unwrap();
    fs::write(
        tpl.join("page.html"),
        "{% extends \"base.html\" %}{% block content %}{{ content | safe }}{% endblock %}",
    )
    .unwrap();
    fs::write(
        tpl.join("folder.html"),
        "{% extends \"base.html\" %}{% block content %}folder{% endblock %}",
    )
    .unwrap();
    fs::write(
        tpl.join("index.html"),
        "{% extends \"base.html\" %}{% block content %}index{% endblock %}",
    )
    .unwrap();

    // Two transform hooks under `<theme>/hooks/transform.d/`.
    write_runnable(
        &theme_dir,
        "hooks/transform.d/callouts.sh",
        "#!/bin/sh\nexit 0\n",
    );
    write_runnable(
        &theme_dir,
        "hooks/transform.d/rogue.sh",
        "#!/bin/sh\nexit 0\n",
    );

    dir
}

#[test]
fn safe_mode_skips_undeclared_theme_hook_with_log_line() {
    let dir = setup_vault_with_theme(true);
    let out = TempDir::new().unwrap();

    cargo_bin_cmd!("ztl")
        .args([
            "-d",
            dir.path().to_str().unwrap(),
            "build",
            "--theme",
            "fountain",
            "-o",
            out.path().to_str().unwrap(),
            "--safe-mode",
        ])
        .assert()
        .success()
        // Declared theme hook (callouts) is *not* skipped.
        .stderr(
            predicate::str::contains("[ztl] --safe-mode: skipped transform/callouts from theme")
                .not(),
        )
        // Undeclared theme hook (rogue) is skipped, with SPEC-shape line.
        .stderr(predicate::str::contains(
            "[ztl] --safe-mode: skipped transform/rogue from theme",
        ));
}

#[test]
fn safe_mode_with_no_declarations_skips_every_hook() {
    let dir = setup_vault_with_theme(false);
    let out = TempDir::new().unwrap();

    cargo_bin_cmd!("ztl")
        .args([
            "-d",
            dir.path().to_str().unwrap(),
            "build",
            "--theme",
            "fountain",
            "-o",
            out.path().to_str().unwrap(),
            "--safe-mode",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "[ztl] --safe-mode: skipped transform/callouts from theme",
        ))
        .stderr(predicate::str::contains(
            "[ztl] --safe-mode: skipped transform/rogue from theme",
        ));
}

#[test]
fn undeclared_warning_emitted_without_safe_mode() {
    let dir = setup_vault_with_theme(true);
    let out = TempDir::new().unwrap();

    cargo_bin_cmd!("ztl")
        .args([
            "-d",
            dir.path().to_str().unwrap(),
            "build",
            "--theme",
            "fountain",
            "-o",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        // SPEC-mandated warning shape.
        .stderr(predicate::str::contains(
            "[ztl] theme fountain ships 1 undeclared hook(s); run",
        ))
        .stderr(predicate::str::contains("'ztl theme show fountain'"))
        .stderr(predicate::str::contains("--safe-mode to suppress"));
}

#[test]
fn safe_mode_suppresses_undeclared_warning() {
    let dir = setup_vault_with_theme(true);
    let out = TempDir::new().unwrap();

    cargo_bin_cmd!("ztl")
        .args([
            "-d",
            dir.path().to_str().unwrap(),
            "build",
            "--theme",
            "fountain",
            "-o",
            out.path().to_str().unwrap(),
            "--safe-mode",
        ])
        .assert()
        .success()
        // The warning is suppressed under safe-mode (skip lines speak
        // for themselves).
        .stderr(predicate::str::contains("ships 1 undeclared hook(s)").not());
}

#[test]
fn safe_mode_vault_hook_is_always_skipped_even_if_id_matches_declaration() {
    let dir = setup_vault_with_theme(true);
    // Vault-side hook with the same extension_id ("callouts") as a
    // declared theme hook. SPEC-032 §10's threat model: vault code is
    // unaffected by the theme allow-list — it must still be skipped.
    write_runnable(
        dir.path(),
        ".ztl/hooks/transform.d/callouts.sh",
        "#!/bin/sh\nexit 0\n",
    );
    let out = TempDir::new().unwrap();

    cargo_bin_cmd!("ztl")
        .args([
            "-d",
            dir.path().to_str().unwrap(),
            "build",
            "--theme",
            "fountain",
            "-o",
            out.path().to_str().unwrap(),
            "--safe-mode",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "[ztl] --safe-mode: skipped transform/callouts from vault",
        ));
}
