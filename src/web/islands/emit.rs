//! SPEC-050 REQ-5001/5012/5018/5024/5027 — island asset emission + page integration.
//!
//! Build-scoped: discovers island components, parses their manifests + theme grants +
//! operator CSP, verifies wiring (REQ-5008/9), and emits the deterministic asset set
//! (shared bus runtime + per-component worker/module scripts + CSP headers + audit). Per
//! page it injects hydration markers onto each island component's `data-z` node, the
//! runtime bootstrap, the persisted pre-paint script, and the CSP `<meta>` — but **only**
//! when the page actually hosts an island (REQ-5012 byte-identical default).

use super::manifest::IslandManifest;
use super::theme::CspConfig;
use super::wiring::{self, WiringReport};
use super::{csp, manifest as island_manifest, theme as island_theme, IResult};
use crate::web::components::resolve::discover_components;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

/// The browser runtime, embedded at compile time and emitted once per island build.
const ISLANDS_RUNTIME: &str = include_str!("../assets/islands.js");

/// The discovered island set for a build.
pub struct IslandSet {
    /// Component name → island manifest (only components that declare island fields).
    pub islands: BTreeMap<String, IslandManifest>,
    /// Component name → `<name>.js` source (worker for content islands, module for
    /// trusted islands).
    pub scripts: BTreeMap<String, String>,
    pub csp: CspConfig,
    pub wiring: WiringReport,
}

impl IslandSet {
    /// Discover islands, parse theme grants + `[security.csp]`, verify wiring.
    pub fn discover(vault_root: &Path, theme: &str) -> IResult<Self> {
        let comps = discover_components(vault_root, theme)?;
        let mut islands = BTreeMap::new();
        let mut scripts = BTreeMap::new();
        for (name, c) in &comps {
            if let Some(im) = island_manifest::parse(&c.manifest)? {
                if let Some(js) = &c.js {
                    scripts.insert(name.clone(), js.clone());
                }
                islands.insert(name.clone(), im);
            }
        }
        // theme grants + operator CSP live in theme.toml / site config; read theme.toml
        // (bundled or on-disk) and the vault config for `[security.csp]`.
        let csp = load_csp(vault_root, theme)?;
        let manifests: Vec<IslandManifest> = islands.values().cloned().collect();
        let wiring = wiring::verify(&manifests, &csp);
        Ok(IslandSet { islands, scripts, csp, wiring })
    }

    /// True when the vault declares no island at all (REQ-5012 backward-compat path).
    pub fn is_empty(&self) -> bool {
        self.islands.is_empty()
    }

    /// Whether any island is a content-author island (gates CSP emission, REQ-5027).
    pub fn has_content_island(&self) -> bool {
        self.islands.values().any(|i| i.content_island)
    }

