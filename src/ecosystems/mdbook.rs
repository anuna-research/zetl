//! mdBook preprocessor adapter (SPEC-033 REQ-3304 / CON-3304 / ADR-3303).
//!
//! Implements the [`EcosystemAdapter`] trait for the mdBook ecosystem.
//! Compiled behind the `ecosystem-mdbook` cargo feature (NFR-3304) so a
//! minimal zetl build can drop this code entirely; the registry entry
//! stays unconditional in [`crate::ecosystems::registry`] so `zetl
//! ecosystem check` can still list `mdbook` on a stripped binary.
//!
//! ## Invocation contract (CON-3304)
//!
//! mdBook preprocessors follow a two-call protocol:
//!
//! - `<exec> supports html` — probe; exit 0 = the preprocessor supports
//!   the html renderer.
//! - `<exec>` — real run; stdin is a `[Context, Book]` JSON array,
//!   stdout is the transformed `Book` JSON.
//!
//! The adapter runs at zetl's `pre-parse` stage (ADR-3303) because the
//! protocol is text-in / text-out: the chapter's raw Markdown lives
//! inside the envelope as a string field, and that's what preprocessors
//! rewrite.
//!
//! ## Envelope shape (REQ-3309)
//!
//! The adapter builds a synthetic mdBook envelope for each invocation:
//!
//! ```json
//! [
//!   {
//!     "root": "<vault_root>",
//!     "config": {
//!       "book": {"title": "<vault_name>", "authors": [], "src": "."},
//!       "preprocessor": {}
//!     },
//!     "renderer": "html",
//!     "mdbook_version": "<adapter-declared>"
//!   },
//!   {
//!     "sections": [
//!       {"Chapter": {
//!         "name": "<page.name>",
//!         "content": "<raw markdown>",
//!         "number": null,
//!         "sub_items": [],
//!         "path": "<page.slug>.md",
//!         "source_path": "<page.slug>.md",
//!         "parent_names": []
//!       }}
//!     ],
//!     "__non_exhaustive": null
//!   }
//! ]
//! ```
//!
//! ## `scope = "page" | "vault"` (REQ-3304)
//!
//! The manifest's `scope` field chooses between per-page invocation
//! (default, one preprocessor call per page; maximally parallelisable)
//! and whole-vault invocation (one call per vault; for preprocessors
//! like `mdbook-toc` that need to see sibling chapters). v1 always
//! receives a single page from the pipeline, so `scope = "vault"`
//! produces the same envelope shape but records an
//! [`Diagnostic`] noting that full-book batching is a later phase.
//! Adapter callers that wire up vault-scope batching at a higher layer
//! drop the diagnostic by using [`build_envelope_with_chapters`] and
//! passing in the pre-batched chapter list themselves.

#![cfg(feature = "ecosystem-mdbook")]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::ecosystems::adapter::{
    Diagnostic, EcosystemAdapter, HookContext, PluginManifest, PluginResponse, RuntimeStatus,
    StageInput, StageOutput,
};
use crate::ecosystems::detection::{install_hint_for, probe_runtime_dep};
use crate::ecosystems::manifest::MdbookScope;
use crate::ecosystems::registry::{Ecosystem, EcosystemEntry};
use crate::hooks::ast::Document;
use crate::hooks::pipeline::Stage;
use crate::hooks::translators::{zetl_ext::ZetlExtTranslator, AstType, TranslationError, Translator};

/// `mdbook_version` string baked into the synthetic envelope (REQ-3309).
///
/// Tracks the mdBook preprocessor protocol version the adapter targets;
/// kept as a compile-time constant so downstream preprocessors reading
/// the envelope can sanity-check compatibility without round-tripping
/// through `Self::probe`.
pub const MDBOOK_PROTOCOL_VERSION: &str = "0.4.40";

/// Binary used for the ecosystem-level runtime probe (the `mdbook` CLI
/// itself — NOT per-preprocessor probes).
const MDBOOK_BINARY: &str = "mdbook";

/// Bounded deadline for the `<exec> supports html` probe. Preprocessors
/// that hang here are treated as not-supporting.
const SUPPORTS_HTML_TIMEOUT: Duration = Duration::from_secs(5);

// ── Manifest options ─────────────────────────────────────────────────────

/// Ecosystem-specific options block read out of
/// [`PluginManifest::options`] (REQ-3304 / REQ-3312). Every field is
/// optional; deserialisation ignores unknown keys so downstream evolution
/// stays additive.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MdbookOptions {
    /// Invocation scope (see [`MdbookScope`]). Defaults to `Page`.
    #[serde(default)]
    pub scope: Option<MdbookScope>,
    /// Override the `renderer` field in the synthetic Context envelope.
    /// Defaults to `"html"` per REQ-3304 (the adapter probes with
    /// `supports html`, so preprocessors that gate on a different
    /// renderer name would fail the probe anyway).
    #[serde(default)]
    pub renderer: Option<String>,
    /// Override the `mdbook_version` field. Useful when a preprocessor
    /// pins against a specific mdBook major; defaults to
    /// [`MDBOOK_PROTOCOL_VERSION`].
    #[serde(default)]
    pub mdbook_version: Option<String>,
}

impl MdbookOptions {
    pub fn from_value(value: &Value) -> Result<Self, serde_json::Error> {
        if value.is_null() {
            Ok(Self::default())
        } else {
            serde_json::from_value(value.clone())
        }
    }

    pub fn effective_scope(&self) -> MdbookScope {
        self.scope.unwrap_or(MdbookScope::Page)
    }

    pub fn effective_renderer(&self) -> &str {
        self.renderer.as_deref().unwrap_or("html")
    }

    pub fn effective_mdbook_version(&self) -> &str {
        self.mdbook_version
            .as_deref()
            .unwrap_or(MDBOOK_PROTOCOL_VERSION)
    }
}

// ── Envelope construction (REQ-3309) ─────────────────────────────────────

/// A single chapter inside the synthetic [`Book`]. The zetl adapter only
/// ever emits one chapter per invocation in v1 (one page per call), but
/// the type is exposed so higher-layer callers can pre-batch whole-vault
/// payloads for `scope = "vault"` preprocessors.
#[derive(Debug, Clone, PartialEq)]
pub struct Chapter {
    /// Chapter display name — the zetl page name field.
    pub name: String,
    /// Raw chapter Markdown. This is the field preprocessors rewrite;
    /// after the run, [`extract_chapter_content`] reads it back out.
    pub content: String,
    /// Chapter path inside the synthetic book. zetl uses
    /// `<page.slug>.md` so preprocessors that resolve
    /// relative links stay happy.
    pub path: PathBuf,
}

