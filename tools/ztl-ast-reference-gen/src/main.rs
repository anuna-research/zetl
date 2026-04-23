//! Generate `docs/ztl-ast-reference.md` from `tools/ztl-ast-schema-v1.json`.
//!
//! SPEC-032 REQ-3202: plugin authors should not have to read a JSON Schema
//! file to learn the AST. The schema is the source of truth for structure;
//! this generator produces the human-readable companion and CI fails on
//! any uncommitted diff between the two.
//!
//! Canonical examples and HTML-rendering expectations are maintained here
//! rather than in the schema — JSON Schema has no first-class slot for
//! either, and inlining prose examples would muddy the machine-readable
//! contract.
//!
//! Usage:
//!   cargo run -p ztl-ast-reference-gen            # write docs/ztl-ast-reference.md
//!   cargo run -p ztl-ast-reference-gen -- --check # exit nonzero on diff

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const SCHEMA_PATH: &str = "tools/ztl-ast-schema-v1.json";
const OUT_PATH: &str = "docs/ztl-ast-reference.md";

fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    let check_only = args.iter().any(|a| a == "--check");

    let repo_root = find_repo_root()?;
    let schema_path = repo_root.join(SCHEMA_PATH);
    let out_path = repo_root.join(OUT_PATH);

    let schema_src = fs::read_to_string(&schema_path)
        .with_context(|| format!("reading schema at {}", schema_path.display()))?;
    let schema: Value = serde_json::from_str(&schema_src)
        .with_context(|| format!("parsing schema at {}", schema_path.display()))?;

    let rendered = render(&schema)?;

    if check_only {
        let on_disk = fs::read_to_string(&out_path)
            .with_context(|| format!("reading {} for --check", out_path.display()))?;
        if on_disk != rendered {
            eprintln!(
                "ztl-ast-reference-gen: {} is stale; re-run without --check to regenerate.",
                out_path.display()
            );
            std::process::exit(1);
        }
        println!(
            "ztl-ast-reference-gen: {} is up to date.",
            out_path.display()
        );
    } else {
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        fs::write(&out_path, rendered)
            .with_context(|| format!("writing {}", out_path.display()))?;
        println!("ztl-ast-reference-gen: wrote {}", out_path.display());
    }
    Ok(())
}

/// Walk upward from the executable's cwd until we hit the directory that
/// contains both `Cargo.toml` and `tools/ztl-ast-schema-v1.json`. Cargo
/// sets `CARGO_MANIFEST_DIR` to the package's own manifest dir when run
/// via `cargo run`, which is the tool subdir, not the repo root — so we
/// must ascend explicitly.
fn find_repo_root() -> Result<PathBuf> {
    let start = env::current_dir().context("getting current dir")?;
    let mut cur: &Path = &start;
    loop {
        if cur.join("tools/ztl-ast-schema-v1.json").is_file() && cur.join("Cargo.toml").is_file() {
            return Ok(cur.to_path_buf());
        }
        match cur.parent() {
            Some(p) => cur = p,
            None => bail!(
                "could not locate repo root (looking for tools/ztl-ast-schema-v1.json) starting from {}",
                start.display()
            ),
        }
    }
}

// Fixed section order. The schema's $defs are unordered; the reference
// doc must be byte-stable so the CI diff gate is meaningful.
const NODE_ORDER: &[(&str, Section)] = &[
    ("Document", Section::Root),
    ("Heading", Section::Block),
    ("Paragraph", Section::Block),
    ("BlockQuote", Section::Block),
    ("List", Section::Block),
    ("ListItem", Section::Block),
    ("CodeBlock", Section::Block),
    ("ThematicBreak", Section::Block),
    ("HtmlBlock", Section::Block),
    ("SplBlock", Section::Block),
    ("Embed", Section::Block),
    ("Text", Section::Inline),
    ("Emphasis", Section::Inline),
    ("Strong", Section::Inline),
    ("Code", Section::Inline),
    ("Link", Section::Inline),
    ("Image", Section::Inline),
    ("LineBreak", Section::Inline),
    ("SoftBreak", Section::Inline),
    ("HtmlInline", Section::Inline),
    ("Wikilink", Section::Inline),
];

#[derive(Copy, Clone, PartialEq, Eq)]
enum Section {
    Root,
    Block,
    Inline,
}

