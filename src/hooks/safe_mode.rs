//! Safe-mode build + theme hook declaration (SPEC-032 REQ-3223).
//!
//! Two affordances live here:
//!
//! 1. **`--safe-mode` filter** — given a [`StagePipeline`] (the resolved
//!    output of [`crate::hooks::composition::compose_stage`]) and a
//!    [`SafeMode`] policy, [`apply`] returns the same pipeline shape with
//!    untrusted hooks removed and a list of [`SkippedHook`] records the
//!    caller emits to stderr per [`format_skip_line`].
//!
//!    "Untrusted" means: every vault hook, plus every theme hook whose
//!    `(stage, extension_id)` pair is *not* declared in the active
//!    theme's `[[theme.hooks]]` table.
//!
//! 2. **Declaration audit** — [`audit_theme_declarations`] walks the
//!    theme's `hooks/<stage>.d/` directories and the manifest's
//!    `[[theme.hooks]]` declarations, reporting hooks present on disk
//!    but missing from the manifest (the warning surface
//!    [`format_undeclared_warning`] formats) plus declared entries with
//!    no on-disk match (informational; surfaced by `zetl theme show`).
//!
//! This module owns the pure logic only. CLI flag wiring (`--safe-mode`
//! on `zetl build` / `zetl serve`) and the actual stderr writes happen
//! in their respective command handlers.

use std::collections::BTreeSet;
use std::path::Path;

use crate::hooks::composition::{
    compose_all_stages, ComposedHook, CompositionSource, StagePipeline,
};
use crate::hooks::pipeline::Stage;
use crate::web::theme::{ThemeHookDecl, ThemeManifest};

/// Policy describing which hooks are allow-listed under safe-mode.
///
/// Built from a [`ThemeManifest`]'s `[[theme.hooks]]` array via
/// [`Self::from_manifest`]. An empty manifest (or a theme with no
/// declarations) yields a policy that allows zero hooks — i.e.
/// `--safe-mode` with no declarations skips everything, matching the
/// SPEC-032 §10 threat model.
#[derive(Debug, Clone, Default)]
pub struct SafeMode {
    /// `(stage, extension_id)` pairs the active theme has declared.
    /// `Stage` is parsed from the declaration's string form; entries
    /// whose `stage` field doesn't match a known stage are silently
    /// dropped (the manifest parse step is structural; surfacing the
    /// typo lives in the audit path so the warning carries context).
    allowed: BTreeSet<(Stage, String)>,
}

impl SafeMode {
    /// Build a safe-mode policy from the theme manifest's declarations.
    /// Returns an empty policy when the manifest is `None`.
    pub fn from_manifest(manifest: Option<&ThemeManifest>) -> Self {
        let Some(manifest) = manifest else {
            return Self::default();
        };
        let mut allowed = BTreeSet::new();
        for decl in &manifest.theme.hooks {
            if let Some(stage) = parse_stage(&decl.stage) {
                allowed.insert((stage, decl.extension_id.clone()));
            }
        }
        Self { allowed }
    }

    /// Construct directly from already-parsed `(stage, id)` pairs.
    /// Used by tests and by callers that have their own declaration
    /// source (e.g. ecosystem adapters).
    pub fn from_pairs<I, S>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (Stage, S)>,
        S: Into<String>,
    {
        Self {
            allowed: pairs
                .into_iter()
                .map(|(stage, id)| (stage, id.into()))
                .collect(),
        }
    }

    /// Number of declared hooks the policy allows to run.
    pub fn allowed_count(&self) -> usize {
        self.allowed.len()
    }

    /// True if the policy permits this composed hook to execute under
    /// safe-mode. Vault hooks are always denied; theme hooks must
    /// match a declared `(stage, extension_id)` pair.
    pub fn allows(&self, hook: &ComposedHook) -> bool {
        if hook.source != CompositionSource::Theme {
            return false;
        }
        self.allowed
            .contains(&(hook.stage, hook.extension_id.clone()))
    }
}

/// One skipped hook reported by [`apply`].
///
/// The fields are exactly what [`format_skip_line`] needs to render the
/// SPEC-mandated `[zetl] --safe-mode: skipped <stage>/<extension_id> from <source>`
/// log line, kept as structured data so JSON-mode callers can serialise
/// them too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedHook {
    pub stage: Stage,
    pub extension_id: String,
    pub source: CompositionSource,
}