    /// Persisted topics across all islands → (topic, type-expr, default JSON literal).
    fn persisted_topics(&self) -> Vec<(String, String, String)> {
        let mut out = Vec::new();
        for im in self.islands.values() {
            for (name, decl) in &im.topics {
                if decl.persisted {
                    if let Some(def) = &decl.default {
                        out.push((name.clone(), decl.type_expr.clone(), toml_to_json(def)));
                    }
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// The render-blocking pre-paint script (REQ-5018) + its base64 sha256 (CSP hash), or
    /// `None` when no persisted topic exists.
    pub fn prepaint(&self) -> Option<(String, String)> {
        let topics = self.persisted_topics();
        if topics.is_empty() {
            return None;
        }
        let mut entries = String::new();
        for (topic, _ty, default_json) in &topics {
            // Each topic: read localStorage, RECOGNISE the value before applying it as a
            // DOM signal on <html> (data-zt-<topic>) — REQ-5006/CON-5004 require
            // recognition of the untrusted stored value before apply. The value is only
            // applied if it is a simple safe token (no control chars, no CSS-selector
            // breakout, bounded length); otherwise the declared default is used. All in
            // try/catch, default on any failure (BUG-8).
            entries.push_str(&format!(
                "try{{var v=localStorage.getItem('zetl:topic:{t}');var d={def};\
var val=(v==null||v==='')?d:JSON.parse(v);\
if(typeof val==='object'){{val=d;}}\
var sv=String(val);\
if(!/^[A-Za-z0-9_.:-]{{0,64}}$/.test(sv)){{sv=String(d);}}\
document.documentElement.setAttribute('data-zt-{t}',sv);\
}}catch(e){{try{{document.documentElement.setAttribute('data-zt-{t}',String({def}));}}catch(_){{}}}}",
                t = sanitise_attr_name(topic),
                def = default_json,
            ));
        }
        let script = format!("(function(){{{entries}}})();");
        let hash = sha256_b64(&script);
        Some((script, hash))
    }

    /// Emit the deterministic asset set to `out`. No-op when the set is empty.
    pub fn emit_assets(&self, out: &Path) -> std::io::Result<()> {
        if self.is_empty() {
            return Ok(());
        }
        let static_dir = out.join("_static");
        std::fs::create_dir_all(&static_dir)?;
        // shared bus runtime, once (REQ-5001/5004)
        std::fs::write(static_dir.join("zetl-islands.js"), ISLANDS_RUNTIME)?;
        // per-component scripts (deterministic order via BTreeMap)
        if !self.scripts.is_empty() {
            let islands_dir = static_dir.join("islands");
            std::fs::create_dir_all(&islands_dir)?;
            for (name, js) in &self.scripts {
                std::fs::write(islands_dir.join(format!("{name}.js")), js)?;
            }
        }
        // CSP headers artifact (served-deploy form) for content-island builds (REQ-5027)
        if self.has_content_island() {
            let hashes = self.prepaint().map(|(_, h)| vec![h]).unwrap_or_default();
            let policy = csp::content_island_policy(&self.csp, &hashes);
            std::fs::write(out.join("_headers.csp"), csp::headers_artifact(&policy))?;
        }
        // wiring audit (deterministic) — REQ-5009 / OBS-4801 extension
        std::fs::write(static_dir.join("island-audit.json"), self.audit_json())?;
        Ok(())
    }

    /// A deterministic JSON audit of the wiring graph (REQ-5009).
    fn audit_json(&self) -> String {
        let mut s = String::from("{\"edges\":[");
        for (i, e) in self.wiring.edges.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&format!(
                "{{\"topic\":{},\"publisher\":{},\"subscriber\":{}}}",
                json_str(&e.topic),
                json_str(&e.publisher),
                json_str(&e.subscriber)
            ));
        }
        s.push_str("],\"findings\":[");
        for (i, f) in self.wiring.findings.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&format!(
                "{{\"code\":{},\"fatal\":{},\"message\":{}}}",
                json_str(&f.code),
                f.fatal,
                json_str(&f.message)
            ));
        }
        s.push_str("]}");
        s
    }

    /// Inject hydration markers + head assets into a rendered page. Returns the modified
    /// HTML. No-op (byte-identical) when the page hosts no island component.
    pub fn inject_into_page(&self, html: &str, root_path: &str) -> String {
        // Which island components actually appear on this page — matched ONLY as a real
        // `data-z="name"` attribute *inside a start tag* (BUG-5). A literal occurrence in
        // prose/code text is never inside an open tag, and the body sanitiser strips
        // `data-z` from author content, so this never matches author-influenced bytes.
        let present: Vec<&String> = self
            .islands
            .keys()
            .filter(|name| island_node_re(name).is_match(html))
            .collect();
        let persisted = self.prepaint();
        if present.is_empty() && persisted.is_none() {
            return html.to_string(); // REQ-5012 byte-identical
        }

        let mut out = html.to_string();

        // 1. stamp markers onto each island component's real data-z start-tag node(s)
        let mut content_island_on_page = false;
        for name in &present {
            let im = &self.islands[*name];
            if im.content_island {
                content_island_on_page = true;
            }
            let markers = self.node_markers(im, root_path);
            let re = island_node_re(name);
            out = re
                .replace_all(&out, |caps: &regex::Captures| {
                    format!("{} {}", &caps[1], markers)
                })
                .into_owned();
        }

        // 2. assemble head assets
        let mut head_top = String::new(); // inserted right after <head>
        let mut head_end = String::new(); // inserted before </head>

        if content_island_on_page {
            let hashes = persisted.as_ref().map(|(_, h)| vec![h.clone()]).unwrap_or_default();
            let policy = csp::content_island_policy(&self.csp, &hashes);
            head_top.push_str(&csp::meta_tag(&policy));
        }
        if let Some((script, _hash)) = &persisted {
            head_top.push_str(&format!("<script>{script}</script>"));
        }
        if !present.is_empty() {
            head_end.push_str(&format!(
                "<script defer src=\"{root_path}_static/zetl-islands.js\"></script>"
            ));
            // trusted in-realm islands load their module directly (REQ-5010); content
            // islands are mounted by the runtime via the worker bridge.
            for name in &present {
                if !self.islands[*name].content_island && self.scripts.contains_key(*name) {
                    head_end.push_str(&format!(
                        "<script type=\"module\" defer src=\"{root_path}_static/islands/{name}.js\"></script>"
                    ));
                }
            }
        }

        if !head_top.is_empty() {
            out = insert_after_head_open(&out, &head_top);
        }
        if !head_end.is_empty() {
            out = insert_before_head_close(&out, &head_end);
        }
        out
    }