fn render(schema: &Value) -> Result<String> {
    let title = schema
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("ztl-ext AST");
    let schema_desc = schema
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let defs = schema
        .get("$defs")
        .and_then(|v| v.as_object())
        .ok_or_else(|| anyhow!("schema has no $defs"))?;

    let mut out = String::new();

    // Preamble. The generator comment is load-bearing: reviewers on a
    // stale PR will see it and know the file is generated.
    out.push_str("<!--\n");
    out.push_str("  AUTO-GENERATED by tools/ztl-ast-reference-gen from\n");
    out.push_str("  tools/ztl-ast-schema-v1.json. Do not hand-edit; regenerate with:\n");
    out.push_str("      cargo run -p ztl-ast-reference-gen\n");
    out.push_str("  CI will fail if this file drifts from the schema (SPEC-032 REQ-3202).\n");
    out.push_str("-->\n\n");

    out.push_str(&format!("# {title}\n\n"));
    if !schema_desc.is_empty() {
        out.push_str(schema_desc);
        out.push_str("\n\n");
    }

    // Quick jump index, grouped by level. Useful for plugin authors
    // hunting for a specific node.
    out.push_str("## Contents\n\n");
    out.push_str("- [Document root](#document)\n");
    out.push_str("- **Block-level nodes**\n");
    for (name, sec) in NODE_ORDER {
        if *sec == Section::Block {
            out.push_str(&format!("  - [{n}](#{a})\n", n = name, a = anchor(name)));
        }
    }
    out.push_str("- **Inline-level nodes**\n");
    for (name, sec) in NODE_ORDER {
        if *sec == Section::Inline {
            out.push_str(&format!("  - [{n}](#{a})\n", n = name, a = anchor(name)));
        }
    }
    out.push_str("- [Shared definitions](#shared-definitions)\n\n");

    // Shared definitions: position, ast_version, frontmatter. Every
    // node carries a position, so documenting it once up front saves
    // repeating the field table 20 times.
    render_shared(&mut out, defs)?;

    // Per-node sections.
    for (name, section) in NODE_ORDER {
        let def = defs
            .get(*name)
            .ok_or_else(|| anyhow!("schema missing $def for {name}"))?;
        render_node(&mut out, name, *section, def)?;
    }

    Ok(out)
}

fn render_shared(out: &mut String, defs: &serde_json::Map<String, Value>) -> Result<()> {
    out.push_str("## Shared definitions\n\n");

    out.push_str("### `ast_version`\n\n");
    if let Some(d) = defs
        .get("ast_version")
        .and_then(|v| v.get("description"))
        .and_then(|v| v.as_str())
    {
        out.push_str(d);
        out.push_str("\n\n");
    }
    if let Some(pat) = defs
        .get("ast_version")
        .and_then(|v| v.get("pattern"))
        .and_then(|v| v.as_str())
    {
        out.push_str(&format!("Pattern: `{pat}`\n\n"));
    }
    out.push_str("Example: `\"1.0\"`\n\n");

    out.push_str("### `position`\n\n");
    if let Some(d) = defs
        .get("position")
        .and_then(|v| v.get("description"))
        .and_then(|v| v.as_str())
    {
        out.push_str(d);
        out.push_str("\n\n");
    }
    out.push_str("| Field | Type | Notes |\n");
    out.push_str("| --- | --- | --- |\n");
    out.push_str("| `start_line` | integer ≥ 1 | 1-indexed, inclusive |\n");
    out.push_str("| `start_col` | integer ≥ 1 | 1-indexed, inclusive |\n");
    out.push_str("| `end_line` | integer ≥ 1 | 1-indexed, inclusive |\n");
    out.push_str("| `end_col` | integer ≥ 1 | 1-indexed, inclusive |\n\n");
    out.push_str(
        "Every node carries a `position` object; the field tables below omit it for brevity.\n\n",
    );

    out.push_str("### `frontmatter`\n\n");
    if let Some(d) = defs
        .get("frontmatter")
        .and_then(|v| v.get("description"))
        .and_then(|v| v.as_str())
    {
        out.push_str(d);
        out.push_str("\n\n");
    }
    out.push_str("Attached to the `Document` root as an arbitrary JSON object (parsed from YAML). Absent when the source has no frontmatter.\n\n");

    Ok(())
}