impl Chapter {
    pub fn new(name: impl Into<String>, content: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            content: content.into(),
            path: path.into(),
        }
    }
}

/// Build the CON-3304 `[Context, Book]` envelope for a pre-batched list
/// of chapters.
///
/// Pure function — no subprocess dispatch, no filesystem access. The
/// resulting [`Value`] is exactly the JSON payload that goes on stdin.
/// Callers batching whole-vault runs assemble their chapter list once
/// and call this directly; the per-page adapter path wraps it via
/// [`build_envelope_for_page`].
pub fn build_envelope_with_chapters(
    vault_root: &Path,
    book_title: &str,
    renderer: &str,
    mdbook_version: &str,
    chapters: &[Chapter],
) -> Value {
    let sections: Vec<Value> = chapters
        .iter()
        .map(|ch| {
            let path_str = ch.path.to_string_lossy().into_owned();
            json!({
                "Chapter": {
                    "name": ch.name,
                    "content": ch.content,
                    "number": Value::Null,
                    "sub_items": [],
                    "path": path_str,
                    "source_path": path_str,
                    "parent_names": [],
                }
            })
        })
        .collect();

    let ctx = json!({
        "root": vault_root.to_string_lossy(),
        "config": {
            "book": {
                "title": book_title,
                "authors": [],
                "src": ".",
            },
            "preprocessor": {},
        },
        "renderer": renderer,
        "mdbook_version": mdbook_version,
    });

    let book = json!({
        "sections": sections,
        "__non_exhaustive": Value::Null,
    });

    json!([ctx, book])
}

/// Build the synthetic envelope for a single page (the v1 per-invocation
/// path). Thin shim over [`build_envelope_with_chapters`] that packages
/// the current page as a one-chapter book.
pub fn build_envelope_for_page(
    vault_root: &Path,
    book_title: &str,
    options: &MdbookOptions,
    page_name: &str,
    page_slug: &str,
    markdown: &str,
) -> Value {
    let path = PathBuf::from(format!("{page_slug}.md"));
    let chapters = [Chapter::new(page_name, markdown, path)];
    build_envelope_with_chapters(
        vault_root,
        book_title,
        options.effective_renderer(),
        options.effective_mdbook_version(),
        &chapters,
    )
}

/// Path to the envelope JSON Schema (REQ-3309 / CON-3309). The schema
/// is the authoritative shape reference; [`validate_envelope`] is a
/// hand-rolled fast-path that enforces the structural subset zetl
/// actually depends on (the full schema is exercised in integration
/// tests against `jsonschema::Validator`).
pub const ENVELOPE_SCHEMA_PATH: &str = "tools/zetl-mdbook-envelope-schema-v1.json";

/// Structurally validate the `[Context, Book]` envelope (REQ-3309).
///
/// Hand-rolled rather than running `jsonschema` so the mdBook adapter
/// has zero extra runtime deps; the full JSON Schema at
/// [`ENVELOPE_SCHEMA_PATH`] is asserted in integration tests. The
/// checks here are the structural subset zetl itself depends on:
/// 2-element outer array, required Context fields, required Book
/// fields, and one Chapter per section when sections are non-empty.
/// Unknown fields on either side are tolerated — mdBook's own Book
/// struct is `#[non_exhaustive]` and v2 may add fields.
pub fn validate_envelope(envelope: &Value) -> Result<(), EnvelopeError> {
    let arr = envelope
        .as_array()
        .ok_or_else(|| EnvelopeError::WrongShape("envelope must be a JSON array".into()))?;
    if arr.len() != 2 {
        return Err(EnvelopeError::WrongShape(format!(
            "envelope must be a 2-element [Context, Book] array, got length {}",
            arr.len()
        )));
    }
    validate_context(&arr[0])?;
    validate_book_strict(&arr[1])
}

fn validate_context(ctx: &Value) -> Result<(), EnvelopeError> {
    let obj = ctx
        .as_object()
        .ok_or_else(|| EnvelopeError::WrongShape("Context must be an object".into()))?;
    for field in ["root", "config", "renderer", "mdbook_version"] {
        if !obj.contains_key(field) {
            return Err(EnvelopeError::MissingField(format!("context.{field}")));
        }
    }
    if !obj["root"].is_string() {
        return Err(EnvelopeError::WrongShape("context.root must be a string".into()));
    }
    if !obj["renderer"].is_string() {
        return Err(EnvelopeError::WrongShape(
            "context.renderer must be a string".into(),
        ));
    }
    if !obj["mdbook_version"].is_string() {
        return Err(EnvelopeError::WrongShape(
            "context.mdbook_version must be a string".into(),
        ));
    }

    let config = obj["config"]
        .as_object()
        .ok_or_else(|| EnvelopeError::WrongShape("context.config must be an object".into()))?;
    let book = config
        .get("book")
        .ok_or_else(|| EnvelopeError::MissingField("context.config.book".into()))?
        .as_object()
        .ok_or_else(|| {
            EnvelopeError::WrongShape("context.config.book must be an object".into())
        })?;
    for field in ["title", "authors", "src"] {
        if !book.contains_key(field) {
            return Err(EnvelopeError::MissingField(format!(
                "context.config.book.{field}"
            )));
        }
    }
    if !book["title"].is_string() {
        return Err(EnvelopeError::WrongShape(
            "context.config.book.title must be a string".into(),
        ));
    }
    if !book["authors"].is_array() {
        return Err(EnvelopeError::WrongShape(
            "context.config.book.authors must be an array".into(),
        ));
    }
    if !book["src"].is_string() {
        return Err(EnvelopeError::WrongShape(
            "context.config.book.src must be a string".into(),
        ));
    }
    if !config.contains_key("preprocessor") {
        return Err(EnvelopeError::MissingField(
            "context.config.preprocessor".into(),
        ));
    }
    if !config["preprocessor"].is_object() {
        return Err(EnvelopeError::WrongShape(
            "context.config.preprocessor must be an object".into(),
        ));
    }
    Ok(())
}

fn validate_book(book: &Value) -> Result<(), EnvelopeError> {
    validate_book_with(book, /* require_non_exhaustive = */ false)
}

/// Strict form of [`validate_book`] — asserts the `__non_exhaustive: null`
/// marker is present. Used on the envelope zetl itself constructs so a
/// regression in [`build_envelope_with_chapters`] is caught.
/// Preprocessor responses use the lenient form because many
/// preprocessors re-emit the Book through their own JSON library
/// (serde-js, python json, …) and drop serde-Rust's `#[non_exhaustive]`
/// marker on the way back.
fn validate_book_strict(book: &Value) -> Result<(), EnvelopeError> {
    validate_book_with(book, true)
}