    /// The `data-island-*` marker attribute string for one island node (DOM contract
    /// matched by islands.js).
    fn node_markers(&self, im: &IslandManifest, root_path: &str) -> String {
        let mut m = format!("data-island=\"{}\"", im.component);
        m.push_str(&format!(" data-island-hydrate=\"{}\"", im.hydrate.as_attr()));
        if im.content_island {
            // content islands run in a Worker via the capability bridge
            m.push_str(&format!(
                " data-island-worker=\"{root_path}_static/islands/{}.js\"",
                im.component
            ));
            if im.paints {
                m.push_str(" data-island-paints=\"true\"");
            }
            // HTML-attribute-escape the JSON before embedding in a single-quoted attr so
            // a `'`/`<`/`&` in any topic or type-expr literal cannot break out (BUG-6).
            m.push_str(&format!(
                " data-island-grants='{}'",
                attr_escape(&self.grants_json(im))
            ));
            m.push_str(&format!(
                " data-island-types='{}'",
                attr_escape(&self.types_json(im))
            ));
        }
        m
    }

    fn grants_json(&self, im: &IslandManifest) -> String {
        let mut parts = Vec::new();
        for t in &im.publishes {
            parts.push(format!("{{\"topic\":{},\"direction\":\"publish\"}}", json_str(t)));
            parts.push(format!("{{\"topic\":{},\"direction\":\"emit\"}}", json_str(t)));
        }
        for t in &im.subscribes {
            parts.push(format!("{{\"topic\":{},\"direction\":\"subscribe\"}}", json_str(t)));
        }
        if im.paints {
            parts.push("{\"direction\":\"render\"}".to_string());
        }
        format!("[{}]", parts.join(","))
    }

    fn types_json(&self, im: &IslandManifest) -> String {
        let mut parts = Vec::new();
        for (name, decl) in &im.topics {
            parts.push(format!("{}:{}", json_str(name), json_str(&decl.type_expr)));
        }
        format!("{{{}}}", parts.join(","))
    }
}

/// Read the operator `[security.csp]` from the vault config + theme.toml.
fn load_csp(vault_root: &Path, theme: &str) -> IResult<CspConfig> {
    // vault site config (`.zetl/config.toml`) takes precedence; fall back to theme.toml.
    for rel in [".zetl/config.toml", ".zetl/zetl.toml"] {
        if let Ok(src) = std::fs::read_to_string(vault_root.join(rel)) {
            if let Ok(val) = src.parse::<toml::Value>() {
                let cfg = island_theme::parse_csp(&val)?;
                if !cfg.directives.is_empty() {
                    return Ok(cfg);
                }
            }
        }
    }
    let theme_toml = if theme != "default" {
        std::fs::read_to_string(vault_root.join(format!(".zetl/themes/{theme}/theme.toml")))
            .ok()
            .or_else(|| crate::web::engine::bundled_template(theme, "theme.toml").map(str::to_string))
    } else {
        crate::web::engine::bundled_template("default", "theme.toml").map(str::to_string)
    };
    if let Some(src) = theme_toml {
        if let Ok(val) = src.parse::<toml::Value>() {
            return island_theme::parse_csp(&val);
        }
    }
    Ok(CspConfig::default())
}

fn insert_after_head_open(html: &str, frag: &str) -> String {
    if let Some(idx) = html.find("<head") {
        if let Some(gt) = html[idx..].find('>') {
            let at = idx + gt + 1;
            let mut s = String::with_capacity(html.len() + frag.len());
            s.push_str(&html[..at]);
            s.push_str(frag);
            s.push_str(&html[at..]);
            return s;
        }
    }
    // no <head> — prepend
    format!("{frag}{html}")
}

fn insert_before_head_close(html: &str, frag: &str) -> String {
    if let Some(at) = html.find("</head>") {
        let mut s = String::with_capacity(html.len() + frag.len());
        s.push_str(&html[..at]);
        s.push_str(frag);
        s.push_str(&html[at..]);
        return s;
    }
    format!("{html}{frag}")
}