fn render_node(out: &mut String, name: &str, section: Section, def: &Value) -> Result<()> {
    let section_label = match section {
        Section::Root => "Root",
        Section::Block => "Block",
        Section::Inline => "Inline",
    };

    out.push_str(&format!("## {name}\n\n"));
    out.push_str(&format!("*{section_label}-level.*\n\n"));

    if let Some(desc) = def.get("description").and_then(|v| v.as_str()) {
        out.push_str(desc);
        out.push_str("\n\n");
    } else {
        // Fall back to a minimal one-liner so every section has prose
        // above the field table. Keeps the reference uniform.
        out.push_str(&format!("CommonMark `{name}` node.\n\n"));
    }

    render_fields(out, def)?;

    out.push_str("**Canonical example**\n\n");
    out.push_str("```json\n");
    out.push_str(canonical_example(name));
    out.push_str("\n```\n\n");

    out.push_str("**HTML rendering expectation**\n\n");
    out.push_str(html_expectation(name));
    out.push_str("\n\n");

    Ok(())
}

fn render_fields(out: &mut String, def: &Value) -> Result<()> {
    let props = def
        .get("properties")
        .and_then(|v| v.as_object())
        .ok_or_else(|| anyhow!("node def has no properties"))?;
    let required: Vec<&str> = def
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    // Canonical key order: `type` first, `position` next (omitted
    // below because it's documented globally), then schema-file order
    // for the rest. serde_json preserves insertion order from the
    // parsed file, which matches the source ordering in the schema.
    let mut ordered_keys: Vec<&String> = Vec::new();
    if props.contains_key("type") {
        // Skip — rendered via the heading and shown in the example.
    }
    if props.contains_key("position") {
        // Skip — shared definition.
    }
    for (k, _) in props {
        if k == "type" || k == "position" {
            continue;
        }
        ordered_keys.push(k);
    }
    // Stable order: schema iteration order. serde_json::Map preserves
    // insertion order with the default `preserve_order` off; but the
    // default is a BTreeMap-like (alphabetical) ordering. Sort
    // alphabetically to guarantee determinism across serde_json
    // feature flags.
    ordered_keys.sort();

    if ordered_keys.is_empty() {
        out.push_str(&format!(
            "No fields beyond `type` + `position`. Discriminator: `\"type\": \"{}\"`.\n\n",
            def.get("properties")
                .and_then(|p| p.get("type"))
                .and_then(|t| t.get("const"))
                .and_then(|c| c.as_str())
                .unwrap_or("?")
        ));
        return Ok(());
    }

    out.push_str("| Field | Type | Required | Notes |\n");
    out.push_str("| --- | --- | --- | --- |\n");
    for key in ordered_keys {
        let schema = &props[key];
        let ty = render_type(schema);
        let req = if required.iter().any(|r| r == key) {
            "yes"
        } else {
            "no"
        };
        let notes = field_notes(key, schema);
        out.push_str(&format!("| `{key}` | {ty} | {req} | {notes} |\n"));
    }
    out.push('\n');
    Ok(())
}

fn render_type(schema: &Value) -> String {
    if let Some(c) = schema.get("const").and_then(|v| v.as_str()) {
        return format!("`\"{c}\"` (const)");
    }
    if let Some(r) = schema.get("$ref").and_then(|v| v.as_str()) {
        // `#/$defs/Name` → `Name`
        let name = r.rsplit('/').next().unwrap_or(r);
        return format!("[`{}`](#{})", name, anchor_for_ref(name));
    }
    if let Some(arr) = schema.get("type").and_then(|v| v.as_array()) {
        let parts: Vec<String> = arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        return parts.join(" \\| ");
    }
    if let Some(t) = schema.get("type").and_then(|v| v.as_str()) {
        if t == "array" {
            let items = schema.get("items");
            if let Some(items) = items {
                return format!("array of {}", render_type(items));
            }
            return "array".into();
        }
        return t.into();
    }
    "?".into()
}

