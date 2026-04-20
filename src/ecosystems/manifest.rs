//! Per-ecosystem manifest fields (SPEC-033 REQ-3312 / CON-3312).
//!
//! The SPEC-032 base hook manifest (see [`crate::hooks::manifest::Manifest`])
//! declares stage, timeouts, selectors, ordering, and contract terms in a
//! way that is identical across every ecosystem. When a hook targets an
//! ecosystem adapter (Pandoc filters, mdBook preprocessors, remark
//! plugins) it needs a few additional fields — which binary to spawn,
//! which npm package to `import()`, whether a preprocessor operates on
//! one page or the whole vault. Those live here.
//!
//! ## Shape (CON-3312)
//!
//! One tagged-union enum, internally tagged on the manifest's own
//! top-level `ecosystem = "..."` field, with one variant per registered
//! ecosystem plus `zetl-native` for hooks that don't target an adapter:
//!
//! ```toml
//! # Pandoc filter
//! ecosystem = "pandoc"
//! exec = "pandoc-crossref"
//! args = ["--csl", "apa.csl"]
//!
//! # mdBook preprocessor
//! ecosystem = "mdbook"
//! exec = "mdbook-mermaid"
//! scope = "page"
//!
//! # remark plugin
//! ecosystem = "remark"
//! package = "remark-gfm"
//! version = ">=3.0 <4"
//! options = { singleTilde = false }
//! ```
//!
//! Each variant struct uses `#[serde(deny_unknown_fields)]` so a field
//! from the wrong ecosystem (e.g. `package = "..."` on an
//! `ecosystem = "pandoc"` manifest) is a parse error with a message
//! pointing at the offending key.
//!
//! ## Cross-field validation
//!
//! Pandoc manifests must declare exactly one of `exec` / `lua_filter`.
//! That constraint is not expressible purely in serde; [`validate`] runs
//! the check after deserialisation and returns a typed
//! [`EcosystemManifestError`] for each violation.
//!
//! ## Integration with the base manifest
//!
//! The base parser ([`crate::hooks::manifest::parse_manifest`]) extracts
//! the `ecosystem` key (and every non-base top-level key) before
//! deserialising the SPEC-032 fields. When an `ecosystem` key is
//! present, the extracted sub-table is parsed into [`EcosystemSpecific`]
//! and attached to [`crate::hooks::manifest::Manifest::extra`]. A
//! manifest without an `ecosystem` key keeps `extra = None` and behaves
//! exactly as a SPEC-032 zetl-native hook — this module's parser is
//! strictly additive.

use std::path::PathBuf;

use serde::Deserialize;
use toml::value::Table;

// ── Public enums ────────────────────────────────────────────────────────────

/// Per-ecosystem manifest block (SPEC-033 CON-3312).
///
/// Internally tagged on the `ecosystem = "..."` field. Unknown ids are
/// a parse error; the `zetl-native` variant accepts no extra fields and
/// is the default when a manifest declares the ecosystem explicitly but
/// doesn't target an adapter.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "ecosystem", rename_all = "kebab-case")]
pub enum EcosystemSpecific {
    /// Pandoc filter — `exec` or `lua_filter`, plus optional `args`.
    Pandoc(PandocManifestFields),
    /// mdBook preprocessor — `exec` and optional `scope`.
    Mdbook(MdbookManifestFields),
    /// remark plugin — `package` + optional `version` + optional `options`.
    Remark(RemarkManifestFields),
    /// Hook is zetl-native (no adapter). Accepted so authors can
    /// declare the ecosystem explicitly for documentation without
    /// switching syntaxes. Carries an empty struct body with
    /// `deny_unknown_fields` so an ecosystem-specific field alongside
    /// `ecosystem = "zetl-native"` still surfaces as a parse error.
    #[serde(rename = "zetl-native")]
    ZetlNative(ZetlNativeManifestFields),
}

