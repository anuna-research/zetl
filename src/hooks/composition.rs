//! Directory drop-in composition and precedence (SPEC-032 REQ-3206 / CON-3206
//! and REQ-3217 / CON-3217).
//!
//! For each of the three [`Stage`]s the system discovers hook executables
//! from two locations and resolves them into a single ordered pipeline:
//!
//! 1. `<vault>/.zetl/hooks/<stage>.d/*` — vault hooks (highest precedence).
//! 2. `<theme-dir>/hooks/<stage>.d/*` — theme-bundled hooks.
//!
//! **Precedence rule (REQ-3206).** A vault hook whose filename collides
//! with a theme hook's filename replaces the theme version wholesale; it
//! does not merge. An empty (0-byte) vault file still shadows the theme
//! hook — the theme hook is *not* invoked — and is itself marked disabled.
//!
//! **Ordering (REQ-3217).** An optional sibling manifest file
//! `<executable>.toml` MAY declare ordering constraints either under an
//! `[ordering]` table (the canonical REQ-3217 form) or at the top level
//! (accepted as a convenience alias):
//!
//! ```toml
//! # Canonical form
//! [ordering]
//! before = ["admonition"]
//! after  = ["callouts"]
//!
//! # Top-level alias — equivalent
//! before = ["admonition"]
//! after  = ["callouts"]
//! ```
//!
//! Plus `optional = true` / `extension_id = "..."` (always top-level). The
//! resulting pipeline is a topological sort over the enabled hook set with
//! filename lex-sort as the tiebreaker between otherwise-unordered hooks.
//! Cycles are reported as a [`CompositionError::Cycle`] with the offending
//! path named by extension_id.
//!
//! `extension_id` defaults to the filename minus its extension AND its
//! leading ordering prefix (one or more digits followed by `-`): e.g.
//! `20-tasks.py` → `tasks`, `callouts` → `callouts`.
//!
//! This module is the discovery + ordering surface only. Invocation
//! (selector evaluation, probe, persistent-mode protocol, failure
//! scoping) is layered on by downstream tasks.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::hooks::pipeline::Stage;
use crate::hooks::translators::AstType;

/// Where a composed hook originated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompositionSource {
    /// Hook from the active theme's `hooks/<stage>.d/` directory.
    Theme,
    /// Hook from the vault's `.zetl/hooks/<stage>.d/` directory.
    Vault,
}

impl std::fmt::Display for CompositionSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompositionSource::Theme => f.write_str("theme"),
            CompositionSource::Vault => f.write_str("vault"),
        }
    }
}

/// Reason a discovered hook is treated as disabled (shadowed without
/// invocation) per REQ-3206.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisabledReason {
    /// File is 0 bytes.
    EmptyFile,
    /// File is not executable and has no shebang line.
    NotExecutableAndNoShebang,
}

impl std::fmt::Display for DisabledReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DisabledReason::EmptyFile => f.write_str("file is empty (0 bytes)"),
            DisabledReason::NotExecutableAndNoShebang => {
                f.write_str("file is not executable and has no shebang")
            }
        }
    }
}

/// A single composed hook — the product of directory-drop-in discovery and
/// sidecar-manifest parsing for REQ-3206 / REQ-3217.
#[derive(Debug, Clone)]
pub struct ComposedHook {
    pub stage: Stage,
    /// Filename (no directory part), used for lex-sort tiebreaker and
    /// collision detection across vault/theme.
    pub filename: String,
    /// Extension id for template-vars namespacing (REQ-3214) and for
    /// `before`/`after` references (REQ-3217). Defaults to the filename
    /// minus its extension and any leading `\d+-` ordering prefix; may be
    /// overridden in the sidecar manifest.
    pub extension_id: String,
    /// Absolute path to the hook executable.
    pub path: PathBuf,
    /// Absolute path to the sidecar manifest file (if one exists).
    pub manifest_path: Option<PathBuf>,
    /// Where this hook came from (theme or vault).
    pub source: CompositionSource,
    /// Extension ids this hook should run before.
    pub before: Vec<String>,
    /// Extension ids this hook should run after.
    pub after: Vec<String>,
    /// Whether the hook declares itself optional. A missing optional hook
    /// referenced by `before`/`after` drops the constraint; a missing
    /// non-optional reference is a [`CompositionError::MissingRequiredRef`].
    pub optional: bool,
    /// AST ecosystem the hook wants to operate against (SPEC-032 REQ-3221).
    /// Defaults to [`AstType::ZetlExt`] when the manifest doesn't declare
    /// one. `transform`-stage hooks branch on this to choose the
    /// translator; `pre-parse` / `post-render` hooks ignore it (the
    /// boundary is raw bytes, not AST).
    pub ast_type: AstType,
    /// Semver range declared by the manifest, interpreted against
    /// [`Self::ast_type`]'s version scheme (REQ-3221). `None` means the
    /// manifest didn't declare a range — the pipeline treats missing as
    /// "any version accepted"; stricter compatibility checks live in
    /// `task-capability-probe` / SPEC-033 REQ-3311.
    pub ast_version: Option<String>,
    /// Node types the hook declares it will preserve under the
    /// `[contract].preserves` tier-1 field (REQ-3224). The pipeline's
    /// marker-strip detector (REQ-3221) counts occurrences of these
    /// names pre/post and warns on any net decrease. If the manifest
    /// omits the field, the default list for the hook's [`AstType`]
    /// is used (`["Wikilink","Embed","SplBlock"]` for foreign-ext
    /// adapters, empty for zetl-ext).
    pub preserves: Vec<String>,
    /// Ecosystem id declared on the manifest's top-level `ecosystem = "..."`
    /// field (SPEC-033 REQ-3301 / CON-3312). `None` means the hook is a
    /// zetl-native hook (no ecosystem adapter involved); a `Some(id)`
    /// value is the manifest's verbatim string — not validated here so
    /// unknown ids surface at dispatch time, not composition time.
    pub ecosystem: Option<String>,
    /// Disable reason if the hook passed the pre-probe filters of REQ-3206;
    /// `None` means the hook is enabled (subject to probe / selector later).
    pub disabled: Option<DisabledReason>,
}

impl ComposedHook {
    pub fn is_enabled(&self) -> bool {
        self.disabled.is_none()
    }
}

/// Result of composing one stage — enabled hooks in resolved pipeline order,
/// plus hooks that were shadowed or disabled during discovery.
#[derive(Debug)]
pub struct StagePipeline {
    /// The stage this pipeline is for.
    pub stage: Stage,
    /// Enabled hooks in the final topologically-sorted order.
    pub hooks: Vec<ComposedHook>,
    /// Theme hooks that were replaced by a same-filename vault hook. Kept
    /// for diagnostics and the `hook coverage` report.
    pub shadowed: Vec<ComposedHook>,
    /// Hooks that were discovered but marked disabled (0-byte, unrunnable).
    pub disabled: Vec<ComposedHook>,
    /// Non-fatal diagnostics surfaced during composition (missing optional
    /// references, unknown manifest fields, etc.).
    pub warnings: Vec<String>,
}

/// Errors that abort composition for a stage.
#[derive(Debug, Clone)]
pub enum CompositionError {
    /// A cycle was detected in the `before`/`after` constraint graph.
    /// `path` names the cycle as a list of extension_ids (first == last).
    /// `files` lists the manifest files involved, for diagnostics.
    Cycle {
        stage: Stage,
        path: Vec<String>,
        files: Vec<PathBuf>,
    },
    /// A `before` or `after` reference named a hook that is not present and
    /// the referencing hook is not marked `optional = true`.
    MissingRequiredRef {
        stage: Stage,
        hook_id: String,
        ref_id: String,
        kind: RefKind,
        manifest: Option<PathBuf>,
    },
    /// A sidecar manifest file failed to parse.
    ManifestParse { path: PathBuf, error: String },
}