fn field_notes(key: &str, schema: &Value) -> String {
    let mut bits: Vec<String> = Vec::new();
    if let Some(min) = schema.get("minimum").and_then(|v| v.as_i64()) {
        bits.push(format!("min {min}"));
    }
    if let Some(max) = schema.get("maximum").and_then(|v| v.as_i64()) {
        bits.push(format!("max {max}"));
    }
    // `[string, null]` unions: the schema uses `"type": ["string", "null"]`
    // for optional-as-null fields. Call that out in prose rather than
    // repeating `string | null` in the type column — authors need to
    // know that `null` is the absent value, not the string "null".
    let key_specific_null_note = matches!(key, "start");
    if let Some(arr) = schema.get("type").and_then(|v| v.as_array()) {
        let has_null = arr.iter().any(|v| v.as_str() == Some("null"));
        if has_null && !key_specific_null_note {
            bits.push("`null` when absent".into());
        }
    }
    if key == "start" {
        // List.start. The schema permits integer-or-null; the prose
        // should match the List description rather than the generic
        // "null when absent" line used elsewhere.
        bits.push("`null` for unordered lists".into());
    }
    if bits.is_empty() {
        String::new()
    } else {
        bits.join("; ")
    }
}

fn anchor(name: &str) -> String {
    name.to_ascii_lowercase()
}

fn anchor_for_ref(name: &str) -> String {
    // Inline/block/node refs collapse to "shared-definitions"; specific
    // node refs link to the node's section.
    match name {
        "position" | "ast_version" | "frontmatter" | "node" | "block" | "inline" => {
            "shared-definitions".into()
        }
        other => other.to_ascii_lowercase(),
    }
}

