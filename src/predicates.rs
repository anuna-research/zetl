//! SPEC-045 REQ-4507 / CON-4502 — the OPTIONAL strict-vocabulary file
//! `.zetl/predicates.toml`.
//!
//! The vocabulary is **emergent by default**: when the file is absent, the set
//! of predicates is exactly the set observed in the corpus and nothing here is
//! consulted. When present, the file moves a vault along the *emergent →
//! crystallising → controlled* trajectory and carries
//! **governance/presentation/interop metadata only** — never semantic meaning
//! (no definitions, no inverse/transitive/domain/range). Meaning lives in the
//! predicate name, the target node, and SPL rules (ADR-4502).
//!
//! The file is NEVER seeded; absence is the default.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// The three vocabulary states (REQ-4507).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VocabularyState {
    /// No file — invention is never blocked; only advisory drift lints.
    Emergent,
    /// File present, `enforce = false` — undeclared warns + nearest-match.
    Crystallising,
    /// File present, `enforce = true` — undeclared predicate is an error.
    Controlled,
}

/// A grouping cosmetic for rendering (REQ-4511). NOT a semantic class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Classification,
    Provenance,
    Structural,
    Lifecycle,
    Generative,
}

/// Per-predicate administrative metadata. Every field is optional and
/// administrative; none expresses meaning (CON-4502).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PredicateMeta {
    /// Display-label override; default is auto-derived (REQ-4515).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    /// Grouping cosmetic for rendering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<Category>,
    /// A CURIE for RDF export (REQ-4516).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maps_to: Option<String>,
    /// Marks scaffolding predicates for an "upgrade me" lint.
    #[serde(default, skip_serializing_if = "is_false")]
    pub construction: bool,
    /// Marks `relates_to::` for the over-use lint.
    #[serde(default, skip_serializing_if = "is_false")]
    pub catch_all: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// The parsed `.zetl/predicates.toml`. Present ⇒ a crystallising/controlled
/// vocabulary; absent (modelled as `None` by [`load_from_vault`]) ⇒ emergent.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PredicatesConfig {
    /// `true` ⇒ undeclared predicate is an ERROR (controlled); `false` ⇒
    /// warn + nearest-match (crystallising). Defaults true when the file
    /// exists.
    #[serde(default = "default_enforce")]
    pub enforce: bool,
    /// `true` ⇒ suppress `predicate-prefer-conforms-to` (ADR-4503).
    #[serde(default)]
    pub accept_is_a: bool,
    /// CURIE prefix → namespace IRI (REQ-4516).
    #[serde(default)]
    pub prefixes: BTreeMap<String, String>,
    /// The declared (controlled) set; the value is governance/presentation
    /// metadata, never a definition.
    #[serde(default)]
    pub predicates: BTreeMap<String, PredicateMeta>,
}

fn default_enforce() -> bool {
    true
}

impl Default for PredicatesConfig {
    fn default() -> Self {
        Self {
            enforce: true,
            accept_is_a: false,
            prefixes: BTreeMap::new(),
            predicates: BTreeMap::new(),
        }
    }
}

/// Failure surface for loading a [`PredicatesConfig`].
#[derive(Debug)]
pub enum PredicatesConfigError {
    Io(String),
    /// Malformed TOML / unknown field / invalid value.
    Parse(String),
    /// A `[predicates]` key violated the CON-4501 grammar.
    BadPredicateName(String),
}

impl std::fmt::Display for PredicatesConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "could not read .zetl/predicates.toml: {e}"),
            Self::Parse(e) => write!(f, "malformed .zetl/predicates.toml: {e}"),
            Self::BadPredicateName(p) => write!(
                f,
                "invalid predicate name '{p}' in .zetl/predicates.toml (must be snake `^[a-z][a-z0-9_]*$` or CURIE `prefix:localName`)"
            ),
        }
    }
}

impl std::error::Error for PredicatesConfigError {}

impl PredicatesConfig {
    /// Parse from a TOML string, validating every `[predicates]` key against
    /// the CON-4501 grammar (fail-closed — LangSec full recognition).
    pub fn from_toml_str(s: &str) -> Result<Self, PredicatesConfigError> {
        let cfg: Self =
            toml::from_str(s).map_err(|e| PredicatesConfigError::Parse(e.to_string()))?;
        for key in cfg.predicates.keys() {
            if !is_valid_predicate(key) {
                return Err(PredicatesConfigError::BadPredicateName(key.clone()));
            }
        }
        Ok(cfg)
    }

    /// Load `.zetl/predicates.toml` from the vault. **Absent ⇒ `None`** (the
    /// emergent default, not an error). Present-but-malformed ⇒ `Err`.
    pub fn load_from_vault(vault_root: &Path) -> Result<Option<Self>, PredicatesConfigError> {
        let path = vault_root.join(".zetl").join("predicates.toml");
        match std::fs::read_to_string(&path) {
            Ok(s) => Self::from_toml_str(&s).map(Some),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(PredicatesConfigError::Io(e.to_string())),
        }
    }

    /// The vocabulary state implied by this (present) config.
    pub fn state(&self) -> VocabularyState {
        if self.enforce {
            VocabularyState::Controlled
        } else {
            VocabularyState::Crystallising
        }
    }

    /// Whether `predicate` is in the declared set.
    pub fn is_declared(&self, predicate: &str) -> bool {
        self.predicates.contains_key(predicate)
    }