impl std::fmt::Display for CompositionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompositionError::Cycle { stage, path, .. } => {
                write!(f, "cycle in {stage} hook ordering: {}", path.join(" → "))
            }
            CompositionError::MissingRequiredRef {
                stage,
                hook_id,
                ref_id,
                kind,
                ..
            } => write!(
                f,
                "hook '{hook_id}' ({stage}) declares {kind} = [{ref_id:?}] but no such hook exists"
            ),
            CompositionError::ManifestParse { path, error } => {
                write!(
                    f,
                    "failed to parse hook manifest {}: {error}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for CompositionError {}

/// Which kind of ordering reference failed to resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    Before,
    After,
}

impl std::fmt::Display for RefKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RefKind::Before => f.write_str("before"),
            RefKind::After => f.write_str("after"),
        }
    }
}

// ── Public API ─────────────────────────────────────────────────────────────

/// Compose the pipeline for a single stage.
///
/// Applies REQ-3206 directory drop-in precedence (vault > theme) and
/// REQ-3217 `before` / `after` ordering with filename lex-sort as the
/// tiebreaker. Returns a [`StagePipeline`] with enabled hooks ordered for
/// execution, plus bookkeeping for shadowed and disabled hooks so the
/// caller can render diagnostics.
pub fn compose_stage(
    vault_root: &Path,
    theme_hooks_dir: Option<&Path>,
    stage: Stage,
) -> Result<StagePipeline, CompositionError> {
    let theme_entries = match theme_hooks_dir {
        Some(dir) => scan_stage_dir(dir, stage, CompositionSource::Theme)?,
        None => Vec::new(),
    };
    let vault_stage_root = vault_root.join(".zetl").join("hooks");
    let vault_entries = scan_stage_dir(&vault_stage_root, stage, CompositionSource::Vault)?;

    // Apply REQ-3206 precedence: a vault filename shadows the theme entry
    // with the same filename.
    let vault_names: HashSet<String> = vault_entries.iter().map(|h| h.filename.clone()).collect();
    let mut shadowed = Vec::new();
    let mut theme_surviving = Vec::new();
    for hook in theme_entries {
        if vault_names.contains(&hook.filename) {
            shadowed.push(hook);
        } else {
            theme_surviving.push(hook);
        }
    }

    // Union + lex-sort by filename for deterministic order. Source tiebreak
    // (vault vs theme) cannot occur here because collisions have already
    // been resolved above, but we sort deterministically anyway.
    let mut all: Vec<ComposedHook> = theme_surviving.into_iter().chain(vault_entries).collect();
    all.sort_by(|a, b| a.filename.cmp(&b.filename));

    // Partition enabled vs disabled.
    let (enabled, disabled): (Vec<ComposedHook>, Vec<ComposedHook>) =
        all.into_iter().partition(|h| h.is_enabled());

    // Topological sort over enabled hooks respecting before/after.
    let (ordered, warnings) = topo_sort(stage, enabled)?;

    Ok(StagePipeline {
        stage,
        hooks: ordered,
        shadowed,
        disabled,
        warnings,
    })
}

/// Compose the pipelines for every stage. Convenience wrapper that returns
/// one [`StagePipeline`] per [`Stage`] in [`Stage::all`] order.
pub fn compose_all_stages(
    vault_root: &Path,
    theme_hooks_dir: Option<&Path>,
) -> Result<Vec<StagePipeline>, CompositionError> {
    Stage::all()
        .into_iter()
        .map(|s| compose_stage(vault_root, theme_hooks_dir, s))
        .collect()
}

/// Compute the default extension_id for a filename: strip the file
/// extension AND any leading numeric-plus-dash ordering prefix
/// (e.g. `20-tasks.py` → `tasks`, `callouts` → `callouts`).
pub fn default_extension_id(filename: &str) -> String {
    // Strip extension first.
    let stem = match filename.rfind('.') {
        Some(idx) if idx > 0 => &filename[..idx],
        _ => filename,
    };
    // Strip leading `\d+-` if present.
    let bytes = stem.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i > 0 && i < bytes.len() && bytes[i] == b'-' {
        stem[i + 1..].to_string()
    } else {
        stem.to_string()
    }
}

// ── Sidecar manifest (REQ-3217 subset) ─────────────────────────────────────

/// Subset of REQ-3203's manifest format that composition cares about. The
/// full parser (selector, contract, timeouts, ast_version, …) lives in
/// task-manifest-parser; this module only consumes the ordering + id
/// fields. Unknown top-level keys are *ignored* here (not rejected) so
/// composition stays forward-compatible with richer manifests written for
/// the manifest-parser task.
///
/// REQ-3217 canonicalises ordering constraints under a `[ordering]` table.
/// Top-level `before` / `after` keys remain accepted as a convenience alias
/// so existing hook manifests keep working; when both are supplied the two
/// lists are concatenated.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CompositionManifest {
    extension_id: Option<String>,
    before: Option<Vec<String>>,
    after: Option<Vec<String>>,
    optional: Option<bool>,
    ordering: Option<OrderingTable>,
    /// REQ-3221 — declares which ecosystem's AST shape the hook
    /// expects. Composition parses it so the pipeline can look up a
    /// translator at dispatch time; an unknown value is a hard parse
    /// error (handled by [`AstType`]'s serde impl).
    ast_type: Option<AstType>,
    /// REQ-3221 — semver range against the declared ast_type's version
    /// scheme. Composition preserves it verbatim; range evaluation
    /// happens at capability-probe time (task-capability-probe).
    ast_version: Option<String>,
    /// REQ-3224 — behavioural contract block. Only the `preserves`
    /// field is read here; the rest is owned by task-behavioural-
    /// contracts.
    contract: Option<ContractTable>,
    /// REQ-3301 / CON-3312 — ecosystem adapter id (e.g. `"pandoc"`,
    /// `"mdbook"`, `"remark"`). Preserved verbatim; validation against
    /// the ecosystem registry happens at dispatch time, not composition,
    /// so an unknown id still composes but its hook is diagnosed when
    /// actually invoked. `None` means the hook is zetl-native.
    ecosystem: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct OrderingTable {
    before: Option<Vec<String>>,
    after: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ContractTable {
    preserves: Option<Vec<String>>,
}

impl CompositionManifest {
    /// Merge `[ordering].before` with top-level `before` (canonical form
    /// first, alias second; duplicates preserved).
    fn resolved_before(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(t) = &self.ordering {
            if let Some(b) = &t.before {
                out.extend(b.iter().cloned());
            }
        }
        if let Some(b) = &self.before {
            out.extend(b.iter().cloned());
        }
        out
    }

    fn resolved_after(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(t) = &self.ordering {
            if let Some(a) = &t.after {
                out.extend(a.iter().cloned());
            }
        }
        if let Some(a) = &self.after {
            out.extend(a.iter().cloned());
        }
        out
    }
}

fn read_manifest(path: &Path) -> Result<CompositionManifest, CompositionError> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return Ok(CompositionManifest::default()),
    };
    toml::from_str::<CompositionManifest>(&text).map_err(|e| CompositionError::ManifestParse {
        path: path.to_path_buf(),
        error: e.to_string(),
    })
}

// ── Discovery ──────────────────────────────────────────────────────────────