/// Canonical JSON example for each node type. Hand-maintained because
/// the JSON Schema intentionally doesn't carry example payloads —
/// examples belong to the prose contract, not the structural one.
fn canonical_example(name: &str) -> &'static str {
    match name {
        "Document" => {
            r#"{
  "ast_version": "1.0",
  "type": "Document",
  "position": { "start_line": 1, "start_col": 1, "end_line": 3, "end_col": 1 },
  "frontmatter": { "title": "Hello" },
  "children": [
    {
      "type": "Paragraph",
      "position": { "start_line": 3, "start_col": 1, "end_line": 3, "end_col": 6 },
      "children": [
        { "type": "Text", "position": { "start_line": 3, "start_col": 1, "end_line": 3, "end_col": 6 }, "text": "Hello" }
      ]
    }
  ]
}"#
        }
        "Heading" => {
            r#"{
  "type": "Heading",
  "position": { "start_line": 1, "start_col": 1, "end_line": 1, "end_col": 10 },
  "level": 2,
  "children": [
    { "type": "Text", "position": { "start_line": 1, "start_col": 4, "end_line": 1, "end_col": 10 }, "text": "Hello." }
  ]
}"#
        }
        "Paragraph" => {
            r#"{
  "type": "Paragraph",
  "position": { "start_line": 1, "start_col": 1, "end_line": 1, "end_col": 12 },
  "children": [
    { "type": "Text", "position": { "start_line": 1, "start_col": 1, "end_line": 1, "end_col": 12 }, "text": "Hello world." }
  ]
}"#
        }
        "BlockQuote" => {
            r#"{
  "type": "BlockQuote",
  "position": { "start_line": 1, "start_col": 1, "end_line": 1, "end_col": 8 },
  "children": [
    {
      "type": "Paragraph",
      "position": { "start_line": 1, "start_col": 3, "end_line": 1, "end_col": 8 },
      "children": [
        { "type": "Text", "position": { "start_line": 1, "start_col": 3, "end_line": 1, "end_col": 8 }, "text": "quoted" }
      ]
    }
  ]
}"#
        }
        "List" => {
            r#"{
  "type": "List",
  "position": { "start_line": 1, "start_col": 1, "end_line": 2, "end_col": 5 },
  "ordered": false,
  "tight": true,
  "start": null,
  "children": [
    {
      "type": "ListItem",
      "position": { "start_line": 1, "start_col": 1, "end_line": 1, "end_col": 5 },
      "children": [
        {
          "type": "Paragraph",
          "position": { "start_line": 1, "start_col": 3, "end_line": 1, "end_col": 5 },
          "children": [
            { "type": "Text", "position": { "start_line": 1, "start_col": 3, "end_line": 1, "end_col": 5 }, "text": "one" }
          ]
        }
      ]
    }
  ]
}"#
        }
        "ListItem" => {
            r#"{
  "type": "ListItem",
  "position": { "start_line": 1, "start_col": 1, "end_line": 1, "end_col": 5 },
  "children": [
    {
      "type": "Paragraph",
      "position": { "start_line": 1, "start_col": 3, "end_line": 1, "end_col": 5 },
      "children": [
        { "type": "Text", "position": { "start_line": 1, "start_col": 3, "end_line": 1, "end_col": 5 }, "text": "one" }
      ]
    }
  ]
}"#
        }
        "CodeBlock" => {
            r#"{
  "type": "CodeBlock",
  "position": { "start_line": 1, "start_col": 1, "end_line": 3, "end_col": 4 },
  "fenced": true,
  "lang": "rust",
  "info": "rust",
  "text": "fn main() {}\n"
}"#
        }
        "ThematicBreak" => {
            r#"{
  "type": "ThematicBreak",
  "position": { "start_line": 1, "start_col": 1, "end_line": 1, "end_col": 4 }
}"#
        }
        "HtmlBlock" => {
            r#"{
  "type": "HtmlBlock",
  "position": { "start_line": 1, "start_col": 1, "end_line": 1, "end_col": 20 },
  "text": "<div class=\"note\">hi</div>"
}"#
        }
        "SplBlock" => {
            r#"{
  "type": "SplBlock",
  "position": { "start_line": 1, "start_col": 1, "end_line": 3, "end_col": 4 },
  "info": "spl",
  "text": "(task learn-rust (status todo))"
}"#
        }
        "Embed" => {
            r#"{
  "type": "Embed",
  "position": { "start_line": 1, "start_col": 1, "end_line": 1, "end_col": 16 },
  "target": "Alpha",
  "heading": null,
  "block_id": null
}"#
        }
        "Text" => {
            r#"{
  "type": "Text",
  "position": { "start_line": 1, "start_col": 1, "end_line": 1, "end_col": 6 },
  "text": "Hello"
}"#
        }
        "Emphasis" => {
            r#"{
  "type": "Emphasis",
  "position": { "start_line": 1, "start_col": 1, "end_line": 1, "end_col": 7 },
  "children": [
    { "type": "Text", "position": { "start_line": 1, "start_col": 2, "end_line": 1, "end_col": 6 }, "text": "hi" }
  ]
}"#
        }
        "Strong" => {
            r#"{
  "type": "Strong",
  "position": { "start_line": 1, "start_col": 1, "end_line": 1, "end_col": 7 },
  "children": [
    { "type": "Text", "position": { "start_line": 1, "start_col": 3, "end_line": 1, "end_col": 5 }, "text": "hi" }
  ]
}"#
        }
        "Code" => {
            r#"{
  "type": "Code",
  "position": { "start_line": 1, "start_col": 1, "end_line": 1, "end_col": 7 },
  "text": "println"
}"#
        }
        "Link" => {
            r#"{
  "type": "Link",
  "position": { "start_line": 1, "start_col": 1, "end_line": 1, "end_col": 30 },
  "url": "https://example.com",
  "title": null,
  "children": [
    { "type": "Text", "position": { "start_line": 1, "start_col": 2, "end_line": 1, "end_col": 8 }, "text": "site" }
  ]
}"#
        }
        "Image" => {
            r#"{
  "type": "Image",
  "position": { "start_line": 1, "start_col": 1, "end_line": 1, "end_col": 25 },
  "url": "cat.png",
  "title": "a cat",
  "alt": "a cat",
  "children": [
    { "type": "Text", "position": { "start_line": 1, "start_col": 3, "end_line": 1, "end_col": 8 }, "text": "a cat" }
  ]
}"#
        }
        "LineBreak" => {
            r#"{
  "type": "LineBreak",
  "position": { "start_line": 1, "start_col": 6, "end_line": 1, "end_col": 8 }
}"#
        }
        "SoftBreak" => {
            r#"{
  "type": "SoftBreak",
  "position": { "start_line": 1, "start_col": 6, "end_line": 1, "end_col": 7 }
}"#
        }
        "HtmlInline" => {
            r#"{
  "type": "HtmlInline",
  "position": { "start_line": 1, "start_col": 1, "end_line": 1, "end_col": 5 },
  "text": "<br>"
}"#
        }
        "Wikilink" => {
            r#"{
  "type": "Wikilink",
  "position": { "start_line": 1, "start_col": 1, "end_line": 1, "end_col": 19 },
  "target": "Alpha",
  "alias": "the alpha",
  "heading": "Intro",
  "block_id": null
}"#
        }
        _ => "// TODO: canonical example missing",
    }
}

