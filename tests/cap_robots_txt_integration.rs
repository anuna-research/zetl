//! robots.txt emission — integration tests for SPEC-034 REQ-3418 /
//! CON-3406 ("general robots.txt REQ", per task-cap-robots-txt).
//!
//! Drives `zetl::web::build::build_static` end-to-end over a minimal vault
//! and asserts that the emitted `dist/robots.txt` always carries the
//! required `Disallow: /c/` + `Disallow: /_zetl/` rules while preserving
//! any operator-supplied `public/robots.txt` — including broader rules
//! like `Disallow: /` that cover our paths implicitly.

use std::fs;
use std::path::Path;

use tempfile::TempDir;

fn write(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn minimal_vault() -> TempDir {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "index.md", "# Home\n\nwelcome\n");
    tmp
}

fn build(vault: &Path, out: &Path, public: Option<&Path>) {
    let data =
        zetl::web::reindex_with(vault, &zetl::scanner::ScanOptions::default()).expect("reindex");
    zetl::web::build::build_static(
        &data,
        vault,
        out.to_str().unwrap(),
        "default",
        false,
        public.and_then(Path::to_str),
        None,
    )
    .expect("build_static");
}

#[test]
fn test_3414_default_build_emits_both_disallows() {
    let tmp = minimal_vault();
    let out = tmp.path().join("dist");
    build(tmp.path(), &out, None);

    let body = fs::read_to_string(out.join("robots.txt")).unwrap();
    assert!(
        body.contains("Disallow: /c/\n"),
        "missing Disallow: /c/ in {body:?}"
    );
    assert!(
        body.contains("Disallow: /_zetl/\n"),
        "missing Disallow: /_zetl/ in {body:?}"
    );
    // No operator content ⇒ the canonical single-block form.
    assert_eq!(
        body, "User-agent: *\nDisallow: /c/\nDisallow: /_zetl/\n",
        "unexpected default body: {body:?}"
    );
}

#[test]
fn test_3414_operator_robots_merged_verbatim() {
    let tmp = minimal_vault();
    let pub_dir = tmp.path().join("public");
    let operator = "User-agent: Googlebot\n\
                    Disallow: /private/\n\
                    \n\
                    Sitemap: https://example.com/sitemap.xml\n";
    write(&pub_dir, "robots.txt", operator);

    let out = tmp.path().join("dist");
    build(tmp.path(), &out, Some(&pub_dir));

    let body = fs::read_to_string(out.join("robots.txt")).unwrap();
    assert!(
        body.starts_with(operator),
        "operator block must lead, got: {body:?}"
    );
    assert!(
        body.contains("\nUser-agent: *\n"),
        "appended *-block not found in {body:?}"
    );
    assert!(body.contains("Disallow: /c/\n"));
    assert!(body.contains("Disallow: /_zetl/\n"));
    // Operator's private rule preserved.
    assert!(body.contains("Disallow: /private/\n"));
    assert!(body.contains("Sitemap: https://example.com/sitemap.xml\n"));
}

#[test]
fn test_3414_stricter_disallow_root_preserved() {
    // Operator locks down the whole site (e.g. staging mirror). Our
    // additions must never relax the rule: the broader `Disallow: /`
    // stays in place and the explicit per-subtree rules are appended
    // so audit tooling can still grep for them.
    let tmp = minimal_vault();
    let pub_dir = tmp.path().join("public");
    write(&pub_dir, "robots.txt", "User-agent: *\nDisallow: /\n");

    let out = tmp.path().join("dist");
    build(tmp.path(), &out, Some(&pub_dir));

    let body = fs::read_to_string(out.join("robots.txt")).unwrap();
    assert!(
        body.contains("Disallow: /\n"),
        "strict root disallow dropped: {body:?}"
    );
    assert!(body.contains("Disallow: /c/\n"));
    assert!(body.contains("Disallow: /_zetl/\n"));
    // Exactly one `Disallow: /` — we must never duplicate the operator's
    // rule when re-emitting.
    assert_eq!(
        body.lines()
            .filter(|l| l.trim() == "Disallow: /")
            .count(),
        1,
        "duplicated strict rule in {body:?}"
    );
}

#[test]
fn test_3414_operator_already_covers_required_no_duplicate() {
    let tmp = minimal_vault();
    let pub_dir = tmp.path().join("public");
    let operator = "User-agent: *\n\
                    Disallow: /c/\n\
                    Disallow: /_zetl/\n";
    write(&pub_dir, "robots.txt", operator);

    let out = tmp.path().join("dist");
    build(tmp.path(), &out, Some(&pub_dir));

    let body = fs::read_to_string(out.join("robots.txt")).unwrap();
    // No new `User-agent: *` block — operator's single declaration is
    // preserved unchanged.
    assert_eq!(
        body.matches("User-agent: *").count(),
        1,
        "expected exactly one UA block: {body:?}"
    );
    assert_eq!(body.matches("Disallow: /c/").count(), 1);
    assert_eq!(body.matches("Disallow: /_zetl/").count(), 1);
}