/// Scan one side (vault or theme) of a stage's hooks.
///
/// Looks in two locations (per REQ-3206):
/// - `<root>/<stage>.d/*` — the canonical drop-in directory.
/// - `<root>/<stage>` — a single-file hook, treated as a one-entry `.d/`.
fn scan_stage_dir(
    hooks_root: &Path,
    stage: Stage,
    source: CompositionSource,
) -> Result<Vec<ComposedHook>, CompositionError> {
    let mut out = Vec::new();

    let stage_dir = hooks_root.join(format!("{}.d", stage.as_str()));
    if stage_dir.is_dir() {
        for entry in read_dir_sorted(&stage_dir) {
            if !entry.is_file() {
                continue;
            }
            let file_name = match entry.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            // Skip sidecar manifests themselves; they are parsed alongside
            // their executable, not listed as hooks.
            if file_name.ends_with(".toml") {
                continue;
            }
            // Skip hidden/dotfiles.
            if file_name.starts_with('.') {
                continue;
            }
            let hook = build_hook(entry, stage, source)?;
            out.push(hook);
        }
    }

    // Single-file hook `<root>/<stage>` (no `.d/`), REQ-3206 final paragraph.
    let single = hooks_root.join(stage.as_str());
    if single.is_file() {
        let file_name = stage.as_str().to_string();
        let hook = build_hook_with_name(single, file_name, stage, source)?;
        out.push(hook);
    }

    Ok(out)
}

fn build_hook(
    path: PathBuf,
    stage: Stage,
    source: CompositionSource,
) -> Result<ComposedHook, CompositionError> {
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    build_hook_with_name(path, filename, stage, source)
}

fn build_hook_with_name(
    path: PathBuf,
    filename: String,
    stage: Stage,
    source: CompositionSource,
) -> Result<ComposedHook, CompositionError> {
    let disabled = disabled_reason(&path);
    let manifest_path = {
        let mut p = path.clone();
        let with_toml = match p.extension().and_then(|e| e.to_str()) {
            Some(_) => {
                let mut buf = p.as_os_str().to_owned();
                buf.push(".toml");
                PathBuf::from(buf)
            }
            None => {
                p.set_extension("toml");
                p
            }
        };
        if with_toml.is_file() {
            Some(with_toml)
        } else {
            None
        }
    };

    let manifest = match &manifest_path {
        Some(p) => read_manifest(p)?,
        None => CompositionManifest::default(),
    };

    let before = manifest.resolved_before();
    let after = manifest.resolved_after();
    let extension_id = manifest
        .extension_id
        .clone()
        .unwrap_or_else(|| default_extension_id(&filename));
    let optional = manifest.optional.unwrap_or(false);
    let ast_type = manifest.ast_type.unwrap_or_default();
    let ast_version = manifest.ast_version.clone();
    let preserves = manifest
        .contract
        .as_ref()
        .and_then(|c| c.preserves.clone())
        .unwrap_or_else(|| {
            ast_type
                .default_preserves()
                .iter()
                .map(|s| (*s).to_string())
                .collect()
        });

    let ecosystem = manifest.ecosystem.clone();

    Ok(ComposedHook {
        stage,
        filename,
        extension_id,
        path,
        manifest_path,
        source,
        before,
        after,
        optional,
        ast_type,
        ast_version,
        preserves,
        ecosystem,
        disabled,
    })
}

fn read_dir_sorted(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(r) => r.flatten().map(|e| e.path()).collect(),
        Err(_) => return Vec::new(),
    };
    entries.sort();
    entries
}

/// Cheap REQ-3206 pre-probe disable checks — `None` means the hook passes
/// the pre-probe filters (it may still be disabled later by the probe
/// itself per REQ-3216, which is out of scope here).
fn disabled_reason(path: &Path) -> Option<DisabledReason> {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return Some(DisabledReason::NotExecutableAndNoShebang),
    };
    if meta.len() == 0 {
        return Some(DisabledReason::EmptyFile);
    }
    if !is_runnable(path, &meta) {
        return Some(DisabledReason::NotExecutableAndNoShebang);
    }
    None
}

#[cfg(unix)]
fn is_runnable(path: &Path, meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    if meta.permissions().mode() & 0o111 != 0 {
        return true;
    }
    has_shebang(path)
}

#[cfg(windows)]
fn is_runnable(path: &Path, _meta: &std::fs::Metadata) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => matches!(
            ext.to_ascii_lowercase().as_str(),
            "exe" | "bat" | "cmd" | "com" | "ps1"
        ),
        None => has_shebang(path),
    }
}

fn has_shebang(path: &Path) -> bool {
    use std::io::Read;
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut buf = [0u8; 2];
    match f.read_exact(&mut buf) {
        Ok(()) => buf == *b"#!",
        Err(_) => false,
    }
}

// ── Topological sort (REQ-3217 / CON-3217) ────────────────────────────────

/// Topologically sort the enabled hook set respecting `before` / `after`
/// constraints, with filename lex-sort as the tiebreaker between hooks that
/// are otherwise unordered.
fn topo_sort(
    stage: Stage,
    hooks: Vec<ComposedHook>,
) -> Result<(Vec<ComposedHook>, Vec<String>), CompositionError> {
    if hooks.is_empty() {
        return Ok((hooks, Vec::new()));
    }

    // Index by extension_id. When two hooks share an id (allowed for
    // template-vars coalescing per REQ-3214), before/after references to
    // that id apply to every matching hook. We track a list of indices.
    let mut id_to_indices: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, h) in hooks.iter().enumerate() {
        id_to_indices
            .entry(h.extension_id.clone())
            .or_default()
            .push(i);
    }

    let mut warnings = Vec::new();

    // Build edges: u → v means "u must run before v".
    //   hook H with before=[X]: edge H → X (for each hook matching id X)
    //   hook H with after=[Y]:  edge Y → H (for each hook matching id Y)
    let mut out_edges: Vec<HashSet<usize>> = vec![HashSet::new(); hooks.len()];
    let mut in_degree: Vec<usize> = vec![0; hooks.len()];

    for (i, hook) in hooks.iter().enumerate() {
        for ref_id in &hook.before {
            add_constraint_edges(
                stage,
                &hooks,
                &id_to_indices,
                i,
                ref_id,
                RefKind::Before,
                &mut out_edges,
                &mut in_degree,
                &mut warnings,
                // H before X:
                |h, x| (h, x),
            )?;
        }
        for ref_id in &hook.after {
            add_constraint_edges(
                stage,
                &hooks,
                &id_to_indices,
                i,
                ref_id,
                RefKind::After,
                &mut out_edges,
                &mut in_degree,
                &mut warnings,
                // Y before H:
                |h, y| (y, h),
            )?;
        }
    }

    // Kahn's algorithm with filename tiebreaker. Use a BTreeSet keyed on
    // (filename, index) so the first element is always the lexicographically
    // smallest filename among the current in-degree-zero frontier.
    let mut frontier: std::collections::BTreeSet<(String, usize)> =
        std::collections::BTreeSet::new();
    for (i, d) in in_degree.iter().enumerate() {
        if *d == 0 {
            frontier.insert((hooks[i].filename.clone(), i));
        }
    }

    let mut result_indices = Vec::with_capacity(hooks.len());
    while let Some(first) = frontier.iter().next().cloned() {
        frontier.remove(&first);
        let (_, i) = first;
        result_indices.push(i);
        // Snapshot neighbours to avoid borrowing `out_edges` while mutating.
        let neighbours: Vec<usize> = out_edges[i].iter().copied().collect();
        for v in neighbours {
            in_degree[v] -= 1;
            if in_degree[v] == 0 {
                frontier.insert((hooks[v].filename.clone(), v));
            }
        }
    }

    if result_indices.len() != hooks.len() {
        // Cycle — recover a cycle path using DFS over the residual graph.
        let cycle_path_indices = find_cycle(&out_edges, &in_degree).unwrap_or_default();
        let path = cycle_path_indices
            .iter()
            .map(|&i| hooks[i].extension_id.clone())
            .collect::<Vec<_>>();
        let files = cycle_path_indices
            .iter()
            .filter_map(|&i| {
                hooks[i]
                    .manifest_path
                    .clone()
                    .or_else(|| Some(hooks[i].path.clone()))
            })
            .collect::<Vec<_>>();
        return Err(CompositionError::Cycle { stage, path, files });
    }

    // Move ordered hooks into the final vector.
    // Use a Vec<Option<ComposedHook>> to permute without cloning.
    let mut slot: Vec<Option<ComposedHook>> = hooks.into_iter().map(Some).collect();
    let ordered: Vec<ComposedHook> = result_indices
        .iter()
        .map(|&i| slot[i].take().expect("each index drained once"))
        .collect();

    Ok((ordered, warnings))
}