fn validate_book_with(book: &Value, require_non_exhaustive: bool) -> Result<(), EnvelopeError> {
    let obj = book
        .as_object()
        .ok_or_else(|| EnvelopeError::WrongShape("Book must be an object".into()))?;
    let sections = obj
        .get("sections")
        .ok_or_else(|| EnvelopeError::MissingField("book.sections".into()))?
        .as_array()
        .ok_or_else(|| EnvelopeError::WrongShape("book.sections must be an array".into()))?;
    if require_non_exhaustive {
        if !obj.contains_key("__non_exhaustive") {
            return Err(EnvelopeError::MissingField("book.__non_exhaustive".into()));
        }
        if !obj["__non_exhaustive"].is_null() {
            return Err(EnvelopeError::WrongShape(
                "book.__non_exhaustive must be null".into(),
            ));
        }
    } else if obj.contains_key("__non_exhaustive") && !obj["__non_exhaustive"].is_null() {
        return Err(EnvelopeError::WrongShape(
            "book.__non_exhaustive must be null when present".into(),
        ));
    }
    for (i, section) in sections.iter().enumerate() {
        validate_section(section, i)?;
    }
    Ok(())
}

fn validate_section(section: &Value, index: usize) -> Result<(), EnvelopeError> {
    // Accept every BookItem variant the schema allows. zetl only emits
    // `Chapter`, but preprocessors reading the Book back may re-emit
    // PartTitle / Separator; reject only genuinely unrecognised shapes.
    if let Some(s) = section.as_str() {
        if s == "Separator" {
            return Ok(());
        }
        return Err(EnvelopeError::WrongShape(format!(
            "book.sections[{index}] string must be \"Separator\", got {s:?}"
        )));
    }
    let obj = section.as_object().ok_or_else(|| {
        EnvelopeError::WrongShape(format!(
            "book.sections[{index}] must be an object or \"Separator\" string"
        ))
    })?;
    if let Some(chapter) = obj.get("Chapter") {
        validate_chapter(chapter, index)
    } else if obj.contains_key("PartTitle") {
        if obj["PartTitle"].is_string() {
            Ok(())
        } else {
            Err(EnvelopeError::WrongShape(format!(
                "book.sections[{index}].PartTitle must be a string"
            )))
        }
    } else {
        Err(EnvelopeError::WrongShape(format!(
            "book.sections[{index}] must be a Chapter, PartTitle, or Separator"
        )))
    }
}

fn validate_chapter(chapter: &Value, index: usize) -> Result<(), EnvelopeError> {
    let obj = chapter.as_object().ok_or_else(|| {
        EnvelopeError::WrongShape(format!(
            "book.sections[{index}].Chapter must be an object"
        ))
    })?;
    for field in [
        "name",
        "content",
        "number",
        "sub_items",
        "path",
        "source_path",
        "parent_names",
    ] {
        if !obj.contains_key(field) {
            return Err(EnvelopeError::MissingField(format!(
                "book.sections[{index}].Chapter.{field}"
            )));
        }
    }
    if !obj["name"].is_string() {
        return Err(EnvelopeError::WrongShape(format!(
            "book.sections[{index}].Chapter.name must be a string"
        )));
    }
    if !obj["content"].is_string() {
        return Err(EnvelopeError::WrongShape(format!(
            "book.sections[{index}].Chapter.content must be a string"
        )));
    }
    if !(obj["number"].is_null() || obj["number"].is_array()) {
        return Err(EnvelopeError::WrongShape(format!(
            "book.sections[{index}].Chapter.number must be null or an array"
        )));
    }
    if !obj["sub_items"].is_array() {
        return Err(EnvelopeError::WrongShape(format!(
            "book.sections[{index}].Chapter.sub_items must be an array"
        )));
    }
    if !(obj["path"].is_string() || obj["path"].is_null()) {
        return Err(EnvelopeError::WrongShape(format!(
            "book.sections[{index}].Chapter.path must be a string or null"
        )));
    }
    if !(obj["source_path"].is_string() || obj["source_path"].is_null()) {
        return Err(EnvelopeError::WrongShape(format!(
            "book.sections[{index}].Chapter.source_path must be a string or null"
        )));
    }
    if !obj["parent_names"].is_array() {
        return Err(EnvelopeError::WrongShape(format!(
            "book.sections[{index}].Chapter.parent_names must be an array"
        )));
    }
    Ok(())
}

/// Pull the transformed first chapter's `content` out of a preprocessor's
/// response book.
///
/// Preprocessors emit a `Book` JSON (without the surrounding context) on
/// stdout. The v1 per-page adapter only cares about the first chapter
/// since that's what we sent in. Returns an error if the shape is not
/// the one mdBook itself emits.
pub fn extract_chapter_content(book: &Value) -> Result<String, EnvelopeError> {
    let sections = book
        .get("sections")
        .ok_or_else(|| EnvelopeError::MissingField("sections".into()))?
        .as_array()
        .ok_or_else(|| EnvelopeError::WrongShape("sections must be an array".into()))?;

    if sections.is_empty() {
        return Err(EnvelopeError::EmptySections);
    }

    let chapter = sections[0]
        .get("Chapter")
        .ok_or_else(|| EnvelopeError::WrongShape("first section is not a Chapter".into()))?;

    chapter
        .get("content")
        .and_then(|c| c.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| EnvelopeError::MissingField("Chapter.content".into()))
}

/// Typed error from [`extract_chapter_content`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvelopeError {
    /// The preprocessor's response didn't include a required envelope
    /// field. Carries the dotted field path for diagnostics.
    MissingField(String),
    /// The field existed but had the wrong JSON shape (array vs object,
    /// string vs number, etc.).
    WrongShape(String),
    /// `book.sections` was present but empty — the preprocessor stripped
    /// the single chapter we sent. Treated as an error because a pre-parse
    /// hook cannot drop the page wholesale.
    EmptySections,
}

impl std::fmt::Display for EnvelopeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnvelopeError::MissingField(p) => write!(f, "mdbook envelope missing field: {p}"),
            EnvelopeError::WrongShape(msg) => write!(f, "mdbook envelope has wrong shape: {msg}"),
            EnvelopeError::EmptySections => f.write_str("mdbook envelope has empty sections"),
        }
    }
}

impl std::error::Error for EnvelopeError {}

// ── Probe: supports html ─────────────────────────────────────────────────