/// Filter `pipeline` to the safe-mode allow-list.
///
/// Returns the trimmed pipeline alongside the list of removed hooks in
/// pipeline order. The caller is responsible for emitting one
/// [`format_skip_line`] line per [`SkippedHook`] on stderr (the SPEC
/// names stderr as the channel; routing it via a logger is the
/// caller's choice).
///
/// `shadowed` and `disabled` carry-throughs are preserved unchanged —
/// safe-mode is a runtime filter on the *enabled* set, not a discovery-
/// time pruning. This keeps `zetl hook coverage` honest about what
/// composed but did not run.
pub fn apply(pipeline: StagePipeline, policy: &SafeMode) -> (StagePipeline, Vec<SkippedHook>) {
    let StagePipeline {
        stage,
        hooks,
        shadowed,
        disabled,
        warnings,
    } = pipeline;

    let mut kept = Vec::with_capacity(hooks.len());
    let mut skipped = Vec::new();
    for hook in hooks {
        if policy.allows(&hook) {
            kept.push(hook);
        } else {
            skipped.push(SkippedHook {
                stage: hook.stage,
                extension_id: hook.extension_id.clone(),
                source: hook.source,
            });
        }
    }

    (
        StagePipeline {
            stage,
            hooks: kept,
            shadowed,
            disabled,
            warnings,
        },
        skipped,
    )
}

/// Convenience wrapper that filters every stage of `pipelines` in one
/// call. Returns the trimmed pipelines and the concatenated skip list
/// in pipeline-iteration order (pre-parse → transform → post-render).
pub fn apply_all(
    pipelines: Vec<StagePipeline>,
    policy: &SafeMode,
) -> (Vec<StagePipeline>, Vec<SkippedHook>) {
    let mut trimmed = Vec::with_capacity(pipelines.len());
    let mut all_skipped = Vec::new();
    for p in pipelines {
        let (kept, mut skipped) = apply(p, policy);
        trimmed.push(kept);
        all_skipped.append(&mut skipped);
    }
    (trimmed, all_skipped)
}

/// Format the SPEC-mandated stderr line for one skipped hook.
///
/// Shape (REQ-3223):
/// `[zetl] --safe-mode: skipped <stage>/<extension_id> from <source>`
pub fn format_skip_line(skipped: &SkippedHook) -> String {
    format!(
        "[zetl] --safe-mode: skipped {}/{} from {}",
        skipped.stage, skipped.extension_id, skipped.source
    )
}

/// Result of comparing what a theme declares against what it ships.
///
/// `undeclared` are on-disk hooks the manifest forgot to list — the
/// SPEC's first-use warning surface. `missing_on_disk` are declarations
/// that point at hooks no longer present (typo, deleted file, never
/// shipped); informational, surfaced by `zetl theme show` so theme
/// authors can spot their own drift.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ThemeAudit {
    /// Theme name, threaded through so callers can format the warning
    /// without a second lookup.
    pub theme_name: String,
    /// Hooks composed from the theme's `hooks/<stage>.d/` directory
    /// that have no matching `[[theme.hooks]]` entry. Sorted by
    /// `(stage, extension_id)` for deterministic output.
    pub undeclared: Vec<UndeclaredHook>,
    /// Manifest entries that point at no on-disk hook. Sorted by
    /// `(stage, extension_id)`.
    pub missing_on_disk: Vec<ThemeHookDecl>,
}

/// One hook present on disk but missing from `[[theme.hooks]]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndeclaredHook {
    pub stage: Stage,
    pub extension_id: String,
}

impl ThemeAudit {
    /// True when the theme has at least one hook on disk that the
    /// manifest doesn't declare. Drives the SPEC-mandated first-use
    /// warning emission.
    pub fn has_undeclared(&self) -> bool {
        !self.undeclared.is_empty()
    }
}

/// Format the SPEC-mandated theme warning when a theme ships
/// undeclared hooks.
///
/// Shape (REQ-3223):
/// `[zetl] theme <name> ships <N> undeclared hook(s); run`
/// `'zetl theme show <name>' for details, or --safe-mode to suppress`
///
/// The two-line wrap matches the prose in the spec; callers that want
/// a single line can `.replace('\n', " ")` after formatting.
pub fn format_undeclared_warning(audit: &ThemeAudit) -> String {
    format!(
        "[zetl] theme {} ships {} undeclared hook(s); run \
         'zetl theme show {}' for details, or --safe-mode to suppress",
        audit.theme_name,
        audit.undeclared.len(),
        audit.theme_name,
    )
}