impl EcosystemSpecific {
    /// Stable ecosystem id (matches the manifest tag string).
    pub fn id(&self) -> &'static str {
        match self {
            EcosystemSpecific::Pandoc(_) => "pandoc",
            EcosystemSpecific::Mdbook(_) => "mdbook",
            EcosystemSpecific::Remark(_) => "remark",
            EcosystemSpecific::ZetlNative(_) => "zetl-native",
        }
    }
}

/// Empty body for the `zetl-native` ecosystem variant.
///
/// Exists solely so `#[serde(deny_unknown_fields)]` can reject the
/// case of a user writing `ecosystem = "zetl-native"` alongside an
/// ecosystem-specific field like `exec` or `package`.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ZetlNativeManifestFields {}

/// Pandoc filter fields (SPEC-033 REQ-3312).
///
/// Exactly one of [`Self::exec`] and [`Self::lua_filter`] must be set;
/// that constraint is checked in [`validate`] because serde cannot
/// express XOR natively.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PandocManifestFields {
    /// Pandoc filter binary spawned via Pandoc's `--filter` convention.
    /// Mutually exclusive with [`Self::lua_filter`].
    #[serde(default)]
    pub exec: Option<String>,
    /// argv tail passed to the filter after Pandoc's own arguments.
    #[serde(default)]
    pub args: Vec<String>,
    /// Lua filter path passed via Pandoc's `--lua-filter` option.
    /// Mutually exclusive with [`Self::exec`].
    #[serde(default)]
    pub lua_filter: Option<PathBuf>,
}

/// mdBook preprocessor fields (SPEC-033 REQ-3312).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MdbookManifestFields {
    /// Preprocessor binary (conventionally `mdbook-<name>`).
    pub exec: String,
    /// Whether the preprocessor operates on one page at a time
    /// ([`MdbookScope::Page`], the default) or receives the full
    /// envelope for the whole vault ([`MdbookScope::Vault`]).
    #[serde(default)]
    pub scope: MdbookScope,
}

/// Invocation scope for an mdBook preprocessor (SPEC-033 REQ-3312).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum MdbookScope {
    /// Invoke the preprocessor once per page. Default.
    #[default]
    Page,
    /// Invoke the preprocessor once with the full vault envelope.
    Vault,
}

impl MdbookScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            MdbookScope::Page => "page",
            MdbookScope::Vault => "vault",
        }
    }
}

impl std::fmt::Display for MdbookScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// remark plugin fields (SPEC-033 REQ-3312).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemarkManifestFields {
    /// npm package name resolved by the remark harness via dynamic
    /// `import()` (conventionally `remark-<name>`).
    pub package: String,
    /// Optional semver range against the installed package's
    /// `package.json` version. `None` means "any version accepted" at
    /// the manifest layer; plugin-version-drift detection (SPEC-033
    /// REQ-3314) applies its own checks later.
    #[serde(default)]
    pub version: Option<String>,
    /// Free-form options table passed verbatim to the plugin's
    /// initialiser. Kept as a toml Table so downstream serialisation
    /// into the harness's JSON payload is lossless.
    #[serde(default)]
    pub options: Table,
}

// ── Errors ──────────────────────────────────────────────────────────────────

/// Typed errors produced by [`parse`] and [`validate`].
#[derive(Debug, Clone, PartialEq)]
pub enum EcosystemManifestError {
    /// TOML deserialisation failed — unknown field, wrong type,
    /// unknown ecosystem id, missing required field, etc. The toml
    /// crate's message carries line/column info where available.
    Parse(String),
    /// A Pandoc manifest declared both `exec` and `lua_filter`. The
    /// two are mutually exclusive per REQ-3312.
    PandocExecAndLuaFilter,
    /// A Pandoc manifest declared neither `exec` nor `lua_filter`.
    /// At least one is required.
    PandocMissingExec,
}