fn sanitise_attr_name(topic: &str) -> String {
    topic.replace(':', "-")
}

/// A regex matching a real `data-z="<name>"` attribute *inside a start tag* — i.e. with
/// no `<`/`>` between the tag open and the attribute. This distinguishes a component's
/// emitted root attribute from a literal `data-z="…"` occurrence in prose/code text
/// (which is never inside an open tag), so marker injection cannot be forged or corrupt
/// unrelated markup (BUG-5). Capture group 1 is the matched tag prefix up to and
/// including the `data-z` attribute.
fn island_node_re(name: &str) -> regex::Regex {
    let pat = format!(
        r#"(<[a-zA-Z][a-zA-Z0-9]*[^<>]*?\sdata-z="{}")"#,
        regex::escape(name)
    );
    // The pattern is always valid (name is regex-escaped); fall back to a never-match.
    regex::Regex::new(&pat).unwrap_or_else(|_| regex::Regex::new(r"$.^").unwrap())
}

/// HTML-attribute-escape (for single- or double-quoted attribute values).
fn attr_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('\'', "&#39;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn sha256_b64(s: &str) -> String {
    use base64::Engine;
    let digest = Sha256::digest(s.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(digest)
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Minimal TOML-literal → JSON-literal for pre-paint defaults (scalars + flat tables).
fn toml_to_json(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => json_str(s),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => f.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        toml::Value::Datetime(d) => json_str(&d.to_string()),
        toml::Value::Array(a) => {
            format!("[{}]", a.iter().map(toml_to_json).collect::<Vec<_>>().join(","))
        }
        toml::Value::Table(t) => {
            let parts: Vec<String> = t
                .iter()
                .map(|(k, v)| format!("{}:{}", json_str(k), toml_to_json(v)))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, c: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, c).unwrap();
    }

    fn poll_vault() -> tempfile::TempDir {
        let d = tempfile::TempDir::new().unwrap();
        let v = d.path();
        write(
            v,
            ".zetl/components/poll/poll.html",
            "<div data-z=\"{{ _name }}\">poll</div>",
        );
        write(
            v,
            ".zetl/components/poll/poll.toml",
            r#"name = "poll"
content_invocable = true
content_props = []
publishes = ["content:vote"]
render = "worker"
paints = true
hydrate = "visible"
[props]
[island.topics."content:vote"]
type = "enum(\"yes\",\"no\")"
"#,
        );
        write(v, ".zetl/components/poll/poll.js", "// worker code\n");
        d
    }

    #[test]
    fn discovers_island_and_emits_assets() {
        let d = poll_vault();
        let set = IslandSet::discover(d.path(), "default").unwrap();
        assert!(!set.is_empty());
        assert!(set.has_content_island());
        assert!(set.scripts.contains_key("poll"));

        let out = d.path().join("dist");
        std::fs::create_dir_all(&out).unwrap();
        set.emit_assets(&out).unwrap();
        assert!(out.join("_static/zetl-islands.js").is_file());
        assert!(out.join("_static/islands/poll.js").is_file());
        assert!(out.join("_static/island-audit.json").is_file());
        assert!(out.join("_headers.csp").is_file());
    }

    #[test]
    fn injects_markers_and_assets() {
        let d = poll_vault();
        let set = IslandSet::discover(d.path(), "default").unwrap();
        let page = "<html><head><title>x</title></head><body><div data-z=\"poll\">poll</div></body></html>";
        let out = set.inject_into_page(page, "./");
        assert!(out.contains("data-island=\"poll\""), "marker injected: {out}");
        assert!(out.contains("data-island-worker=\"./_static/islands/poll.js\""));
        assert!(out.contains("data-island-paints=\"true\""));
        assert!(out.contains("content:vote"), "grants/types present");
        assert!(out.contains("zetl-islands.js"), "runtime bootstrap injected");
        assert!(out.contains("Content-Security-Policy"), "CSP meta injected for content island");
    }

    #[test]
    fn no_island_on_page_is_byte_identical() {
        let d = poll_vault();
        let set = IslandSet::discover(d.path(), "default").unwrap();
        let page = "<html><head></head><body><p>no islands here</p></body></html>";
        let out = set.inject_into_page(page, "./");
        assert_eq!(out, page, "page without islands must be untouched (REQ-5012)");
    }
}
