//! Integration tests for LLM-agent discovery artefacts: `sitemap.xml`,
//! `llms.txt`, and per-page raw source files emitted by `ztl build`.

use std::fs;
use tempfile::TempDir;

fn write_file(root: &std::path::Path, relative: &str, content: &str) {
    let full = root.join(relative);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&full, content).unwrap();
}

fn ztl_build(vault: &std::path::Path, out_dir: &std::path::Path) {
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("ztl");
    cmd.arg("-d").arg(vault).arg("--no-cache");
    cmd.arg("build").arg("-o").arg(out_dir);
    let output = cmd.output().expect("run ztl build");
    assert!(
        output.status.success(),
        "ztl build failed.\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn build_emits_sitemap_xml_with_every_page() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "Alpha.md", "# Alpha\n[[Beta]]\n");
    write_file(dir.path(), "Beta.md", "# Beta\nbody\n");
    let out = dir.path().join("dist");
    ztl_build(dir.path(), &out);

    let sitemap = fs::read_to_string(out.join("sitemap.xml")).expect("sitemap.xml emitted");
    assert!(sitemap.starts_with("<?xml"));
    assert!(sitemap.contains("<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">"));
    assert!(sitemap.contains("<loc>/alpha/</loc>"));
    assert!(sitemap.contains("<loc>/beta/</loc>"));
    assert!(sitemap.contains("<loc>/</loc>"));
}

#[test]
fn build_emits_llms_txt_with_resource_list() {
    let dir = TempDir::new().unwrap();
    write_file(dir.path(), "Page.md", "# Page\nbody\n");
    let out = dir.path().join("dist");
    ztl_build(dir.path(), &out);

    let llms = fs::read_to_string(out.join("llms.txt")).expect("llms.txt emitted");
    // Starts with the vault name header.
    assert!(llms.starts_with("# "));
    assert!(llms.contains("## Resources"));
    // Build-mode variant documents the static indices, not /api/.
    assert!(llms.contains("/search-index.json"));
    assert!(llms.contains("/pages.json"));
    assert!(llms.contains("/graph-index.json"));
    assert!(llms.contains("/sitemap.xml"));
    assert!(!llms.contains("/api/"));
}

#[test]
fn build_emits_raw_markdown_per_page() {
    let dir = TempDir::new().unwrap();
    let body = "# Alpha\n\nThis is the raw source.\n\n[[Beta]]\n";
    write_file(dir.path(), "Alpha.md", body);
    write_file(dir.path(), "Beta.md", "# Beta\n");
    let out = dir.path().join("dist");
    ztl_build(dir.path(), &out);

    // Raw source sits next to the rendered HTML, same URL shape.
    let md = fs::read_to_string(out.join("alpha/index.md")).expect("alpha/index.md emitted");
    assert_eq!(md, body, "raw md should be byte-identical to source");
    assert!(out.join("alpha/index.html").exists());
    assert!(out.join("beta/index.md").exists());
}