impl std::fmt::Display for EcosystemManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EcosystemManifestError::Parse(msg) => f.write_str(msg),
            EcosystemManifestError::PandocExecAndLuaFilter => f.write_str(
                "pandoc manifest declares both `exec` and `lua_filter`; \
                 they are mutually exclusive — pick one",
            ),
            EcosystemManifestError::PandocMissingExec => f.write_str(
                "pandoc manifest must declare `exec = \"<filter-binary>\"` \
                 or `lua_filter = \"<path>\"`",
            ),
        }
    }
}

impl std::error::Error for EcosystemManifestError {}

// ── Public API ──────────────────────────────────────────────────────────────

/// Deserialise an ecosystem manifest block from a TOML sub-table.
///
/// The sub-table must carry the `ecosystem` tag plus every field the
/// chosen variant expects. Unknown fields, unknown ecosystem ids, and
/// wrong types surface as [`EcosystemManifestError::Parse`].
///
/// Cross-field validation (Pandoc's `exec` XOR `lua_filter`) is applied
/// after deserialisation.
pub fn parse(table: Table) -> Result<EcosystemSpecific, EcosystemManifestError> {
    let value: EcosystemSpecific = toml::Value::Table(table)
        .try_into()
        .map_err(|e: toml::de::Error| EcosystemManifestError::Parse(e.to_string()))?;
    validate(&value)?;
    Ok(value)
}

/// Run the cross-field validations serde cannot express on its own.
///
/// Currently: Pandoc's `exec` XOR `lua_filter` constraint. Other
/// ecosystems' fields are fully expressible via `deny_unknown_fields`
/// and serde's required-field enforcement, so they pass trivially.
pub fn validate(spec: &EcosystemSpecific) -> Result<(), EcosystemManifestError> {
    if let EcosystemSpecific::Pandoc(p) = spec {
        match (&p.exec, &p.lua_filter) {
            (Some(_), Some(_)) => return Err(EcosystemManifestError::PandocExecAndLuaFilter),
            (None, None) => return Err(EcosystemManifestError::PandocMissingExec),
            _ => {}
        }
    }
    Ok(())
}

