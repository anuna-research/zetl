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
use super::theme::{Direction, IslandGrant};
use super::topic::{recognise_topic, TopicKind};
use super::wiring::{self, WiringReport};
use super::{csp, manifest as island_manifest, theme as island_theme, IResult, IslandError};
use crate::web::components::resolve::discover_components;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

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
    /// Theme-author grants ([[theme.island-grants]]) — a content island may read a
    /// trusted topic only when one of these grants it.
    pub grants: Vec<IslandGrant>,
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
                // An island manifest declares runtime behaviour (publishes/subscribes/
                // render/paints/hydrate) — it is meaningless without a client script. A
                // missing `<name>.js` would emit a worker URL pointing at a file that is
                // never written, so hydration always fails. Reject at build (Codex P2).
                match &c.js {
                    Some(js) => {
                        scripts.insert(name.clone(), js.clone());
                    }
                    None => {
                        return Err(IslandError::new(
                            "island-script-missing",
                            format!("component `{name}` declares island fields but ships no `{name}.js`"),
                        ));
                    }
                }
                islands.insert(name.clone(), im);
            }
        }
        // theme grants + operator CSP live in theme.toml / site config; read theme.toml
        // (bundled or on-disk) and the vault config for `[security.csp]`.
        let csp = load_csp(vault_root, theme)?;
        let grants = load_island_grants(vault_root, theme)?;

        // CON-5002: a content island may SUBSCRIBE to a trusted (non-`content:`) topic
        // ONLY when a matching [[theme.island-grants]] entry grants the read. Listing it
        // in the component manifest is not enough — that would let untrusted worker code
        // read trusted bus topics at will (Codex P1). Fail closed at build.
        for im in islands.values() {
            if !im.content_island {
                continue;
            }
            for t in &im.subscribes {
                if matches!(recognise_topic(t)?, TopicKind::Trusted) {
                    let granted = grants.iter().any(|g| {
                        g.component == im.component
                            && &g.topic == t
                            && g.direction == Direction::Subscribe
                    });
                    if !granted {
                        return Err(IslandError::new(
                            "island-capability-ungranted",
                            format!(
                                "content island `{}` subscribes trusted topic `{t}` without a matching [[theme.island-grants]] entry",
                                im.component
                            ),
                        ));
                    }
                }
            }
        }

        let manifests: Vec<IslandManifest> = islands.values().cloned().collect();
        let wiring = wiring::verify(&manifests, &csp);
        Ok(IslandSet {
            islands,
            scripts,
            csp,
            grants,
            wiring,
        })
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
            // The localStorage key uses the RAW topic (`zetl:topic:content:vote`) to match
            // what the runtime writes (CON-5004); only the DOM attribute name is sanitised
            // (`data-zt-content-vote`). Using the sanitised name for the key made pre-paint
            // read a key the runtime never wrote, so stored values were ignored (Codex P2).
            entries.push_str(&format!(
                "try{{var v=localStorage.getItem('zetl:topic:{key}');var d={def};\
var val=(v==null||v==='')?d:JSON.parse(v);\
if(typeof val==='object'){{val=d;}}\
var sv=String(val);\
if(!/^[A-Za-z0-9_.:-]{{0,64}}$/.test(sv)){{sv=String(d);}}\
document.documentElement.setAttribute('data-zt-{attr}',sv);\
}}catch(e){{try{{document.documentElement.setAttribute('data-zt-{attr}',String({def}));}}catch(_){{}}}}",
                key = js_single_quote_escape(topic),
                attr = sanitise_attr_name(topic),
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
        // CSP headers artifact (served-deploy form) for content-island builds (REQ-5027).
        // ALL enforcement (script/style restrictions with per-page inline-asset hashes,
        // and connect-src egress incl. operator widenings) lives in each page's
        // authoritative <meta> CSP — none of it can go in a single build-wide header
        // without either blocking the theme's own inline assets or overriding per-page
        // egress. The header therefore carries ONLY `frame-ancestors` (which <meta>
        // cannot express), so header and meta never conflict (Codex P2).
        if self.has_content_island() {
            std::fs::write(out.join("_headers.csp"), csp::headers_artifact(&self.csp))?;
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

        // 2. head-end: external island scripts (covered by script-src 'self', no hash).
        let mut head_end = String::new();
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
        if !head_end.is_empty() {
            out = insert_before_head_close(&out, &head_end);
        }

        // 3. pre-paint inline script (render-blocking) right after <head>.
        if let Some((script, _hash)) = &persisted {
            out = insert_after_head_open(&out, &format!("<script>{script}</script>"));
        }

        // 4. CSP meta as the FIRST head child (content-island pages). script-src must admit
        //    every inline <script> already on the page (the theme/pre-paint ship some) by
        //    sha256 — otherwise the default-deny baseline would block them (Codex P2).
        //    Inline styles are handled by style-src 'unsafe-inline' (theme uses style="" /
        //    CSSOM, which hashes can't cover), so only scripts need hashing here.
        if content_island_on_page {
            let script_hashes = inline_script_hashes(&out);
            let policy = csp::content_island_policy(&self.csp, &script_hashes);
            out = insert_after_head_open(&out, &csp::meta_tag(&policy));
        }
        out
    }

    /// The `data-island-*` marker attribute string for one island node (DOM contract
    /// matched by islands.js).
    fn node_markers(&self, im: &IslandManifest, root_path: &str) -> String {
        let mut m = format!("data-island=\"{}\"", im.component);
        m.push_str(&format!(
            " data-island-hydrate=\"{}\"",
            im.hydrate.as_attr()
        ));
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
        }
        // Declared topic types are needed by the runtime bus for EVERY island (trusted +
        // content) so payloads recognise correctly; emit whenever any topic is declared.
        if !im.topics.is_empty() {
            m.push_str(&format!(
                " data-island-types='{}'",
                attr_escape(&self.types_json(im))
            ));
        }
        // Persisted-topic defaults: without this marker the runtime never calls
        // seedPersisted/persistOnChange, so persisted topics are never written to
        // localStorage and cross-tab reflection is dead (Codex P2).
        let persist = self.persist_json(im);
        if persist != "{}" {
            m.push_str(&format!(" data-island-persist='{}'", attr_escape(&persist)));
        }
        m
    }

    /// `{ "<topic>": <defaultJson> }` for the island's persisted topics (REQ-5006).
    fn persist_json(&self, im: &IslandManifest) -> String {
        let mut parts = Vec::new();
        for (name, decl) in &im.topics {
            if decl.persisted {
                if let Some(def) = &decl.default {
                    parts.push(format!("{}:{}", json_str(name), toml_to_json(def)));
                }
            }
        }
        format!("{{{}}}", parts.join(","))
    }

    fn grants_json(&self, im: &IslandManifest) -> String {
        let mut parts = Vec::new();
        for t in &im.publishes {
            parts.push(format!(
                "{{\"topic\":{},\"direction\":\"publish\"}}",
                json_str(t)
            ));
            parts.push(format!(
                "{{\"topic\":{},\"direction\":\"emit\"}}",
                json_str(t)
            ));
        }
        for t in &im.subscribes {
            // A content island's subscribe grant on a trusted topic is emitted ONLY when
            // the theme granted it (discover already errors otherwise; this keeps an
            // ungranted trusted topic out of the runtime grant table regardless). Content
            // topics and trusted-island subscribes are always emitted.
            let allowed = !im.content_island
                || recognise_topic(t)
                    .map(|k| k == TopicKind::Content)
                    .unwrap_or(false)
                || self.grants.iter().any(|g| {
                    g.component == im.component
                        && &g.topic == t
                        && g.direction == Direction::Subscribe
                });
            if allowed {
                parts.push(format!(
                    "{{\"topic\":{},\"direction\":\"subscribe\"}}",
                    json_str(t)
                ));
            }
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
/// Read `[[theme.island-grants]]` from the active theme's `theme.toml` (disk or bundled).
fn load_island_grants(vault_root: &Path, theme: &str) -> IResult<Vec<IslandGrant>> {
    let theme_toml = if theme != "default" {
        std::fs::read_to_string(vault_root.join(format!(".zetl/themes/{theme}/theme.toml")))
            .ok()
            .or_else(|| {
                crate::web::engine::bundled_template(theme, "theme.toml").map(str::to_string)
            })
    } else {
        crate::web::engine::bundled_template("default", "theme.toml").map(str::to_string)
    };
    match theme_toml.and_then(|s| s.parse::<toml::Value>().ok()) {
        Some(val) => island_theme::parse_island_grants(&val),
        None => Ok(Vec::new()),
    }
}

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
            .or_else(|| {
                crate::web::engine::bundled_template(theme, "theme.toml").map(str::to_string)
            })
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

/// Escape a string for embedding inside a single-quoted JS string literal. Topic names
/// are CON-5001-restricted (no quotes/backslashes), but escape defensively.
fn js_single_quote_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
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

/// Collect base64 `sha256` digests of every INLINE `<script>` (no `src`) block on the
/// page, so the content-island CSP `script-src` can admit the theme's + pre-paint's inline
/// scripts by hash (REQ-5027 — never `unsafe-inline` for scripts). The browser hashes the
/// exact text between the tags, which is what we hash. Deduped + sorted for determinism
/// (NFR-5003). (Inline styles use `style-src 'unsafe-inline'`, so they need no hashes.)
fn inline_script_hashes(html: &str) -> Vec<String> {
    use regex::Regex;
    use std::collections::BTreeSet;
    static SCRIPT: OnceLock<Regex> = OnceLock::new();
    let script = SCRIPT.get_or_init(|| {
        Regex::new(r"(?is)<script(?P<attrs>[^>]*)>(?P<body>.*?)</script>").unwrap()
    });

    let mut scripts = BTreeSet::new();
    for cap in script.captures_iter(html) {
        // skip external scripts (covered by script-src 'self')
        if cap
            .name("attrs")
            .map(|a| a.as_str().to_ascii_lowercase().contains("src"))
            .unwrap_or(false)
        {
            continue;
        }
        let body = cap.name("body").map(|m| m.as_str()).unwrap_or("");
        if !body.is_empty() {
            scripts.insert(sha256_b64(body));
        }
    }
    scripts.into_iter().collect()
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
            format!(
                "[{}]",
                a.iter().map(toml_to_json).collect::<Vec<_>>().join(",")
            )
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
        assert!(
            out.contains("data-island=\"poll\""),
            "marker injected: {out}"
        );
        assert!(out.contains("data-island-worker=\"./_static/islands/poll.js\""));
        assert!(out.contains("data-island-paints=\"true\""));
        assert!(out.contains("content:vote"), "grants/types present");
        assert!(
            out.contains("zetl-islands.js"),
            "runtime bootstrap injected"
        );
        assert!(
            out.contains("Content-Security-Policy"),
            "CSP meta injected for content island"
        );
    }

    #[test]
    fn no_island_on_page_is_byte_identical() {
        let d = poll_vault();
        let set = IslandSet::discover(d.path(), "default").unwrap();
        let page = "<html><head></head><body><p>no islands here</p></body></html>";
        let out = set.inject_into_page(page, "./");
        assert_eq!(
            out, page,
            "page without islands must be untouched (REQ-5012)"
        );
    }
}