/// Run `<exec> supports html` per CON-3304. Returns `Ok(true)` when the
/// preprocessor exits 0, `Ok(false)` when it exits non-zero (the mdBook
/// protocol's "no, I don't support this renderer" signal), and `Err` for
/// plumbing-level failures (binary not found, timeout, I/O error).
pub fn supports_html(exec: &Path, timeout: Duration) -> Result<bool, SupportsError> {
    let mut cmd = Command::new(exec);
    cmd.args(["supports", "html"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(SupportsError::NotFound {
                binary: exec.display().to_string(),
            });
        }
        Err(e) => return Err(SupportsError::Io(e)),
    };

    // Watchdog: SIGKILL on timeout so a broken preprocessor can't hang
    // the build. Mirrors the pandoc adapter's subprocess watchdog.
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let pid = child.id();
    let watchdog = thread::spawn(move || match done_rx.recv_timeout(timeout) {
        Ok(()) | Err(RecvTimeoutError::Disconnected) => false,
        Err(RecvTimeoutError::Timeout) => {
            #[cfg(unix)]
            unsafe {
                libc_kill(pid as i32, 9);
            }
            #[cfg(not(unix))]
            {
                let _ = pid;
            }
            true
        }
    });

    let status = match child.wait() {
        Ok(s) => s,
        Err(e) => return Err(SupportsError::Io(e)),
    };
    let _ = done_tx.send(());
    let timed_out = watchdog.join().unwrap_or(false);

    if timed_out {
        return Err(SupportsError::Timeout);
    }

    Ok(status.success())
}

/// Typed error from [`supports_html`].
#[derive(Debug)]
pub enum SupportsError {
    NotFound { binary: String },
    Io(std::io::Error),
    Timeout,
}

impl std::fmt::Display for SupportsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SupportsError::NotFound { binary } => {
                write!(f, "mdbook preprocessor '{binary}' not found on PATH")
            }
            SupportsError::Io(e) => write!(f, "I/O error probing preprocessor: {e}"),
            SupportsError::Timeout => f.write_str("preprocessor probe timed out"),
        }
    }
}

impl std::error::Error for SupportsError {}

// ── Adapter ──────────────────────────────────────────────────────────────

/// mdBook adapter (REQ-3304). See module docs for the full shape.
pub struct MdbookAdapter {
    translator: ZetlExtTranslator,
    /// Resolved path to the host `mdbook` binary, populated by
    /// [`Self::probe`]. The ecosystem-level probe is advisory: many
    /// users have preprocessors installed (`cargo install mdbook-mermaid`)
    /// without mdBook itself on PATH, and preprocessors run
    /// independently.
    mdbook_path: PathBuf,
    mdbook_version: Option<String>,
    /// `true` once the mdbook binary has been confirmed available. Used
    /// for reporting, not for gating — preprocessors run without the
    /// mdbook binary.
    mdbook_available: bool,
    stages: Vec<Stage>,
}

impl Default for MdbookAdapter {
    fn default() -> Self {
        Self {
            translator: ZetlExtTranslator,
            mdbook_path: PathBuf::from(MDBOOK_BINARY),
            mdbook_version: None,
            mdbook_available: false,
            stages: vec![Stage::PreParse],
        }
    }
}

impl MdbookAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mdbook_version(&self) -> Option<&str> {
        self.mdbook_version.as_deref()
    }

    pub fn mdbook_path(&self) -> &Path {
        &self.mdbook_path
    }
}

impl EcosystemAdapter for MdbookAdapter {
    fn id(&self) -> &str {
        "mdbook"
    }

    fn ast_type(&self) -> AstType {
        AstType::ZetlExt
    }

    fn probe(&mut self) -> RuntimeStatus {
        let entry: &EcosystemEntry = Ecosystem::Mdbook.entry();
        let status = probe_runtime_dep(&entry.runtime_dep);
        match &status {
            RuntimeStatus::Available {
                executable,
                version,
            } => {
                self.mdbook_version = Some(version.clone());
                if let Some(path) = executable {
                    self.mdbook_path = path.clone();
                }
                self.mdbook_available = true;
            }
            _ => {
                self.mdbook_available = false;
            }
        }
        status
    }

    fn supported_stages(&self) -> &[Stage] {
        &self.stages
    }

    fn translate_to_foreign(&self, doc: &Document) -> Result<Value, TranslationError> {
        self.translator.zetl_to_foreign(doc)
    }

    fn translate_from_foreign(&self, foreign: Value) -> Result<Document, TranslationError> {
        self.translator.foreign_to_zetl(foreign)
    }