/// Compare a theme's manifest declarations against the hooks present
/// on disk under `theme_hooks_dir` (typically the path returned by
/// [`crate::hooks::resolve_theme_hooks`]).
///
/// `vault_root` is needed to feed
/// [`compose_all_stages`]; the vault side is composed too but ignored
/// for the audit (vault hooks are inherently un-allow-listable under
/// safe-mode).
pub fn audit_theme_declarations(
    theme_name: &str,
    vault_root: &Path,
    theme_hooks_dir: Option<&Path>,
    manifest: Option<&ThemeManifest>,
) -> ThemeAudit {
    let pipelines = match compose_all_stages(vault_root, theme_hooks_dir) {
        Ok(ps) => ps,
        Err(_) => {
            return ThemeAudit {
                theme_name: theme_name.to_string(),
                undeclared: Vec::new(),
                missing_on_disk: Vec::new(),
            };
        }
    };

    let mut declared: BTreeSet<(Stage, String)> = BTreeSet::new();
    let mut decls_by_key: std::collections::BTreeMap<(Stage, String), ThemeHookDecl> =
        std::collections::BTreeMap::new();
    if let Some(m) = manifest {
        for decl in &m.theme.hooks {
            if let Some(stage) = parse_stage(&decl.stage) {
                declared.insert((stage, decl.extension_id.clone()));
                decls_by_key.insert((stage, decl.extension_id.clone()), decl.clone());
            }
        }
    }

    let mut on_disk: BTreeSet<(Stage, String)> = BTreeSet::new();
    for pipe in &pipelines {
        for hook in pipe
            .hooks
            .iter()
            .chain(pipe.shadowed.iter())
            .chain(pipe.disabled.iter())
        {
            if hook.source == CompositionSource::Theme {
                on_disk.insert((hook.stage, hook.extension_id.clone()));
            }
        }
    }

    let mut undeclared: Vec<UndeclaredHook> = on_disk
        .difference(&declared)
        .map(|(stage, id)| UndeclaredHook {
            stage: *stage,
            extension_id: id.clone(),
        })
        .collect();
    undeclared.sort_by(|a, b| {
        a.stage
            .as_str()
            .cmp(b.stage.as_str())
            .then_with(|| a.extension_id.cmp(&b.extension_id))
    });

    let mut missing_on_disk: Vec<ThemeHookDecl> = declared
        .difference(&on_disk)
        .filter_map(|key| decls_by_key.get(key).cloned())
        .collect();
    missing_on_disk.sort_by(|a, b| {
        a.stage
            .cmp(&b.stage)
            .then_with(|| a.extension_id.cmp(&b.extension_id))
    });

    ThemeAudit {
        theme_name: theme_name.to_string(),
        undeclared,
        missing_on_disk,
    }
}