#[allow(clippy::too_many_arguments)]
fn add_constraint_edges<F>(
    stage: Stage,
    hooks: &[ComposedHook],
    id_to_indices: &HashMap<String, Vec<usize>>,
    hook_index: usize,
    ref_id: &str,
    kind: RefKind,
    out_edges: &mut [HashSet<usize>],
    in_degree: &mut [usize],
    warnings: &mut Vec<String>,
    edge_for: F,
) -> Result<(), CompositionError>
where
    F: Fn(usize, usize) -> (usize, usize),
{
    match id_to_indices.get(ref_id) {
        Some(targets) => {
            for &t in targets {
                if t == hook_index {
                    continue; // self-reference is a no-op
                }
                let (from, to) = edge_for(hook_index, t);
                if out_edges[from].insert(to) {
                    in_degree[to] += 1;
                }
            }
            Ok(())
        }
        None => {
            let hook = &hooks[hook_index];
            if hook.optional {
                warnings.push(format!(
                    "hook '{}' ({stage}) declares {kind} = [{:?}] but no such hook is present (optional; constraint dropped)",
                    hook.extension_id, ref_id
                ));
                Ok(())
            } else {
                Err(CompositionError::MissingRequiredRef {
                    stage,
                    hook_id: hook.extension_id.clone(),
                    ref_id: ref_id.to_string(),
                    kind,
                    manifest: hook.manifest_path.clone(),
                })
            }
        }
    }
}