    fn invoke_plugin(
        &mut self,
        manifest: &PluginManifest,
        input: StageInput,
        context: &HookContext<'_>,
    ) -> PluginResponse {
        let start = Instant::now();

        if !self.stages.contains(&context.stage) {
            return PluginResponse::error(
                "unsupported_stage",
                format!(
                    "mdbook adapter does not support stage {} (supports: {:?})",
                    context.stage, self.stages
                ),
                start.elapsed(),
            );
        }

        let markdown = match input {
            StageInput::PreParse { markdown } => markdown,
            other => {
                return PluginResponse::error(
                    "unsupported_stage",
                    format!(
                        "mdbook adapter expected a pre-parse input, got {:?}",
                        other.stage()
                    ),
                    start.elapsed(),
                );
            }
        };

        let options = match MdbookOptions::from_value(&manifest.options) {
            Ok(o) => o,
            Err(e) => {
                return PluginResponse::error(
                    "manifest_parse",
                    format!("could not parse mdbook options: {e}"),
                    start.elapsed(),
                );
            }
        };

        let mut diagnostics = Vec::new();
        if options.effective_scope() == MdbookScope::Vault {
            diagnostics.push(Diagnostic {
                severity: "warning".into(),
                message: format!(
                    "mdbook preprocessor '{}' declared scope=\"vault\", \
                     but v1 invokes preprocessors per page — the envelope \
                     will contain only the current page. Cross-chapter \
                     preprocessors (e.g. mdbook-toc) will not see sibling \
                     chapters.",
                    manifest.extension_id
                ),
                page_slug: Some(context.build.page.slug.clone()),
            });
        }

        // CON-3304 probe — short-circuit before spawning the real run.
        // The probe uses its own bounded timeout, separate from the
        // manifest timeout so a hung probe can't starve the invocation.
        match supports_html(&manifest.executable, SUPPORTS_HTML_TIMEOUT) {
            Ok(true) => {}
            Ok(false) => {
                return PluginResponse::error(
                    "contract_violation",
                    format!(
                        "mdbook preprocessor '{}' does not support the html renderer \
                         (`{} supports html` exited non-zero)",
                        manifest.extension_id,
                        manifest.executable.display()
                    ),
                    start.elapsed(),
                );
            }
            Err(SupportsError::NotFound { binary }) => {
                return PluginResponse::Error {
                    reason: "runtime_absence".into(),
                    detail: format!(
                        "mdbook preprocessor '{}' could not be spawned: binary '{}' not found. {}",
                        manifest.extension_id,
                        binary,
                        install_hint_for(MDBOOK_BINARY)
                    ),
                    stderr: String::new(),
                    duration: start.elapsed(),
                };
            }
            Err(SupportsError::Timeout) => {
                return PluginResponse::error(
                    "timeout",
                    format!(
                        "mdbook preprocessor '{}' probe `supports html` timed out",
                        manifest.extension_id
                    ),
                    start.elapsed(),
                );
            }
            Err(SupportsError::Io(e)) => {
                return PluginResponse::error(
                    "spawn_failed",
                    format!(
                        "mdbook preprocessor '{}' probe failed: {e}",
                        manifest.extension_id
                    ),
                    start.elapsed(),
                );
            }
        }

        let book_title = context.build.page.name.clone();
        let envelope = build_envelope_for_page(
            &context.build.vault_root,
            &book_title,
            &options,
            &context.build.page.name,
            &context.build.page.slug,
            &markdown,
        );

        let stdin_bytes = match serde_json::to_vec(&envelope) {
            Ok(b) => b,
            Err(e) => {
                return PluginResponse::error(
                    "serialisation",
                    format!("could not serialise mdbook envelope: {e}"),
                    start.elapsed(),
                );
            }
        };

        let mut cmd = Command::new(&manifest.executable);
        cmd.args(&manifest.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        match spawn_and_exchange(cmd, &stdin_bytes, manifest.timeout) {
            Ok(exchange) => {
                let duration = start.elapsed();
                let book: Value = match serde_json::from_slice(&exchange.stdout) {
                    Ok(v) => v,
                    Err(e) => {
                        return PluginResponse::Error {
                            reason: "malformed_output".into(),
                            detail: format!(
                                "mdbook preprocessor '{}' produced non-JSON stdout: {e}",
                                manifest.extension_id
                            ),
                            stderr: String::from_utf8_lossy(&exchange.stderr).into_owned(),
                            duration,
                        };
                    }
                };
                if let Err(e) = validate_book(&book) {
                    return PluginResponse::Error {
                        reason: "malformed_output".into(),
                        detail: format!(
                            "mdbook preprocessor '{}' returned a Book that failed envelope validation: {e}",
                            manifest.extension_id
                        ),
                        stderr: String::from_utf8_lossy(&exchange.stderr).into_owned(),
                        duration,
                    };
                }
                match extract_chapter_content(&book) {
                    Ok(out) => PluginResponse::Success {
                        output: StageOutput::PreParse { markdown: out },
                        diagnostics,
                        stderr: String::from_utf8_lossy(&exchange.stderr).into_owned(),
                        duration,
                    },
                    Err(e) => PluginResponse::Error {
                        reason: "malformed_output".into(),
                        detail: format!(
                            "mdbook preprocessor '{}' returned an envelope with invalid shape: {e}",
                            manifest.extension_id
                        ),
                        stderr: String::from_utf8_lossy(&exchange.stderr).into_owned(),
                        duration,
                    },
                }
            }
            Err(e) => {
                let duration = start.elapsed();
                match e {
                    SpawnError::NotFound { binary } => PluginResponse::Error {
                        reason: "runtime_absence".into(),
                        detail: format!(
                            "mdbook preprocessor '{}' could not be spawned: binary '{}' not found",
                            manifest.extension_id, binary
                        ),
                        stderr: String::new(),
                        duration,
                    },
                    SpawnError::Io(err) => PluginResponse::Error {
                        reason: "spawn_failed".into(),
                        detail: format!(
                            "mdbook preprocessor '{}' spawn failed: {err}",
                            manifest.extension_id
                        ),
                        stderr: String::new(),
                        duration,
                    },
                    SpawnError::Timeout => PluginResponse::Error {
                        reason: "timeout".into(),
                        detail: format!(
                            "mdbook preprocessor '{}' exceeded timeout of {:?}",
                            manifest.extension_id, manifest.timeout
                        ),
                        stderr: String::new(),
                        duration,
                    },
                    SpawnError::NonZeroExit { code, stderr } => PluginResponse::Error {
                        reason: "non_zero_exit".into(),
                        detail: format!(
                            "mdbook preprocessor '{}' exited with code {}",
                            manifest.extension_id,
                            code.map(|c| c.to_string())
                                .unwrap_or_else(|| "signal".into()),
                        ),
                        stderr: String::from_utf8_lossy(&stderr).into_owned(),
                        duration,
                    },
                }
            }
        }
    }
}

// ── Subprocess exchange ──────────────────────────────────────────────────

struct SpawnOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

enum SpawnError {
    NotFound { binary: String },
    Io(std::io::Error),
    Timeout,
    NonZeroExit { code: Option<i32>, stderr: Vec<u8> },
}

/// Spawn `cmd`, write `stdin_bytes` to its stdin, collect stdout + stderr
/// with a hard deadline. mdBook preprocessors are one-shot per
/// invocation (CON-3304 explicitly rules out persistent mode for v1), so
/// this function mirrors the pandoc adapter's minimal exchange.
fn spawn_and_exchange(
    mut cmd: Command,
    stdin_bytes: &[u8],
    timeout: Duration,
) -> Result<SpawnOutput, SpawnError> {
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let binary = cmd.get_program().to_string_lossy().into_owned();
            return Err(SpawnError::NotFound { binary });
        }
        Err(e) => return Err(SpawnError::Io(e)),
    };

    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdin_payload = stdin_bytes.to_vec();
    let writer = thread::spawn(move || -> std::io::Result<()> {
        stdin.write_all(&stdin_payload)?;
        stdin.flush()?;
        drop(stdin);
        Ok(())
    });

    let mut stdout_pipe = child.stdout.take().expect("piped stdout");
    let mut stderr_pipe = child.stderr.take().expect("piped stderr");
    let (stderr_tx, stderr_rx) = mpsc::channel::<Vec<u8>>();
    let stderr_handle = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut buf);
        let _ = stderr_tx.send(buf);
    });

    let (done_tx, done_rx) = mpsc::channel::<()>();
    let pid = child.id();
    let watchdog = thread::spawn(move || match done_rx.recv_timeout(timeout) {
        Ok(()) | Err(RecvTimeoutError::Disconnected) => false,
        Err(RecvTimeoutError::Timeout) => {
            #[cfg(unix)]
            unsafe {
                libc_kill(pid as i32, 9);
            }
            #[cfg(not(unix))]
            {
                let _ = pid;
            }
            true
        }
    });

    let mut stdout = Vec::new();
    let read_result = stdout_pipe.read_to_end(&mut stdout);
    let _ = done_tx.send(());
    let timed_out = watchdog.join().unwrap_or(false);
    let _ = writer.join();
    let stderr = stderr_rx.recv().unwrap_or_default();
    let _ = stderr_handle.join();

    match read_result {
        Ok(_) => {}
        Err(e) => return Err(SpawnError::Io(e)),
    }

    let status = match child.wait() {
        Ok(s) => s,
        Err(e) => return Err(SpawnError::Io(e)),
    };

    if timed_out {
        return Err(SpawnError::Timeout);
    }

    if !status.success() {
        return Err(SpawnError::NonZeroExit {
            code: status.code(),
            stderr,
        });
    }

    Ok(SpawnOutput { stdout, stderr })
}