fn parse_stage(s: &str) -> Option<Stage> {
    match s {
        "pre-parse" => Some(Stage::PreParse),
        "transform" => Some(Stage::Transform),
        "post-render" => Some(Stage::PostRender),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::composition::{compose_stage, ComposedHook};
    use crate::hooks::translators::AstType;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Helper: drop a runnable hook file (shebang + 0o755) into
    /// `<root>/<rel>/<stage>.d/<filename>`. Returns the file path.
    fn write_hook(root: &Path, rel: &str, stage: Stage, filename: &str) -> PathBuf {
        let dir = root.join(rel).join(format!("{}.d", stage.as_str()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(filename);
        std::fs::write(&p, "#!/bin/sh\necho ok\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&p).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&p, perms).unwrap();
        }
        p
    }

    fn make_hook(stage: Stage, ext_id: &str, source: CompositionSource) -> ComposedHook {
        ComposedHook {
            stage,
            filename: format!("{ext_id}.py"),
            extension_id: ext_id.to_string(),
            path: PathBuf::from("/tmp/x"),
            manifest_path: None,
            source,
            before: Vec::new(),
            after: Vec::new(),
            optional: false,
            ast_type: AstType::default(),
            ast_version: None,
            preserves: Vec::new(),
            ecosystem: None,
            disabled: None,
        }
    }

    #[test]
    fn empty_manifest_allows_no_hooks() {
        let policy = SafeMode::default();
        let theme_hook = make_hook(Stage::Transform, "callouts", CompositionSource::Theme);
        let vault_hook = make_hook(Stage::Transform, "tasks", CompositionSource::Vault);
        assert!(!policy.allows(&theme_hook));
        assert!(!policy.allows(&vault_hook));
    }

    #[test]
    fn declared_theme_hook_passes_vault_always_skipped() {
        let policy = SafeMode::from_pairs([(Stage::Transform, "callouts")]);
        let allowed = make_hook(Stage::Transform, "callouts", CompositionSource::Theme);
        let undeclared = make_hook(Stage::Transform, "rogue", CompositionSource::Theme);
        let vault_callouts = make_hook(Stage::Transform, "callouts", CompositionSource::Vault);
        assert!(policy.allows(&allowed));
        assert!(!policy.allows(&undeclared));
        assert!(!policy.allows(&vault_callouts));
    }

    #[test]
    fn declaration_is_stage_specific() {
        let policy = SafeMode::from_pairs([(Stage::Transform, "callouts")]);
        let same_id_wrong_stage = make_hook(Stage::PreParse, "callouts", CompositionSource::Theme);
        assert!(!policy.allows(&same_id_wrong_stage));
    }

    #[test]
    fn apply_filters_vault_and_undeclared_theme_hooks() {
        let pipe = StagePipeline {
            stage: Stage::Transform,
            hooks: vec![
                make_hook(Stage::Transform, "callouts", CompositionSource::Theme),
                make_hook(Stage::Transform, "rogue", CompositionSource::Theme),
                make_hook(Stage::Transform, "tasks", CompositionSource::Vault),
            ],
            shadowed: Vec::new(),
            disabled: Vec::new(),
            warnings: Vec::new(),
        };
        let policy = SafeMode::from_pairs([(Stage::Transform, "callouts")]);
        let (kept, skipped) = apply(pipe, &policy);
        assert_eq!(kept.hooks.len(), 1);
        assert_eq!(kept.hooks[0].extension_id, "callouts");
        assert_eq!(skipped.len(), 2);
        // skipped order matches input pipeline order.
        assert_eq!(skipped[0].extension_id, "rogue");
        assert_eq!(skipped[0].source, CompositionSource::Theme);
        assert_eq!(skipped[1].extension_id, "tasks");
        assert_eq!(skipped[1].source, CompositionSource::Vault);
    }

    #[test]
    fn skip_line_format_matches_spec() {
        let s = SkippedHook {
            stage: Stage::Transform,
            extension_id: "callouts".to_string(),
            source: CompositionSource::Theme,
        };
        assert_eq!(
            format_skip_line(&s),
            "[zetl] --safe-mode: skipped transform/callouts from theme"
        );
    }

    #[test]
    fn warning_line_format_matches_spec() {
        let audit = ThemeAudit {
            theme_name: "fountain".to_string(),
            undeclared: vec![
                UndeclaredHook {
                    stage: Stage::Transform,
                    extension_id: "callouts".to_string(),
                },
                UndeclaredHook {
                    stage: Stage::PreParse,
                    extension_id: "tasks".to_string(),
                },
            ],
            missing_on_disk: Vec::new(),
        };
        let line = format_undeclared_warning(&audit);
        assert!(line.starts_with("[zetl] theme fountain ships 2 undeclared hook(s); run"));
        assert!(line.contains("'zetl theme show fountain'"));
        assert!(line.contains("--safe-mode to suppress"));
    }

    #[test]
    fn audit_finds_undeclared_and_missing() {
        let tmp = TempDir::new().unwrap();
        let vault_root = tmp.path().join("vault");
        let theme_root = tmp.path().join("theme");
        std::fs::create_dir_all(&vault_root).unwrap();
        std::fs::create_dir_all(&theme_root).unwrap();

        write_hook(&theme_root, "", Stage::Transform, "callouts.py");
        write_hook(&theme_root, "", Stage::Transform, "rogue.py");

        let manifest = ThemeManifest {
            theme: crate::web::theme::ThemeInfo {
                name: "fountain".to_string(),
                version: "1.0.0".to_string(),
                description: None,
                author: None,
                license: None,
                homepage: None,
                min_zetl_version: None,
                templates: None,
                hooks: vec![
                    ThemeHookDecl {
                        stage: "transform".to_string(),
                        extension_id: "callouts".to_string(),
                        ecosystem: None,
                        summary: None,
                        contract: None,
                    },
                    ThemeHookDecl {
                        stage: "transform".to_string(),
                        extension_id: "ghost".to_string(),
                        ecosystem: None,
                        summary: None,
                        contract: None,
                    },
                ],
            },
            graph_inline: None,
            graph: None,
        };

        let audit =
            audit_theme_declarations("fountain", &vault_root, Some(&theme_root), Some(&manifest));
        assert_eq!(audit.theme_name, "fountain");
        assert_eq!(audit.undeclared.len(), 1);
        assert_eq!(audit.undeclared[0].extension_id, "rogue");
        assert_eq!(audit.missing_on_disk.len(), 1);
        assert_eq!(audit.missing_on_disk[0].extension_id, "ghost");
        assert!(audit.has_undeclared());
    }

    #[test]
    fn audit_with_no_undeclared_does_not_flag() {
        let tmp = TempDir::new().unwrap();
        let vault_root = tmp.path().join("vault");
        let theme_root = tmp.path().join("theme");
        std::fs::create_dir_all(&vault_root).unwrap();
        std::fs::create_dir_all(&theme_root).unwrap();

        write_hook(&theme_root, "", Stage::Transform, "callouts.py");

        let manifest = ThemeManifest {
            theme: crate::web::theme::ThemeInfo {
                name: "fountain".to_string(),
                version: "1.0.0".to_string(),
                description: None,
                author: None,
                license: None,
                homepage: None,
                min_zetl_version: None,
                templates: None,
                hooks: vec![ThemeHookDecl {
                    stage: "transform".to_string(),
                    extension_id: "callouts".to_string(),
                    ecosystem: None,
                    summary: None,
                    contract: None,
                }],
            },
            graph_inline: None,
            graph: None,
        };

        let audit =
            audit_theme_declarations("fountain", &vault_root, Some(&theme_root), Some(&manifest));
        assert!(!audit.has_undeclared());
        assert!(audit.missing_on_disk.is_empty());
    }

    #[test]
    fn audit_handles_missing_manifest_as_all_undeclared() {
        let tmp = TempDir::new().unwrap();
        let vault_root = tmp.path().join("vault");
        let theme_root = tmp.path().join("theme");
        std::fs::create_dir_all(&vault_root).unwrap();
        std::fs::create_dir_all(&theme_root).unwrap();
        write_hook(&theme_root, "", Stage::Transform, "callouts.py");

        let audit = audit_theme_declarations("fountain", &vault_root, Some(&theme_root), None);
        assert_eq!(audit.undeclared.len(), 1);
        assert_eq!(audit.undeclared[0].extension_id, "callouts");
    }

    #[test]
    fn from_manifest_drops_unknown_stage_strings() {
        let manifest = ThemeManifest {
            theme: crate::web::theme::ThemeInfo {
                name: "fountain".to_string(),
                version: "1.0.0".to_string(),
                description: None,
                author: None,
                license: None,
                homepage: None,
                min_zetl_version: None,
                templates: None,
                hooks: vec![
                    ThemeHookDecl {
                        stage: "transform".to_string(),
                        extension_id: "callouts".to_string(),
                        ecosystem: None,
                        summary: None,
                        contract: None,
                    },
                    ThemeHookDecl {
                        // Typo — silently dropped from policy; the
                        // audit path is what surfaces the typo.
                        stage: "tranform".to_string(),
                        extension_id: "broken".to_string(),
                        ecosystem: None,
                        summary: None,
                        contract: None,
                    },
                ],
            },
            graph_inline: None,
            graph: None,
        };
        let policy = SafeMode::from_manifest(Some(&manifest));
        assert_eq!(policy.allowed_count(), 1);
    }

    #[test]
    fn integration_compose_then_safe_mode_filters() {
        let tmp = TempDir::new().unwrap();
        let vault_root = tmp.path().join("vault");
        let theme_root = tmp.path().join("theme");
        std::fs::create_dir_all(&vault_root).unwrap();
        std::fs::create_dir_all(&theme_root).unwrap();

        // Theme ships two transforms; only one declared.
        write_hook(&theme_root, "", Stage::Transform, "callouts.py");
        write_hook(&theme_root, "", Stage::Transform, "rogue.py");
        // Vault adds its own transform.
        write_hook(&vault_root, ".zetl/hooks", Stage::Transform, "tasks.py");

        let pipe = compose_stage(&vault_root, Some(&theme_root), Stage::Transform).unwrap();
        assert_eq!(pipe.hooks.len(), 3);

        let policy = SafeMode::from_pairs([(Stage::Transform, "callouts")]);
        let (filtered, skipped) = apply(pipe, &policy);
        assert_eq!(filtered.hooks.len(), 1);
        assert_eq!(filtered.hooks[0].extension_id, "callouts");
        assert_eq!(skipped.len(), 2);

        let lines: Vec<String> = skipped.iter().map(format_skip_line).collect();
        assert!(lines
            .iter()
            .any(|l| l == "[zetl] --safe-mode: skipped transform/rogue from theme"));
        assert!(lines
            .iter()
            .any(|l| l == "[zetl] --safe-mode: skipped transform/tasks from vault"));
    }
}