/// HTML rendering expectation per node. This is the contract the
/// default renderer (and post-render hooks) are expected to honour;
/// theme CSS can restyle but the element shape should match.
fn html_expectation(name: &str) -> &'static str {
    match name {
        "Document" => "Renders as the body of the page — children are emitted in order. `frontmatter` is not rendered; it is exposed to templates via the `page.meta` surface.",
        "Heading" => "Renders as `<h{level}>…</h{level}>` (e.g. level 2 → `<h2>`). Inline children render inside the element. The default renderer may also emit `id` + an anchor link; that slug is derived from the heading text and is not part of the AST.",
        "Paragraph" => "Renders as `<p>…</p>` wrapping inline children.",
        "BlockQuote" => "Renders as `<blockquote>…</blockquote>` wrapping block children.",
        "List" => "Renders as `<ul>` when `ordered=false`, `<ol>` when `ordered=true`. When `ordered=true` and `start` differs from `1`, the renderer emits `start=\"{start}\"`. `tight=true` suppresses paragraph wrappers around single-paragraph items.",
        "ListItem" => "Renders as `<li>…</li>`. Inside a *tight* list, a single-`Paragraph` item renders its inline children directly without a `<p>` wrapper; inside a *loose* list, the `<p>` wrapper is preserved.",
        "CodeBlock" => "Renders as `<pre><code class=\"language-{lang}\">…</code></pre>` when `lang` is set, or `<pre><code>…</code></pre>` otherwise. `text` is HTML-escaped and emitted verbatim — the block preserves newlines.",
        "ThematicBreak" => "Renders as `<hr>`.",
        "HtmlBlock" => "Passes `text` through to the output verbatim. The renderer's sanitiser (if any) is the only transformation applied.",
        "SplBlock" => "By default renders as a fenced code block with `info=\"spl\"` — i.e. `<pre><code class=\"language-spl\">…</code></pre>`. Themes or transform hooks may replace this with structured output (e.g. an interactive task-list widget).",
        "Embed" => "Renders as the resolved transclusion: the target page's fragment is inlined at the embed position. When `heading` is set, only that heading's subtree inlines; when `block_id` is set, only the referenced block inlines. Unresolvable embeds render as a broken-link placeholder `<span class=\"ztl-embed-missing\">![[target]]</span>`.",
        "Text" => "Renders as the HTML-escaped literal `text`. No element wrapper.",
        "Emphasis" => "Renders as `<em>…</em>` wrapping inline children.",
        "Strong" => "Renders as `<strong>…</strong>` wrapping inline children.",
        "Code" => "Renders as `<code>…</code>` with HTML-escaped `text`.",
        "Link" => "Renders as `<a href=\"{url}\" title=\"{title}\">…</a>`, with `title` omitted when null. Inline children render inside the anchor.",
        "Image" => "Renders as `<img src=\"{url}\" alt=\"{alt}\" title=\"{title}\">`, with `title` omitted when null. Child inlines are flattened into `alt` text for accessibility.",
        "LineBreak" => "Renders as `<br>`.",
        "SoftBreak" => "Renders as a literal newline character (`\\n`) inside the output. Most browsers collapse this to whitespace; authors who want a visual break should use `LineBreak`.",
        "HtmlInline" => "Passes `text` through verbatim — same sanitiser policy as `HtmlBlock`.",
        "Wikilink" => "Renders as `<a href=\"{resolved}\" class=\"ztl-wikilink\">{label}</a>`. `{resolved}` is the slug for `target` (with `#heading` or `#^block-id` appended when set). `{label}` is `alias` when set, otherwise `target`. Unresolvable wikilinks render as `<a href=\"#\" class=\"ztl-wikilink ztl-wikilink-missing\">{label}</a>`.",
        _ => "// TODO: HTML expectation missing",
    }
}