/// Base manifest keys that are NOT part of any ecosystem sub-table.
///
/// Used by [`crate::hooks::manifest::parse_manifest`] to split a raw
/// manifest table into a base half (SPEC-032 fields) and an ecosystem
/// half (this module's fields). A key not in this set is treated as
/// ecosystem-specific when `ecosystem = "..."` is present on the
/// manifest and extracted into the sub-table handed to [`parse`].
pub const BASE_MANIFEST_KEYS: &[&str] = &[
    "stage",
    "mode",
    "timeout_ms",
    "memory_mib",
    "ast_type",
    "ast_version",
    "extension_id",
    "optional",
    "before",
    "after",
    "ordering",
    "select",
    "contract",
];

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str) -> Table {
        toml::from_str(text).expect("test inputs must be valid TOML")
    }

    // ── Pandoc ────────────────────────────────────────────────────────────

    #[test]
    fn pandoc_with_exec_parses() {
        let t = table(
            r#"
ecosystem = "pandoc"
exec = "pandoc-crossref"
args = ["--csl", "apa.csl"]
"#,
        );
        let spec = parse(t).unwrap();
        match spec {
            EcosystemSpecific::Pandoc(p) => {
                assert_eq!(p.exec.as_deref(), Some("pandoc-crossref"));
                assert_eq!(p.args, vec!["--csl".to_string(), "apa.csl".to_string()]);
                assert!(p.lua_filter.is_none());
            }
            other => panic!("expected Pandoc, got {other:?}"),
        }
    }

    #[test]
    fn pandoc_with_lua_filter_parses() {
        let t = table(
            r#"
ecosystem = "pandoc"
lua_filter = "filters/counter.lua"
"#,
        );
        let spec = parse(t).unwrap();
        match spec {
            EcosystemSpecific::Pandoc(p) => {
                assert_eq!(
                    p.lua_filter.as_deref(),
                    Some(std::path::Path::new("filters/counter.lua"))
                );
                assert!(p.exec.is_none());
            }
            other => panic!("expected Pandoc, got {other:?}"),
        }
    }

    #[test]
    fn pandoc_exec_and_lua_filter_both_rejected() {
        let t = table(
            r#"
ecosystem = "pandoc"
exec = "pandoc-crossref"
lua_filter = "filters/counter.lua"
"#,
        );
        let err = parse(t).unwrap_err();
        assert_eq!(err, EcosystemManifestError::PandocExecAndLuaFilter);
    }

    #[test]
    fn pandoc_missing_exec_and_lua_filter_rejected() {
        let t = table(
            r#"
ecosystem = "pandoc"
args = ["--csl", "apa.csl"]
"#,
        );
        let err = parse(t).unwrap_err();
        assert_eq!(err, EcosystemManifestError::PandocMissingExec);
    }

    #[test]
    fn pandoc_with_package_rejected() {
        // `package` is a remark-only field — on a pandoc manifest it
        // must surface as a parse error (TEST-3312 main assertion).
        let t = table(
            r#"
ecosystem = "pandoc"
exec = "pandoc-crossref"
package = "remark-gfm"
"#,
        );
        let err = parse(t).unwrap_err();
        match err {
            EcosystemManifestError::Parse(msg) => {
                assert!(
                    msg.contains("unknown field") || msg.contains("package"),
                    "expected unknown-field error mentioning `package`, got: {msg}"
                );
            }
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn pandoc_with_scope_rejected() {
        let t = table(
            r#"
ecosystem = "pandoc"
exec = "pandoc-crossref"
scope = "page"
"#,
        );
        let err = parse(t).unwrap_err();
        match err {
            EcosystemManifestError::Parse(msg) => {
                assert!(msg.contains("unknown field") || msg.contains("scope"));
            }
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    // ── mdBook ────────────────────────────────────────────────────────────

    #[test]
    fn mdbook_with_exec_and_default_scope() {
        let t = table(
            r#"
ecosystem = "mdbook"
exec = "mdbook-mermaid"
"#,
        );
        let spec = parse(t).unwrap();
        match spec {
            EcosystemSpecific::Mdbook(m) => {
                assert_eq!(m.exec, "mdbook-mermaid");
                assert_eq!(m.scope, MdbookScope::Page);
            }
            other => panic!("expected Mdbook, got {other:?}"),
        }
    }

    #[test]
    fn mdbook_with_vault_scope() {
        let t = table(
            r#"
ecosystem = "mdbook"
exec = "mdbook-toc"
scope = "vault"
"#,
        );
        let spec = parse(t).unwrap();
        match spec {
            EcosystemSpecific::Mdbook(m) => assert_eq!(m.scope, MdbookScope::Vault),
            other => panic!("expected Mdbook, got {other:?}"),
        }
    }

    #[test]
    fn mdbook_without_exec_rejected() {
        let t = table(r#"ecosystem = "mdbook""#);
        let err = parse(t).unwrap_err();
        match err {
            EcosystemManifestError::Parse(msg) => {
                assert!(msg.contains("missing field") || msg.contains("exec"));
            }
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn mdbook_invalid_scope_rejected() {
        let t = table(
            r#"
ecosystem = "mdbook"
exec = "mdbook-mermaid"
scope = "sideways"
"#,
        );
        let err = parse(t).unwrap_err();
        match err {
            EcosystemManifestError::Parse(_) => {}
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn mdbook_with_package_rejected() {
        let t = table(
            r#"
ecosystem = "mdbook"
exec = "mdbook-mermaid"
package = "remark-gfm"
"#,
        );
        let err = parse(t).unwrap_err();
        match err {
            EcosystemManifestError::Parse(msg) => {
                assert!(msg.contains("unknown field") || msg.contains("package"));
            }
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    // ── remark ────────────────────────────────────────────────────────────

    #[test]
    fn remark_with_package_only() {
        let t = table(
            r#"
ecosystem = "remark"
package = "remark-gfm"
"#,
        );
        let spec = parse(t).unwrap();
        match spec {
            EcosystemSpecific::Remark(r) => {
                assert_eq!(r.package, "remark-gfm");
                assert!(r.version.is_none());
                assert!(r.options.is_empty());
            }
            other => panic!("expected Remark, got {other:?}"),
        }
    }

    #[test]
    fn remark_full_block() {
        let t = table(
            r#"
ecosystem = "remark"
package = "remark-gfm"
version = ">=3.0 <4"

[options]
singleTilde = false
firstLineBlank = true
"#,
        );
        let spec = parse(t).unwrap();
        match spec {
            EcosystemSpecific::Remark(r) => {
                assert_eq!(r.package, "remark-gfm");
                assert_eq!(r.version.as_deref(), Some(">=3.0 <4"));
                assert_eq!(
                    r.options.get("singleTilde").and_then(|v| v.as_bool()),
                    Some(false)
                );
                assert_eq!(
                    r.options.get("firstLineBlank").and_then(|v| v.as_bool()),
                    Some(true)
                );
            }
            other => panic!("expected Remark, got {other:?}"),
        }
    }

    #[test]
    fn remark_without_package_rejected() {
        let t = table(r#"ecosystem = "remark""#);
        let err = parse(t).unwrap_err();
        match err {
            EcosystemManifestError::Parse(msg) => {
                assert!(msg.contains("missing field") || msg.contains("package"));
            }
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn remark_with_exec_rejected() {
        let t = table(
            r#"
ecosystem = "remark"
package = "remark-gfm"
exec = "pandoc-crossref"
"#,
        );
        let err = parse(t).unwrap_err();
        match err {
            EcosystemManifestError::Parse(msg) => {
                assert!(msg.contains("unknown field") || msg.contains("exec"));
            }
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    // ── zetl-native and unknown ───────────────────────────────────────────

    #[test]
    fn zetl_native_with_no_extra_fields() {
        let t = table(r#"ecosystem = "zetl-native""#);
        let spec = parse(t).unwrap();
        assert_eq!(
            spec,
            EcosystemSpecific::ZetlNative(ZetlNativeManifestFields::default())
        );
        assert_eq!(spec.id(), "zetl-native");
    }

    #[test]
    fn zetl_native_with_extra_fields_rejected() {
        let t = table(
            r#"
ecosystem = "zetl-native"
exec = "pandoc-crossref"
"#,
        );
        let err = parse(t).unwrap_err();
        match err {
            EcosystemManifestError::Parse(_) => {}
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn unknown_ecosystem_rejected() {
        let t = table(r#"ecosystem = "djot""#);
        let err = parse(t).unwrap_err();
        match err {
            EcosystemManifestError::Parse(msg) => {
                // Serde's internally-tagged error phrasing varies across
                // crate versions; accept either "unknown variant" or the
                // id appearing in the message.
                assert!(
                    msg.contains("unknown variant")
                        || msg.contains("djot")
                        || msg.contains("ecosystem"),
                    "expected unknown-variant error mentioning `djot`, got: {msg}"
                );
            }
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn missing_ecosystem_tag_rejected() {
        let t = table(r#"exec = "pandoc-crossref""#);
        let err = parse(t).unwrap_err();
        match err {
            EcosystemManifestError::Parse(_) => {}
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    // ── id() round-trip ───────────────────────────────────────────────────

    #[test]
    fn id_matches_tag_for_every_variant() {
        assert_eq!(
            EcosystemSpecific::Pandoc(PandocManifestFields {
                exec: Some("x".into()),
                ..Default::default()
            })
            .id(),
            "pandoc"
        );
        assert_eq!(
            EcosystemSpecific::Mdbook(MdbookManifestFields {
                exec: "x".into(),
                scope: MdbookScope::Page,
            })
            .id(),
            "mdbook"
        );
        assert_eq!(
            EcosystemSpecific::Remark(RemarkManifestFields {
                package: "x".into(),
                version: None,
                options: Table::new(),
            })
            .id(),
            "remark"
        );
        assert_eq!(
            EcosystemSpecific::ZetlNative(ZetlNativeManifestFields::default()).id(),
            "zetl-native"
        );
    }
}