/// Find one cycle in the residual graph. Returns indices in cycle order
/// (the last element is the first repeated node so the diagnostic can print
/// `a → b → a`). Used only in the error path, so simplicity over speed.
fn find_cycle(out_edges: &[HashSet<usize>], in_degree: &[usize]) -> Option<Vec<usize>> {
    let n = out_edges.len();
    // Consider only vertices that still have positive in-degree after
    // Kahn's — those are on or downstream of a cycle.
    let residual: HashSet<usize> = (0..n).filter(|&i| in_degree[i] > 0).collect();
    if residual.is_empty() {
        return None;
    }

    // DFS from each residual vertex looking for a back-edge.
    for &start in &residual {
        let mut stack: Vec<(usize, usize)> = vec![(start, 0)]; // (node, neighbour cursor)
        let mut on_path: HashSet<usize> = HashSet::new();
        let mut path: Vec<usize> = Vec::new();
        on_path.insert(start);
        path.push(start);

        while !stack.is_empty() {
            let top = stack.len() - 1;
            let (node, cursor) = stack[top];
            let neighbours: Vec<usize> = out_edges[node]
                .iter()
                .copied()
                .filter(|n| residual.contains(n))
                .collect();
            if cursor < neighbours.len() {
                let next = neighbours[cursor];
                stack[top].1 = cursor + 1;
                if on_path.contains(&next) {
                    // Found a cycle: splice path from `next` to end.
                    let start_idx = path.iter().position(|&p| p == next).unwrap();
                    let mut cycle: Vec<usize> = path[start_idx..].to_vec();
                    cycle.push(next);
                    return Some(cycle);
                }
                on_path.insert(next);
                path.push(next);
                stack.push((next, 0));
            } else {
                stack.pop();
                path.pop();
                on_path.remove(&node);
            }
        }
    }
    None
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    fn write_exe(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
        path
    }

    fn write_empty(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, "").unwrap();
        path
    }

    fn write_manifest(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        path
    }

    fn make_stage_dirs(tmp: &Path) -> (PathBuf, PathBuf) {
        let vault = tmp.join("vault");
        let theme = tmp.join("theme");
        let vault_tx = vault.join(".zetl").join("hooks").join("transform.d");
        let theme_tx = theme.join("transform.d");
        fs::create_dir_all(&vault_tx).unwrap();
        fs::create_dir_all(&theme_tx).unwrap();
        (vault_tx, theme_tx)
    }

    // ── default_extension_id ─────────────────────────────────────────────

    #[test]
    fn default_ext_id_strips_prefix_and_extension() {
        assert_eq!(default_extension_id("20-tasks.py"), "tasks");
        assert_eq!(default_extension_id("10-callouts.py"), "callouts");
        assert_eq!(default_extension_id("callouts.py"), "callouts");
        assert_eq!(default_extension_id("callouts"), "callouts");
        assert_eq!(default_extension_id("05-prelude.py"), "prelude");
    }

    #[test]
    fn default_ext_id_only_strips_prefix_if_dash_follows() {
        // `20tasks.py` has no dash → no prefix stripped (and no extension)
        assert_eq!(default_extension_id("20tasks.py"), "20tasks");
        // `2020-report.py` → digits then dash → strip → `report`
        assert_eq!(default_extension_id("2020-report.py"), "report");
        // `-foo.py` — no leading digits, dash stays
        assert_eq!(default_extension_id("-foo.py"), "-foo");
    }

    // ── discovery ────────────────────────────────────────────────────────

    #[test]
    fn discovers_vault_and_theme_hooks_in_stage_dir() {
        let tmp = TempDir::new().unwrap();
        let (vault_tx, theme_tx) = make_stage_dirs(tmp.path());
        let vault_root = tmp.path().join("vault");
        let theme_root = tmp.path().join("theme");

        write_exe(&vault_tx, "20-tasks.py", "#!/bin/sh\ntrue\n");
        write_exe(&theme_tx, "10-callouts.py", "#!/bin/sh\ntrue\n");
        write_exe(&theme_tx, "30-admonition.py", "#!/bin/sh\ntrue\n");

        let pipe = compose_stage(&vault_root, Some(&theme_root), Stage::Transform).unwrap();

        // Three enabled hooks, lex order by filename.
        let names: Vec<&str> = pipe.hooks.iter().map(|h| h.filename.as_str()).collect();
        assert_eq!(
            names,
            vec!["10-callouts.py", "20-tasks.py", "30-admonition.py"]
        );

        // Sources preserved.
        assert_eq!(pipe.hooks[0].source, CompositionSource::Theme);
        assert_eq!(pipe.hooks[1].source, CompositionSource::Vault);
        assert_eq!(pipe.hooks[2].source, CompositionSource::Theme);

        // Extension ids derived from filenames.
        assert_eq!(pipe.hooks[0].extension_id, "callouts");
        assert_eq!(pipe.hooks[1].extension_id, "tasks");
        assert_eq!(pipe.hooks[2].extension_id, "admonition");
    }

    #[test]
    fn vault_filename_replaces_theme_filename() {
        let tmp = TempDir::new().unwrap();
        let (vault_tx, theme_tx) = make_stage_dirs(tmp.path());
        let vault_root = tmp.path().join("vault");
        let theme_root = tmp.path().join("theme");

        write_exe(&vault_tx, "20-tasks.py", "#!/bin/sh\necho vault\n");
        write_exe(&theme_tx, "20-tasks.py", "#!/bin/sh\necho theme\n");
        write_exe(&theme_tx, "30-admonition.py", "#!/bin/sh\ntrue\n");

        let pipe = compose_stage(&vault_root, Some(&theme_root), Stage::Transform).unwrap();

        // tasks.py: vault version survives; theme version in `shadowed`.
        assert_eq!(pipe.hooks.len(), 2);
        let tasks = pipe
            .hooks
            .iter()
            .find(|h| h.filename == "20-tasks.py")
            .unwrap();
        assert_eq!(tasks.source, CompositionSource::Vault);

        assert_eq!(pipe.shadowed.len(), 1);
        assert_eq!(pipe.shadowed[0].filename, "20-tasks.py");
        assert_eq!(pipe.shadowed[0].source, CompositionSource::Theme);
    }

    #[test]
    fn empty_vault_file_shadows_theme_and_is_disabled() {
        // CON-3206 worked example.
        let tmp = TempDir::new().unwrap();
        let (vault_tx, theme_tx) = make_stage_dirs(tmp.path());
        let vault_root = tmp.path().join("vault");
        let theme_root = tmp.path().join("theme");

        // Vault: 20-tasks.py (custom); 10-callouts.py (empty).
        write_exe(&vault_tx, "20-tasks.py", "#!/bin/sh\necho vault\n");
        write_empty(&vault_tx, "10-callouts.py");

        // Theme: canonical 10-callouts.py, 20-tasks.py, 30-admonition.py.
        write_exe(&theme_tx, "10-callouts.py", "#!/bin/sh\necho theme\n");
        write_exe(&theme_tx, "20-tasks.py", "#!/bin/sh\necho theme\n");
        write_exe(&theme_tx, "30-admonition.py", "#!/bin/sh\ntrue\n");

        let pipe = compose_stage(&vault_root, Some(&theme_root), Stage::Transform).unwrap();

        // Effective pipeline from CON-3206:
        //   1. 10-callouts.py — vault's empty file → disabled.
        //   2. 20-tasks.py    — vault's version → runs.
        //   3. 30-admonition.py — theme's version → runs.
        let enabled_names: Vec<&str> = pipe.hooks.iter().map(|h| h.filename.as_str()).collect();
        assert_eq!(enabled_names, vec!["20-tasks.py", "30-admonition.py"]);
        assert_eq!(
            pipe.hooks
                .iter()
                .find(|h| h.filename == "20-tasks.py")
                .unwrap()
                .source,
            CompositionSource::Vault
        );
        assert_eq!(
            pipe.hooks
                .iter()
                .find(|h| h.filename == "30-admonition.py")
                .unwrap()
                .source,
            CompositionSource::Theme
        );

        // 10-callouts.py is present in `disabled`, from vault, with EmptyFile reason.
        assert_eq!(pipe.disabled.len(), 1);
        let disabled = &pipe.disabled[0];
        assert_eq!(disabled.filename, "10-callouts.py");
        assert_eq!(disabled.source, CompositionSource::Vault);
        assert_eq!(disabled.disabled, Some(DisabledReason::EmptyFile));

        // Theme's 10-callouts is in `shadowed` (replaced by vault's empty).
        assert!(pipe
            .shadowed
            .iter()
            .any(|h| h.filename == "10-callouts.py" && h.source == CompositionSource::Theme));
    }

    #[test]
    fn non_executable_without_shebang_is_disabled() {
        let tmp = TempDir::new().unwrap();
        let (vault_tx, _) = make_stage_dirs(tmp.path());
        let vault_root = tmp.path().join("vault");

        // Not executable, no shebang → disabled.
        let p = vault_tx.join("10-plain.py");
        fs::write(&p, "print('hi')\n").unwrap();
        let mut perms = fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o644);
        fs::set_permissions(&p, perms).unwrap();

        let pipe = compose_stage(&vault_root, None, Stage::Transform).unwrap();
        assert!(pipe.hooks.is_empty());
        assert_eq!(pipe.disabled.len(), 1);
        assert_eq!(
            pipe.disabled[0].disabled,
            Some(DisabledReason::NotExecutableAndNoShebang)
        );
    }

    #[test]
    fn non_executable_with_shebang_is_enabled() {
        // REQ-3206: disabled iff 0-byte OR (not-exec AND no-shebang). A
        // non-exec file WITH a shebang should still appear (we can't run
        // it but the spec's pre-probe filter only disables the unruly
        // case, leaving probe to catch the rest).
        let tmp = TempDir::new().unwrap();
        let (vault_tx, _) = make_stage_dirs(tmp.path());
        let vault_root = tmp.path().join("vault");

        let p = vault_tx.join("10-script.py");
        fs::write(&p, "#!/bin/sh\necho hi\n").unwrap();
        let mut perms = fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o644);
        fs::set_permissions(&p, perms).unwrap();

        let pipe = compose_stage(&vault_root, None, Stage::Transform).unwrap();
        assert_eq!(pipe.hooks.len(), 1);
    }

    #[test]
    fn manifest_sidecar_overrides_extension_id_and_orders() {
        let tmp = TempDir::new().unwrap();
        let (vault_tx, _) = make_stage_dirs(tmp.path());
        let vault_root = tmp.path().join("vault");

        write_exe(&vault_tx, "10-foo.py", "#!/bin/sh\ntrue\n");
        write_manifest(
            &vault_tx,
            "10-foo.py.toml",
            r#"extension_id = "my_foo"
before = ["bar"]
optional = true
"#,
        );
        write_exe(&vault_tx, "20-bar.py", "#!/bin/sh\ntrue\n");

        let pipe = compose_stage(&vault_root, None, Stage::Transform).unwrap();
        let foo = pipe
            .hooks
            .iter()
            .find(|h| h.filename == "10-foo.py")
            .unwrap();
        assert_eq!(foo.extension_id, "my_foo");
        assert_eq!(foo.before, vec!["bar".to_string()]);
        assert!(foo.optional);
    }

    #[test]
    fn single_file_hook_without_d_directory_works() {
        // REQ-3206: single-file hooks at `.zetl/hooks/<stage>` are treated
        // as a one-entry `.d/`.
        let tmp = TempDir::new().unwrap();
        let vault_root = tmp.path().join("vault");
        let hooks_dir = vault_root.join(".zetl").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        let p = hooks_dir.join("transform");
        fs::write(&p, "#!/bin/sh\ntrue\n").unwrap();
        let mut perms = fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&p, perms).unwrap();

        let pipe = compose_stage(&vault_root, None, Stage::Transform).unwrap();
        assert_eq!(pipe.hooks.len(), 1);
        assert_eq!(pipe.hooks[0].filename, "transform");
        assert_eq!(pipe.hooks[0].extension_id, "transform");
    }

    #[test]
    fn sidecar_toml_is_not_itself_a_hook() {
        let tmp = TempDir::new().unwrap();
        let (vault_tx, _) = make_stage_dirs(tmp.path());
        let vault_root = tmp.path().join("vault");

        write_exe(&vault_tx, "10-foo.py", "#!/bin/sh\ntrue\n");
        write_manifest(&vault_tx, "10-foo.py.toml", "optional = false\n");

        let pipe = compose_stage(&vault_root, None, Stage::Transform).unwrap();
        assert_eq!(pipe.hooks.len(), 1);
        assert_eq!(pipe.hooks[0].filename, "10-foo.py");
    }

    #[test]
    fn no_hooks_directory_yields_empty_pipeline() {
        let tmp = TempDir::new().unwrap();
        let vault_root = tmp.path().join("vault");
        fs::create_dir_all(&vault_root).unwrap();

        let pipe = compose_stage(&vault_root, None, Stage::Transform).unwrap();
        assert!(pipe.hooks.is_empty());
        assert!(pipe.disabled.is_empty());
        assert!(pipe.shadowed.is_empty());
    }

    #[test]
    fn compose_all_stages_covers_every_stage() {
        let tmp = TempDir::new().unwrap();
        let vault_root = tmp.path().join("vault");
        let pp = vault_root.join(".zetl").join("hooks").join("pre-parse.d");
        let tx = vault_root.join(".zetl").join("hooks").join("transform.d");
        let pr = vault_root.join(".zetl").join("hooks").join("post-render.d");
        fs::create_dir_all(&pp).unwrap();
        fs::create_dir_all(&tx).unwrap();
        fs::create_dir_all(&pr).unwrap();

        write_exe(&pp, "10-a.py", "#!/bin/sh\ntrue\n");
        write_exe(&tx, "10-b.py", "#!/bin/sh\ntrue\n");
        write_exe(&pr, "10-c.py", "#!/bin/sh\ntrue\n");

        let pipes = compose_all_stages(&vault_root, None).unwrap();
        assert_eq!(pipes.len(), 3);
        assert_eq!(pipes[0].stage, Stage::PreParse);
        assert_eq!(pipes[1].stage, Stage::Transform);
        assert_eq!(pipes[2].stage, Stage::PostRender);
        assert_eq!(pipes[0].hooks.len(), 1);
        assert_eq!(pipes[1].hooks.len(), 1);
        assert_eq!(pipes[2].hooks.len(), 1);
    }

    // ── ordering / topological sort (CON-3217) ───────────────────────────

    #[test]
    fn con_3217_worked_example() {
        // Five hooks in transform.d/:
        //   05-prelude.py   prelude    (no ordering)
        //   10-callouts.py  callouts   (no ordering)
        //   10-tasks.py     tasks      after=[callouts]
        //   20-admon.py     admon      before=[tasks]
        //   30-fini.py      fini       (no ordering)
        //
        // Expected final order: prelude, callouts, admon, tasks, fini.
        let tmp = TempDir::new().unwrap();
        let (vault_tx, _) = make_stage_dirs(tmp.path());
        let vault_root = tmp.path().join("vault");

        write_exe(&vault_tx, "05-prelude.py", "#!/bin/sh\ntrue\n");
        write_exe(&vault_tx, "10-callouts.py", "#!/bin/sh\ntrue\n");
        write_exe(&vault_tx, "10-tasks.py", "#!/bin/sh\ntrue\n");
        write_manifest(&vault_tx, "10-tasks.py.toml", r#"after = ["callouts"]"#);
        write_exe(&vault_tx, "20-admon.py", "#!/bin/sh\ntrue\n");
        write_manifest(&vault_tx, "20-admon.py.toml", r#"before = ["tasks"]"#);
        write_exe(&vault_tx, "30-fini.py", "#!/bin/sh\ntrue\n");

        let pipe = compose_stage(&vault_root, None, Stage::Transform).unwrap();
        let ids: Vec<&str> = pipe.hooks.iter().map(|h| h.extension_id.as_str()).collect();
        assert_eq!(ids, vec!["prelude", "callouts", "admon", "tasks", "fini"]);
    }

    #[test]
    fn cycle_detected_with_path() {
        // callouts.after=[tasks], tasks.after=[callouts] → A→B, B→A cycle.
        let tmp = TempDir::new().unwrap();
        let (vault_tx, _) = make_stage_dirs(tmp.path());
        let vault_root = tmp.path().join("vault");

        write_exe(&vault_tx, "10-callouts.py", "#!/bin/sh\ntrue\n");
        write_manifest(&vault_tx, "10-callouts.py.toml", r#"after = ["tasks"]"#);
        write_exe(&vault_tx, "20-tasks.py", "#!/bin/sh\ntrue\n");
        write_manifest(&vault_tx, "20-tasks.py.toml", r#"after = ["callouts"]"#);

        let err = compose_stage(&vault_root, None, Stage::Transform).unwrap_err();
        match err {
            CompositionError::Cycle { path, .. } => {
                // Path starts and ends at the same id.
                assert!(path.len() >= 2);
                assert_eq!(path.first(), path.last());
                let set: HashSet<&str> = path.iter().map(|s| s.as_str()).collect();
                assert!(set.contains("callouts"));
                assert!(set.contains("tasks"));
            }
            other => panic!("expected Cycle, got {other:?}"),
        }
    }

    #[test]
    fn missing_required_ref_errors() {
        let tmp = TempDir::new().unwrap();
        let (vault_tx, _) = make_stage_dirs(tmp.path());
        let vault_root = tmp.path().join("vault");

        write_exe(&vault_tx, "10-foo.py", "#!/bin/sh\ntrue\n");
        write_manifest(&vault_tx, "10-foo.py.toml", r#"after = ["ghost"]"#);

        let err = compose_stage(&vault_root, None, Stage::Transform).unwrap_err();
        match err {
            CompositionError::MissingRequiredRef {
                hook_id,
                ref_id,
                kind,
                ..
            } => {
                assert_eq!(hook_id, "foo");
                assert_eq!(ref_id, "ghost");
                assert_eq!(kind, RefKind::After);
            }
            other => panic!("expected MissingRequiredRef, got {other:?}"),
        }
    }

    #[test]
    fn missing_optional_ref_warns_and_drops_constraint() {
        let tmp = TempDir::new().unwrap();
        let (vault_tx, _) = make_stage_dirs(tmp.path());
        let vault_root = tmp.path().join("vault");

        write_exe(&vault_tx, "10-foo.py", "#!/bin/sh\ntrue\n");
        write_manifest(
            &vault_tx,
            "10-foo.py.toml",
            r#"after = ["ghost"]
optional = true
"#,
        );

        let pipe = compose_stage(&vault_root, None, Stage::Transform).unwrap();
        assert_eq!(pipe.hooks.len(), 1);
        assert_eq!(pipe.hooks[0].extension_id, "foo");
        assert!(pipe.warnings.iter().any(|w| w.contains("ghost")));
    }

    #[test]
    fn malformed_manifest_is_parse_error() {
        let tmp = TempDir::new().unwrap();
        let (vault_tx, _) = make_stage_dirs(tmp.path());
        let vault_root = tmp.path().join("vault");

        write_exe(&vault_tx, "10-foo.py", "#!/bin/sh\ntrue\n");
        write_manifest(&vault_tx, "10-foo.py.toml", "this is = not = valid toml\n");

        let err = compose_stage(&vault_root, None, Stage::Transform).unwrap_err();
        match err {
            CompositionError::ManifestParse { path, .. } => {
                assert!(path.ends_with("10-foo.py.toml"));
            }
            other => panic!("expected ManifestParse, got {other:?}"),
        }
    }

    #[test]
    fn unknown_manifest_fields_are_ignored() {
        // Composition uses only extension_id/before/after/optional. The
        // full manifest parser (task-manifest-parser) owns stricter
        // handling; composition must stay forward-compatible so an
        // extra field like `timeout_ms` doesn't break discovery.
        let tmp = TempDir::new().unwrap();
        let (vault_tx, _) = make_stage_dirs(tmp.path());
        let vault_root = tmp.path().join("vault");

        write_exe(&vault_tx, "10-foo.py", "#!/bin/sh\ntrue\n");
        write_manifest(
            &vault_tx,
            "10-foo.py.toml",
            r#"timeout_ms = 100
[select]
include = ["**/*.md"]
"#,
        );

        let pipe = compose_stage(&vault_root, None, Stage::Transform).unwrap();
        assert_eq!(pipe.hooks.len(), 1);
    }

    #[test]
    fn filename_tiebreaker_sorts_unconstrained_hooks() {
        let tmp = TempDir::new().unwrap();
        let (vault_tx, _) = make_stage_dirs(tmp.path());
        let vault_root = tmp.path().join("vault");

        write_exe(&vault_tx, "c.py", "#!/bin/sh\ntrue\n");
        write_exe(&vault_tx, "a.py", "#!/bin/sh\ntrue\n");
        write_exe(&vault_tx, "b.py", "#!/bin/sh\ntrue\n");

        let pipe = compose_stage(&vault_root, None, Stage::Transform).unwrap();
        let names: Vec<&str> = pipe.hooks.iter().map(|h| h.filename.as_str()).collect();
        assert_eq!(names, vec!["a.py", "b.py", "c.py"]);
    }

    #[test]
    fn before_constraint_wins_over_filename_order() {
        // CON-3217 remark: `20-admon.py` sorts *after* `10-tasks.py`
        // lexicographically, but `admon.before = [tasks]` overrides the
        // filename order. This microcase isolates that property.
        let tmp = TempDir::new().unwrap();
        let (vault_tx, _) = make_stage_dirs(tmp.path());
        let vault_root = tmp.path().join("vault");

        write_exe(&vault_tx, "10-tasks.py", "#!/bin/sh\ntrue\n");
        write_exe(&vault_tx, "20-admon.py", "#!/bin/sh\ntrue\n");
        write_manifest(&vault_tx, "20-admon.py.toml", r#"before = ["tasks"]"#);

        let pipe = compose_stage(&vault_root, None, Stage::Transform).unwrap();
        let ids: Vec<&str> = pipe.hooks.iter().map(|h| h.extension_id.as_str()).collect();
        assert_eq!(ids, vec!["admon", "tasks"]);
    }

    // ── REQ-3217: `[ordering]` table manifest form ────────────────────────

    #[test]
    fn ordering_table_before_and_after_are_accepted() {
        // Canonical REQ-3217 form: constraints live under `[ordering]`.
        let tmp = TempDir::new().unwrap();
        let (vault_tx, _) = make_stage_dirs(tmp.path());
        let vault_root = tmp.path().join("vault");

        write_exe(&vault_tx, "10-tasks.py", "#!/bin/sh\ntrue\n");
        write_manifest(
            &vault_tx,
            "10-tasks.py.toml",
            r#"[ordering]
after = ["callouts"]
"#,
        );
        write_exe(&vault_tx, "20-admon.py", "#!/bin/sh\ntrue\n");
        write_manifest(
            &vault_tx,
            "20-admon.py.toml",
            r#"[ordering]
before = ["tasks"]
"#,
        );
        write_exe(&vault_tx, "30-callouts.py", "#!/bin/sh\ntrue\n");

        let pipe = compose_stage(&vault_root, None, Stage::Transform).unwrap();
        let ids: Vec<&str> = pipe.hooks.iter().map(|h| h.extension_id.as_str()).collect();
        // callouts has no constraints, lex-last filename, but tasks.after
        // forces callouts before tasks; admon.before forces admon before
        // tasks. No constraint between callouts and admon → filename order:
        // `20-admon.py` < `30-callouts.py`, so admon precedes callouts.
        assert_eq!(ids, vec!["admon", "callouts", "tasks"]);
    }

    #[test]
    fn ordering_table_and_top_level_merge() {
        // When both forms are present, we accept their union so authors
        // migrating from one form to the other don't lose constraints.
        let tmp = TempDir::new().unwrap();
        let (vault_tx, _) = make_stage_dirs(tmp.path());
        let vault_root = tmp.path().join("vault");

        write_exe(&vault_tx, "10-a.py", "#!/bin/sh\ntrue\n");
        write_exe(&vault_tx, "20-b.py", "#!/bin/sh\ntrue\n");
        write_exe(&vault_tx, "30-c.py", "#!/bin/sh\ntrue\n");
        write_manifest(
            &vault_tx,
            "30-c.py.toml",
            r#"after = ["a"]

[ordering]
after = ["b"]
"#,
        );

        let pipe = compose_stage(&vault_root, None, Stage::Transform).unwrap();
        let ids: Vec<&str> = pipe.hooks.iter().map(|h| h.extension_id.as_str()).collect();
        // Both constraints must hold: a < c and b < c.
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    /// TEST-3217: CON-3217 worked example reproduced using the canonical
    /// `[ordering]` table form. Same expected order as the top-level form
    /// test, proving the two manifest shapes are semantically equivalent.
    #[test]
    fn test_3217_con_worked_example_ordering_table() {
        let tmp = TempDir::new().unwrap();
        let (vault_tx, _) = make_stage_dirs(tmp.path());
        let vault_root = tmp.path().join("vault");

        write_exe(&vault_tx, "05-prelude.py", "#!/bin/sh\ntrue\n");
        write_exe(&vault_tx, "10-callouts.py", "#!/bin/sh\ntrue\n");
        write_exe(&vault_tx, "10-tasks.py", "#!/bin/sh\ntrue\n");
        write_manifest(
            &vault_tx,
            "10-tasks.py.toml",
            r#"[ordering]
after = ["callouts"]
"#,
        );
        write_exe(&vault_tx, "20-admon.py", "#!/bin/sh\ntrue\n");
        write_manifest(
            &vault_tx,
            "20-admon.py.toml",
            r#"[ordering]
before = ["tasks"]
"#,
        );
        write_exe(&vault_tx, "30-fini.py", "#!/bin/sh\ntrue\n");

        let pipe = compose_stage(&vault_root, None, Stage::Transform).unwrap();
        let ids: Vec<&str> = pipe.hooks.iter().map(|h| h.extension_id.as_str()).collect();
        assert_eq!(ids, vec!["prelude", "callouts", "admon", "tasks", "fini"]);
    }

    #[test]
    fn ordering_table_cycle_detected_at_load_time() {
        // Two hooks referencing each other via [ordering] → cycle.
        let tmp = TempDir::new().unwrap();
        let (vault_tx, _) = make_stage_dirs(tmp.path());
        let vault_root = tmp.path().join("vault");

        write_exe(&vault_tx, "10-callouts.py", "#!/bin/sh\ntrue\n");
        write_manifest(
            &vault_tx,
            "10-callouts.py.toml",
            r#"[ordering]
after = ["tasks"]
"#,
        );
        write_exe(&vault_tx, "20-tasks.py", "#!/bin/sh\ntrue\n");
        write_manifest(
            &vault_tx,
            "20-tasks.py.toml",
            r#"[ordering]
after = ["callouts"]
"#,
        );

        let err = compose_stage(&vault_root, None, Stage::Transform).unwrap_err();
        match err {
            CompositionError::Cycle { path, files, .. } => {
                assert!(path.len() >= 2);
                assert_eq!(path.first(), path.last());
                assert!(!files.is_empty());
                // Diagnostic includes both hook ids.
                let set: HashSet<&str> = path.iter().map(|s| s.as_str()).collect();
                assert!(set.contains("callouts"));
                assert!(set.contains("tasks"));
            }
            other => panic!("expected Cycle, got {other:?}"),
        }
    }

    // ── REQ-3221: ast_type / ast_version / preserves ─────────────────────

    #[test]
    fn ast_type_defaults_to_zetl_ext_when_manifest_absent() {
        let tmp = TempDir::new().unwrap();
        let (vault_tx, _) = make_stage_dirs(tmp.path());
        let vault_root = tmp.path().join("vault");

        write_exe(&vault_tx, "10-foo.py", "#!/bin/sh\ntrue\n");

        let pipe = compose_stage(&vault_root, None, Stage::Transform).unwrap();
        assert_eq!(pipe.hooks.len(), 1);
        assert_eq!(pipe.hooks[0].ast_type, AstType::ZetlExt);
        assert!(pipe.hooks[0].ast_version.is_none());
        // zetl-ext default preserves is empty per REQ-3221.
        assert!(pipe.hooks[0].preserves.is_empty());
    }

    #[test]
    fn ast_type_parsed_from_sidecar_manifest() {
        let tmp = TempDir::new().unwrap();
        let (vault_tx, _) = make_stage_dirs(tmp.path());
        let vault_root = tmp.path().join("vault");

        write_exe(&vault_tx, "10-mdast.py", "#!/bin/sh\ntrue\n");
        write_manifest(
            &vault_tx,
            "10-mdast.py.toml",
            r#"ast_type = "mdast-ext"
ast_version = ">=1.0 <2"
"#,
        );

        let pipe = compose_stage(&vault_root, None, Stage::Transform).unwrap();
        assert_eq!(pipe.hooks.len(), 1);
        assert_eq!(pipe.hooks[0].ast_type, AstType::MdastExt);
        assert_eq!(pipe.hooks[0].ast_version.as_deref(), Some(">=1.0 <2"));
        // Foreign-ext default preserves per REQ-3221.
        assert_eq!(
            pipe.hooks[0].preserves,
            vec![
                "Wikilink".to_string(),
                "Embed".to_string(),
                "SplBlock".to_string()
            ]
        );
    }

    #[test]
    fn ast_type_short_alias_accepted() {
        // Authors may spell `mdast` or `pandoc-types` for ergonomic
        // toml — composition accepts both, canonicalising via the
        // AstType serde impl.
        let tmp = TempDir::new().unwrap();
        let (vault_tx, _) = make_stage_dirs(tmp.path());
        let vault_root = tmp.path().join("vault");

        write_exe(&vault_tx, "10-pan.py", "#!/bin/sh\ntrue\n");
        write_manifest(&vault_tx, "10-pan.py.toml", r#"ast_type = "pandoc-types""#);
        write_exe(&vault_tx, "20-mda.py", "#!/bin/sh\ntrue\n");
        write_manifest(&vault_tx, "20-mda.py.toml", r#"ast_type = "mdast""#);

        let pipe = compose_stage(&vault_root, None, Stage::Transform).unwrap();
        let pan = pipe
            .hooks
            .iter()
            .find(|h| h.filename == "10-pan.py")
            .unwrap();
        let mda = pipe
            .hooks
            .iter()
            .find(|h| h.filename == "20-mda.py")
            .unwrap();
        assert_eq!(pan.ast_type, AstType::PandocExt);
        assert_eq!(mda.ast_type, AstType::MdastExt);
    }

    #[test]
    fn unknown_ast_type_is_manifest_parse_error() {
        let tmp = TempDir::new().unwrap();
        let (vault_tx, _) = make_stage_dirs(tmp.path());
        let vault_root = tmp.path().join("vault");

        write_exe(&vault_tx, "10-foo.py", "#!/bin/sh\ntrue\n");
        write_manifest(&vault_tx, "10-foo.py.toml", r#"ast_type = "djot""#);

        let err = compose_stage(&vault_root, None, Stage::Transform).unwrap_err();
        match err {
            CompositionError::ManifestParse { path, error } => {
                assert!(path.ends_with("10-foo.py.toml"));
                // toml's serde error wraps our AstType rejection.
                assert!(
                    error.contains("djot") || error.contains("variant"),
                    "expected djot/variant error, got: {error}"
                );
            }
            other => panic!("expected ManifestParse, got {other:?}"),
        }
    }

    #[test]
    fn contract_preserves_overrides_default_list() {
        let tmp = TempDir::new().unwrap();
        let (vault_tx, _) = make_stage_dirs(tmp.path());
        let vault_root = tmp.path().join("vault");

        write_exe(&vault_tx, "10-foo.py", "#!/bin/sh\ntrue\n");
        write_manifest(
            &vault_tx,
            "10-foo.py.toml",
            r#"ast_type = "mdast-ext"

[contract]
preserves = ["Wikilink", "CodeBlock"]
"#,
        );

        let pipe = compose_stage(&vault_root, None, Stage::Transform).unwrap();
        assert_eq!(pipe.hooks.len(), 1);
        // Manifest's explicit list wins over the AstType default.
        assert_eq!(
            pipe.hooks[0].preserves,
            vec!["Wikilink".to_string(), "CodeBlock".to_string()]
        );
    }

    #[test]
    fn ordering_table_optional_ref_drops_constraint_silently_to_caller() {
        // REQ-3217: `optional = true` + missing ref → warning + drop (not
        // error). The pipeline still composes so downstream selector-miss
        // behaviour (task-selector-eval) can skip silently at runtime.
        let tmp = TempDir::new().unwrap();
        let (vault_tx, _) = make_stage_dirs(tmp.path());
        let vault_root = tmp.path().join("vault");

        write_exe(&vault_tx, "10-foo.py", "#!/bin/sh\ntrue\n");
        write_manifest(
            &vault_tx,
            "10-foo.py.toml",
            r#"optional = true

[ordering]
after = ["ghost"]
"#,
        );

        let pipe = compose_stage(&vault_root, None, Stage::Transform).unwrap();
        // Hook is still in the pipeline; the dangling ref was dropped.
        assert_eq!(pipe.hooks.len(), 1);
        assert_eq!(pipe.hooks[0].extension_id, "foo");
        assert!(pipe.hooks[0].optional);
        // Warning surfaces the dropped constraint for diagnostics.
        assert!(pipe.warnings.iter().any(|w| w.contains("ghost")));
    }

    #[test]
    fn ecosystem_field_is_preserved_verbatim_from_manifest() {
        // SPEC-033 CON-3312 — the top-level `ecosystem = "..."` value
        // feeds `zetl ecosystem check`'s per-ecosystem count; composition
        // keeps it as an opaque string so unknown ids still compose and
        // surface a diagnostic at dispatch time, not here.
        let tmp = TempDir::new().unwrap();
        let (vault_tx, _) = make_stage_dirs(tmp.path());
        let vault_root = tmp.path().join("vault");

        write_exe(&vault_tx, "10-pan.py", "#!/bin/sh\ntrue\n");
        write_manifest(&vault_tx, "10-pan.py.toml", r#"ecosystem = "pandoc""#);
        write_exe(&vault_tx, "20-native.py", "#!/bin/sh\ntrue\n");
        // No manifest → ecosystem is None.

        let pipe = compose_stage(&vault_root, None, Stage::Transform).unwrap();
        let pan = pipe
            .hooks
            .iter()
            .find(|h| h.filename == "10-pan.py")
            .unwrap();
        let native = pipe
            .hooks
            .iter()
            .find(|h| h.filename == "20-native.py")
            .unwrap();
        assert_eq!(pan.ecosystem.as_deref(), Some("pandoc"));
        assert_eq!(native.ecosystem, None);
    }
}