    /// The display label for a predicate: a strict `display` override, else the
    /// auto-derived label (CON-4506).
    pub fn display_label(&self, predicate: &str) -> String {
        self.predicates
            .get(predicate)
            .and_then(|m| m.display.clone())
            .unwrap_or_else(|| auto_label(predicate))
    }
}

/// The vocabulary state for a possibly-absent config.
pub fn state_of(config: Option<&PredicatesConfig>) -> VocabularyState {
    match config {
        None => VocabularyState::Emergent,
        Some(c) => c.state(),
    }
}

/// SPEC-045 CON-4506 auto-label: take the predicate's local part (after any
/// CURIE `prefix:`), replace `_`/`-` with spaces, and capitalise the first
/// letter. `derived_from` → "Derived from"; `prov:wasDerivedFrom` →
/// "wasDerivedFrom". Pure, deterministic, no I/O.
pub fn auto_label(predicate: &str) -> String {
    // A CURIE keeps its (camelCase) local part verbatim — per the CON-4506
    // worked example `prov:wasDerivedFrom` → "wasDerivedFrom". Only the
    // un-prefixed snake form gets its first letter capitalised.
    let is_curie = predicate.contains(':');
    let local = predicate.split_once(':').map(|(_, l)| l).unwrap_or(predicate);
    let spaced: String = local
        .chars()
        .map(|c| if c == '_' || c == '-' { ' ' } else { c })
        .collect();
    if is_curie {
        return spaced;
    }
    let mut chars = spaced.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

/// Whether a string satisfies the CON-4501 predicate grammar (snake OR CURIE).
pub fn is_valid_predicate(p: &str) -> bool {
    is_snake_predicate(p) || is_curie_predicate(p)
}

/// `^[a-z][a-z0-9_]*$` — the SPL-functor-safe form.
pub fn is_snake_predicate(p: &str) -> bool {
    let mut chars = p.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// `prefix ":" localName` where prefix = `[a-z][a-z0-9]*` and localName =
/// `[A-Za-z][A-Za-z0-9_-]*`.
pub fn is_curie_predicate(p: &str) -> bool {
    let Some((prefix, local)) = p.split_once(':') else {
        return false;
    };
    let mut pc = prefix.chars();
    match pc.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    if !pc.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) {
        return false;
    }
    let mut lc = local.chars();
    match lc.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    lc.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_is_emergent() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = PredicatesConfig::load_from_vault(dir.path()).unwrap();
        assert!(cfg.is_none());
        assert_eq!(state_of(cfg.as_ref()), VocabularyState::Emergent);
    }

    #[test]
    fn present_default_enforce_is_controlled() {
        let cfg = PredicatesConfig::from_toml_str(
            "[predicates]\nderived_from = { display = \"Derived from\", category = \"provenance\" }\n",
        )
        .unwrap();
        assert!(cfg.enforce);
        assert_eq!(cfg.state(), VocabularyState::Controlled);
        assert!(cfg.is_declared("derived_from"));
        assert_eq!(cfg.display_label("derived_from"), "Derived from");
    }

    #[test]
    fn enforce_false_is_crystallising() {
        let cfg = PredicatesConfig::from_toml_str("enforce = false\n").unwrap();
        assert_eq!(cfg.state(), VocabularyState::Crystallising);
    }

    #[test]
    fn interop_prefixes_and_maps_to() {
        let cfg = PredicatesConfig::from_toml_str(
            "[prefixes]\nprov = \"http://www.w3.org/ns/prov#\"\n\n\
             [predicates]\nderived_from = { maps_to = \"prov:wasDerivedFrom\" }\n",
        )
        .unwrap();
        assert_eq!(cfg.prefixes.get("prov").map(|s| s.as_str()), Some("http://www.w3.org/ns/prov#"));
        assert_eq!(
            cfg.predicates["derived_from"].maps_to.as_deref(),
            Some("prov:wasDerivedFrom")
        );
    }

    #[test]
    fn semantic_fields_are_rejected() {
        // No `definition`, `inverse_of`, `transitive`, `domain`, `range`.
        let err = PredicatesConfig::from_toml_str(
            "[predicates]\nderived_from = { inverse_of = \"has_derivative\" }\n",
        );
        assert!(err.is_err());
    }

    #[test]
    fn unknown_top_level_field_rejected() {
        assert!(PredicatesConfig::from_toml_str("nonsense = 1\n").is_err());
    }

    #[test]
    fn bad_predicate_name_rejected() {
        assert!(PredicatesConfig::from_toml_str("[predicates]\n\"Bad Name\" = {}\n").is_err());
    }

    #[test]
    fn auto_label_matches_contract() {
        assert_eq!(auto_label("derived_from"), "Derived from");
        assert_eq!(auto_label("prov:wasDerivedFrom"), "wasDerivedFrom");
        assert_eq!(auto_label("see-also"), "See also");
        assert_eq!(auto_label(""), "");
    }

    #[test]
    fn grammar_recogniser() {
        assert!(is_snake_predicate("derived_from"));
        assert!(!is_snake_predicate("Derived_From"));
        assert!(!is_snake_predicate("prov:x"));
        assert!(is_curie_predicate("prov:wasDerivedFrom"));
        assert!(!is_curie_predicate("Prov:x"));
        assert!(!is_curie_predicate("prov:"));
        assert!(is_valid_predicate("derived_from"));
        assert!(is_valid_predicate("prov:wasDerivedFrom"));
        assert!(!is_valid_predicate("bad name"));
    }
}