/// Thin wrapper around `libc::kill` so the non-unix code path compiles
/// without the dep. Matches the pandoc / remark adapters.
#[cfg(unix)]
unsafe fn libc_kill(pid: i32, signum: i32) {
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    kill(pid, signum);
}

/// Convenience ctor for registry wiring (parity with
/// [`crate::ecosystems::adapter::mock_adapter_ctor`]).
pub fn mdbook_adapter_ctor() -> Box<dyn EcosystemAdapter> {
    Box::new(MdbookAdapter::new())
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::build_context::{BuildContext, BuildMode, PageMeta};
    use std::path::PathBuf;
    use std::time::Duration;

    fn sample_manifest(exec: &str, args: Vec<String>, scope: Option<&str>) -> PluginManifest {
        let options = match scope {
            Some(s) => json!({ "scope": s }),
            None => Value::Null,
        };
        PluginManifest {
            extension_id: "test-mdbook".into(),
            executable: PathBuf::from(exec),
            args,
            options,
            timeout: Duration::from_secs(5),
        }
    }

    // ── Options parsing ──────────────────────────────────────────────────

    #[test]
    fn options_default_when_null() {
        let opts = MdbookOptions::from_value(&Value::Null).unwrap();
        assert!(opts.scope.is_none());
        assert_eq!(opts.effective_scope(), MdbookScope::Page);
        assert_eq!(opts.effective_renderer(), "html");
        assert_eq!(opts.effective_mdbook_version(), MDBOOK_PROTOCOL_VERSION);
    }

    #[test]
    fn options_parse_page_scope() {
        let v = json!({"scope": "page"});
        let opts = MdbookOptions::from_value(&v).unwrap();
        assert_eq!(opts.effective_scope(), MdbookScope::Page);
    }

    #[test]
    fn options_parse_vault_scope() {
        let v = json!({"scope": "vault"});
        let opts = MdbookOptions::from_value(&v).unwrap();
        assert_eq!(opts.effective_scope(), MdbookScope::Vault);
    }

    #[test]
    fn options_parse_custom_renderer_and_version() {
        let v = json!({"renderer": "epub", "mdbook_version": "0.5.0"});
        let opts = MdbookOptions::from_value(&v).unwrap();
        assert_eq!(opts.effective_renderer(), "epub");
        assert_eq!(opts.effective_mdbook_version(), "0.5.0");
    }

    #[test]
    fn options_reject_unknown_scope() {
        let v = json!({"scope": "sideways"});
        MdbookOptions::from_value(&v).unwrap_err();
    }

    // ── Envelope shape (REQ-3309) ────────────────────────────────────────

    #[test]
    fn envelope_is_two_element_array_context_book() {
        let opts = MdbookOptions::default();
        let env = build_envelope_for_page(
            Path::new("/tmp/vault"),
            "Page",
            &opts,
            "Page",
            "page",
            "hello",
        );
        let arr = env.as_array().expect("envelope is a JSON array");
        assert_eq!(arr.len(), 2, "envelope must be [Context, Book] 2-tuple");
    }

    #[test]
    fn envelope_context_carries_required_fields() {
        let opts = MdbookOptions::default();
        let env = build_envelope_for_page(
            Path::new("/tmp/vault"),
            "My Vault",
            &opts,
            "Page",
            "page",
            "",
        );
        let ctx = &env[0];
        assert_eq!(ctx["root"], "/tmp/vault");
        assert_eq!(ctx["renderer"], "html");
        assert_eq!(ctx["mdbook_version"], MDBOOK_PROTOCOL_VERSION);
        assert_eq!(ctx["config"]["book"]["title"], "My Vault");
        assert_eq!(ctx["config"]["book"]["src"], ".");
        assert!(ctx["config"]["preprocessor"].is_object());
    }

    #[test]
    fn envelope_book_contains_one_chapter_with_page_content() {
        let opts = MdbookOptions::default();
        let env = build_envelope_for_page(
            Path::new("/tmp/vault"),
            "Vault",
            &opts,
            "Chapter One",
            "chapter-one",
            "# Heading\n\nBody",
        );
        let sections = env[1]["sections"].as_array().unwrap();
        assert_eq!(sections.len(), 1);
        let ch = &sections[0]["Chapter"];
        assert_eq!(ch["name"], "Chapter One");
        assert_eq!(ch["content"], "# Heading\n\nBody");
        assert_eq!(ch["path"], "chapter-one.md");
        assert_eq!(ch["source_path"], "chapter-one.md");
        assert!(ch["number"].is_null());
        assert!(ch["sub_items"].as_array().unwrap().is_empty());
        assert!(ch["parent_names"].as_array().unwrap().is_empty());
    }

    #[test]
    fn envelope_has_non_exhaustive_marker() {
        // mdBook's internal Book struct is #[non_exhaustive] — its JSON
        // form carries a literal `__non_exhaustive: null` field. Omitting
        // it breaks serde round-tripping on preprocessors that accept
        // the Book back into mdBook's own types.
        let opts = MdbookOptions::default();
        let env = build_envelope_for_page(
            Path::new("/tmp/vault"),
            "V",
            &opts,
            "P",
            "p",
            "",
        );
        assert!(env[1]["__non_exhaustive"].is_null());
    }

    #[test]
    fn envelope_respects_custom_renderer_and_version() {
        let opts = MdbookOptions {
            renderer: Some("linkcheck".into()),
            mdbook_version: Some("0.4.99".into()),
            ..Default::default()
        };
        let env = build_envelope_for_page(
            Path::new("/tmp"),
            "V",
            &opts,
            "P",
            "p",
            "",
        );
        assert_eq!(env[0]["renderer"], "linkcheck");
        assert_eq!(env[0]["mdbook_version"], "0.4.99");
    }

    #[test]
    fn envelope_with_multiple_chapters_emits_every_entry() {
        let chapters = [
            Chapter::new("A", "body of a", PathBuf::from("a.md")),
            Chapter::new("B", "body of b", PathBuf::from("b.md")),
        ];
        let env = build_envelope_with_chapters(
            Path::new("/v"),
            "Vault",
            "html",
            "0.4.40",
            &chapters,
        );
        let sections = env[1]["sections"].as_array().unwrap();
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0]["Chapter"]["name"], "A");
        assert_eq!(sections[1]["Chapter"]["name"], "B");
        assert_eq!(sections[1]["Chapter"]["content"], "body of b");
    }

    // ── extract_chapter_content ──────────────────────────────────────────

    #[test]
    fn extract_reads_first_chapter_content() {
        let book = json!({
            "sections": [
                {"Chapter": {"name": "A", "content": "transformed body", "path": "a.md"}}
            ]
        });
        let out = extract_chapter_content(&book).unwrap();
        assert_eq!(out, "transformed body");
    }

    #[test]
    fn extract_round_trips_envelope_produced_by_builder() {
        let opts = MdbookOptions::default();
        let env = build_envelope_for_page(
            Path::new("/v"),
            "V",
            &opts,
            "Page",
            "page",
            "the body",
        );
        // The second element of the envelope is the Book — that's what a
        // preprocessor emits on stdout for a round-trip identity.
        let book = env[1].clone();
        let back = extract_chapter_content(&book).unwrap();
        assert_eq!(back, "the body");
    }

    #[test]
    fn extract_rejects_missing_sections() {
        let book = json!({"__non_exhaustive": null});
        match extract_chapter_content(&book) {
            Err(EnvelopeError::MissingField(f)) => assert_eq!(f, "sections"),
            other => panic!("expected MissingField(sections), got {other:?}"),
        }
    }

    #[test]
    fn extract_rejects_empty_sections() {
        let book = json!({"sections": []});
        assert_eq!(extract_chapter_content(&book), Err(EnvelopeError::EmptySections));
    }

    #[test]
    fn extract_rejects_non_chapter_section() {
        let book = json!({"sections": [{"PartTitle": "Part One"}]});
        match extract_chapter_content(&book) {
            Err(EnvelopeError::WrongShape(_)) => {}
            other => panic!("expected WrongShape, got {other:?}"),
        }
    }

    // ── validate_envelope (REQ-3309 / CON-3309 shape gate) ───────────────

    #[test]
    fn validate_envelope_accepts_what_builder_emits() {
        // Every envelope the pure builder produces must pass the
        // structural validator. Catches a regression where the builder
        // drifts from the shape `invoke_plugin` promises preprocessors.
        let opts = MdbookOptions::default();
        let env = build_envelope_for_page(
            Path::new("/v"),
            "Vault",
            &opts,
            "Page",
            "page",
            "hello",
        );
        validate_envelope(&env).expect("builder output must validate");
    }

    #[test]
    fn validate_envelope_rejects_non_array() {
        let env = json!({"root": "/v"});
        match validate_envelope(&env) {
            Err(EnvelopeError::WrongShape(m)) => assert!(m.contains("JSON array"), "got {m}"),
            other => panic!("expected WrongShape, got {other:?}"),
        }
    }

    #[test]
    fn validate_envelope_rejects_wrong_array_length() {
        let env = json!([{"root": "/v"}]);
        match validate_envelope(&env) {
            Err(EnvelopeError::WrongShape(m)) => assert!(m.contains("2-element"), "got {m}"),
            other => panic!("expected WrongShape, got {other:?}"),
        }
    }

    #[test]
    fn validate_envelope_rejects_missing_root() {
        let env = json!([
            {
                "config": {"book": {"title": "T", "authors": [], "src": "."}, "preprocessor": {}},
                "renderer": "html",
                "mdbook_version": "0.4.40"
            },
            {"sections": [], "__non_exhaustive": null}
        ]);
        match validate_envelope(&env) {
            Err(EnvelopeError::MissingField(f)) => assert_eq!(f, "context.root"),
            other => panic!("expected MissingField(context.root), got {other:?}"),
        }
    }

    #[test]
    fn validate_envelope_rejects_wrong_renderer_type() {
        let env = json!([
            {
                "root": "/v",
                "config": {"book": {"title": "T", "authors": [], "src": "."}, "preprocessor": {}},
                "renderer": 42,
                "mdbook_version": "0.4.40"
            },
            {"sections": [], "__non_exhaustive": null}
        ]);
        match validate_envelope(&env) {
            Err(EnvelopeError::WrongShape(m)) => assert!(m.contains("renderer"), "got {m}"),
            other => panic!("expected WrongShape, got {other:?}"),
        }
    }

    #[test]
    fn validate_envelope_rejects_missing_config_book_title() {
        let env = json!([
            {
                "root": "/v",
                "config": {"book": {"authors": [], "src": "."}, "preprocessor": {}},
                "renderer": "html",
                "mdbook_version": "0.4.40"
            },
            {"sections": [], "__non_exhaustive": null}
        ]);
        match validate_envelope(&env) {
            Err(EnvelopeError::MissingField(f)) => assert_eq!(f, "context.config.book.title"),
            other => panic!("expected MissingField(context.config.book.title), got {other:?}"),
        }
    }

    #[test]
    fn validate_envelope_rejects_missing_non_exhaustive_marker() {
        // Outgoing envelopes MUST carry the __non_exhaustive marker.
        // Inbound Books (preprocessor responses) don't; covered by
        // validate_book_accepts_preprocessor_response_without_marker.
        let env = json!([
            {
                "root": "/v",
                "config": {"book": {"title": "T", "authors": [], "src": "."}, "preprocessor": {}},
                "renderer": "html",
                "mdbook_version": "0.4.40"
            },
            {"sections": []}
        ]);
        match validate_envelope(&env) {
            Err(EnvelopeError::MissingField(f)) => assert_eq!(f, "book.__non_exhaustive"),
            other => panic!("expected MissingField(book.__non_exhaustive), got {other:?}"),
        }
    }

    #[test]
    fn validate_book_accepts_preprocessor_response_without_marker() {
        // Preprocessors often serialise the Book through a non-Rust JSON
        // library (node, python, …) and drop the __non_exhaustive marker
        // on the way back. Accept the inbound shape anyway.
        let book = json!({
            "sections": [
                {"Chapter": {
                    "name": "P", "content": "body", "number": null,
                    "sub_items": [], "path": "p.md", "source_path": "p.md",
                    "parent_names": []
                }}
            ]
        });
        validate_book(&book).expect("lenient validate_book must accept preprocessor output");
    }

    #[test]
    fn validate_book_accepts_part_title_and_separator_sections() {
        // Preprocessors may restructure into an mdBook tree that includes
        // PartTitle / Separator entries. The validator allows every
        // BookItem variant mdBook itself accepts.
        let book = json!({
            "sections": [
                {"PartTitle": "Part One"},
                {"Chapter": {
                    "name": "c", "content": "", "number": null,
                    "sub_items": [], "path": "c.md", "source_path": "c.md",
                    "parent_names": ["Part One"]
                }},
                "Separator"
            ]
        });
        validate_book(&book).expect("PartTitle / Chapter / Separator must all validate");
    }

    #[test]
    fn validate_book_rejects_chapter_missing_required_field() {
        let book = json!({
            "sections": [
                {"Chapter": {"name": "P", "content": "body"}}
            ]
        });
        match validate_book(&book) {
            Err(EnvelopeError::MissingField(f)) => assert!(
                f.starts_with("book.sections[0].Chapter."),
                "got {f}"
            ),
            other => panic!("expected MissingField on Chapter, got {other:?}"),
        }
    }

    #[test]
    fn validate_envelope_round_trips_across_fixture_corpus() {
        // Every default fixture → envelope → validate must succeed; the
        // preprocessor's identity response (echoing the Book element
        // back) must also validate as a preprocessor response.
        let opts = MdbookOptions::default();
        for fx in crate::ecosystems::default_fixtures() {
            let env = build_envelope_for_page(
                Path::new("/v"),
                "Vault",
                &opts,
                &fx.name,
                &fx.name,
                &fx.sample_markdown,
            );
            validate_envelope(&env).unwrap_or_else(|e| {
                panic!("fixture {:?} envelope failed validation: {e}", fx.name)
            });
            // Round-trip: the Book element alone must validate lenient.
            validate_book(&env[1]).unwrap_or_else(|e| {
                panic!("fixture {:?} Book failed validation: {e}", fx.name)
            });
            // And extracting the chapter content returns the exact input.
            let back = extract_chapter_content(&env[1]).unwrap();
            assert_eq!(
                back, fx.sample_markdown,
                "fixture {:?} content round-trip mismatch",
                fx.name
            );
        }
    }

    #[test]
    fn extract_rejects_chapter_missing_content() {
        let book = json!({"sections": [{"Chapter": {"name": "A"}}]});
        match extract_chapter_content(&book) {
            Err(EnvelopeError::MissingField(f)) => assert_eq!(f, "Chapter.content"),
            other => panic!("expected MissingField(Chapter.content), got {other:?}"),
        }
    }

    // ── Adapter shape ────────────────────────────────────────────────────

    #[test]
    fn adapter_id_and_ast_type_match_registry() {
        let adapter = MdbookAdapter::default();
        assert_eq!(adapter.id(), "mdbook");
        assert_eq!(adapter.ast_type(), AstType::ZetlExt);
        assert_eq!(adapter.supported_stages(), &[Stage::PreParse]);
    }

    #[test]
    fn translate_round_trips_through_zetl_ext() {
        // ZetlExt is an identity translator, so the round trip is
        // byte-equivalent. Still asserting it here so a silent registry
        // swap to a lossy translator is caught.
        let adapter = MdbookAdapter::default();
        let fixtures = crate::ecosystems::default_fixtures();
        for fx in &fixtures {
            let foreign = adapter.translate_to_foreign(&fx.document).unwrap();
            let back = adapter.translate_from_foreign(foreign).unwrap();
            assert_eq!(
                crate::hooks::contract::canonicalise(&back),
                crate::hooks::contract::canonicalise(&fx.document),
                "mdbook adapter's zetl-ext translator lost fidelity on fixture '{}'",
                fx.name
            );
        }
    }

    #[test]
    fn invoke_plugin_rejects_non_pre_parse_stage() {
        let mut adapter = MdbookAdapter::default();
        let build = BuildContext::new(
            BuildMode::Build,
            PathBuf::from("/tmp/mdbook-test"),
            PageMeta::synthetic("Page", "page"),
        );
        let ctx = HookContext::new(Stage::Transform, "test-mdbook", &build);
        let manifest = sample_manifest("/bin/cat", vec![], None);
        let input = StageInput::Transform {
            foreign: json!({}),
        };
        match adapter.invoke_plugin(&manifest, input, &ctx) {
            PluginResponse::Error { reason, .. } => assert_eq!(reason, "unsupported_stage"),
            other => panic!("expected unsupported_stage error, got {other:?}"),
        }
    }

    #[test]
    fn invoke_plugin_reports_not_found_for_bogus_exec() {
        let mut adapter = MdbookAdapter::default();
        let build = BuildContext::new(
            BuildMode::Build,
            PathBuf::from("/tmp/mdbook-test"),
            PageMeta::synthetic("Page", "page"),
        );
        let ctx = HookContext::new(Stage::PreParse, "test-mdbook", &build);
        let manifest = sample_manifest(
            "/definitely-not-a-real-mdbook-preprocessor-zetl-test",
            vec![],
            None,
        );
        let input = StageInput::PreParse {
            markdown: "body".into(),
        };
        match adapter.invoke_plugin(&manifest, input, &ctx) {
            PluginResponse::Error { reason, detail, .. } => {
                assert_eq!(reason, "runtime_absence");
                assert!(
                    detail.contains("not found") || detail.contains("No such"),
                    "expected not-found detail, got: {detail}"
                );
            }
            other => panic!("expected runtime_absence error, got {other:?}"),
        }
    }

    #[test]
    fn supports_html_returns_true_when_exit_zero() {
        // `/bin/true` always exits 0 regardless of argv, so the
        // `supports html` probe succeeds.
        #[cfg(unix)]
        {
            let path = std::path::Path::new("/bin/true");
            if path.exists() {
                let ok = supports_html(path, Duration::from_secs(2)).unwrap();
                assert!(ok, "/bin/true should probe as supporting html");
            }
        }
    }

    #[test]
    fn supports_html_returns_false_when_exit_nonzero() {
        #[cfg(unix)]
        {
            let path = std::path::Path::new("/bin/false");
            if path.exists() {
                let ok = supports_html(path, Duration::from_secs(2)).unwrap();
                assert!(!ok, "/bin/false should probe as NOT supporting html");
            }
        }
    }

    #[test]
    fn supports_html_not_found_is_typed_error() {
        let err = supports_html(
            std::path::Path::new("/definitely-missing-mdbook-binary-zetl-test"),
            Duration::from_secs(1),
        )
        .unwrap_err();
        match err {
            SupportsError::NotFound { .. } => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn mdbook_adapter_ctor_returns_box_dyn() {
        let adapter: Box<dyn EcosystemAdapter> = mdbook_adapter_ctor();
        assert_eq!(adapter.id(), "mdbook");
        assert_eq!(adapter.ast_type(), AstType::ZetlExt);
    }
}
