//! SPEC-048 REQ-4809 — Deduplicated, deterministic component CSS emission (unscoped).
//!
//! v1 collects each used component's `<name>.css`, emits it **once**, in a declared
//! total order (component name, then source layer), into `_static/`. The CSS is emitted
//! as authored (unscoped); selector *scoping* is deferred to SPEC-051. The component
//! root still carries a `data-z="<name>"` marker ([`scope_marker`]) so a later scoping
//! pass can confine rules without changing emitted HTML. NFR-4802: byte-identical
//! across repeated builds.

use std::collections::BTreeMap;

/// The source layer a component was resolved from (REQ-4804); part of the total order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    Vault,
    Theme,
    Default,
}

impl Layer {
    pub fn as_str(self) -> &'static str {
        match self {
            Layer::Vault => "vault",
            Layer::Theme => "theme",
            Layer::Default => "default",
        }
    }
}

/// The HTML attribute marking a component's rendered root for (future) CSS scoping.
pub fn scope_marker(name: &str) -> String {
    format!("data-z=\"{name}\"")
}

/// Accumulates the distinct component stylesheets used during a build. Deduplicates by
/// component name; ordering is deterministic (name, then layer).
#[derive(Debug, Default)]
pub struct CssCollector {
    // name -> (layer, css). First insertion for a name wins (whole-directory
    // resolution already picked one layer per name).
    sheets: BTreeMap<String, (Layer, String)>,
}

impl CssCollector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a component's stylesheet. Idempotent per component name — a component
    /// used N times contributes its CSS once.
    pub fn add(&mut self, name: &str, layer: Layer, css: &str) {
        self.sheets
            .entry(name.to_string())
            .or_insert_with(|| (layer, css.to_string()));
    }

    /// Whether any component CSS has been collected.
    pub fn is_empty(&self) -> bool {
        self.sheets.is_empty()
    }

    pub fn len(&self) -> usize {
        self.sheets.len()
    }

    /// Emit the deduped stylesheet in deterministic order. Each block is preceded by a
    /// `/* component: <name> (<layer>) */` banner so output is self-describing and
    /// stable. Returns `None` when nothing was collected (so no file is written —
    /// REQ-4813 backward-compatible default).
    pub fn emit(&self) -> Option<String> {
        if self.sheets.is_empty() {
            return None;
        }
        let mut out = String::new();
        // BTreeMap iterates by name; layer is captured for the banner and is stable.
        for (name, (layer, css)) in &self.sheets {
            out.push_str(&format!("/* component: {} ({}) */\n", name, layer.as_str()));
            out.push_str(css.trim_end());
            out.push_str("\n\n");
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedup_emits_once() {
        let mut c = CssCollector::new();
        c.add("callout", Layer::Theme, ".callout{color:red}");
        c.add("callout", Layer::Theme, ".callout{color:red}");
        c.add("callout", Layer::Theme, ".callout{color:red}");
        assert_eq!(c.len(), 1);
        let css = c.emit().unwrap();
        assert_eq!(css.matches(".callout{color:red}").count(), 1);
    }

    #[test]
    fn deterministic_order_by_name() {
        let mut a = CssCollector::new();
        a.add("zeta", Layer::Theme, ".z{}");
        a.add("alpha", Layer::Vault, ".a{}");
        let mut b = CssCollector::new();
        b.add("alpha", Layer::Vault, ".a{}");
        b.add("zeta", Layer::Theme, ".z{}");
        // byte-identical regardless of insertion order
        assert_eq!(a.emit(), b.emit());
        let css = a.emit().unwrap();
        assert!(css.find(".a{}").unwrap() < css.find(".z{}").unwrap());
    }

    #[test]
    fn empty_emits_nothing() {
        assert!(CssCollector::new().emit().is_none());
    }

    #[test]
    fn marker_format() {
        assert_eq!(scope_marker("nav-header"), "data-z=\"nav-header\"");
    }
}
