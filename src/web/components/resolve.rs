//! SPEC-048 REQ-4804 — Component resolution via the three-tier fallback.
//!
//! Lookup by `<name>` reuses the existing theme precedence: vault
//! `.zetl/components/<name>/` overrides the active theme's `components/<name>/`, which
//! overrides the bundled default theme's. Resolution is **whole-directory** (template +
//! manifest + CSS resolve as a unit). An unresolvable name is `component-not-found`.

use super::css::Layer;
use super::manifest::{parse_manifest, Manifest};
use super::{CResult, ComponentError};
use std::collections::BTreeMap;
use std::path::Path;

/// A fully resolved component: its macro template source, typed manifest, optional CSS,
/// and the layer it came from (for OBS-4801 override visibility).
#[derive(Debug, Clone)]
pub struct ResolvedComponent {
    pub name: String,
    pub template: String,
    pub manifest: Manifest,
    pub css: Option<String>,
    /// Optional island client script `<name>.js` (SPEC-050). Ignored by SPEC-048; emitted
    /// as a worker asset by the islands pipeline when the component declares island fields.
    pub js: Option<String>,
    pub layer: Layer,
}

fn load_from_dir(dir: &Path, name: &str, layer: Layer) -> CResult<ResolvedComponent> {
    let template_path = dir.join(format!("{name}.html"));
    let manifest_path = dir.join(format!("{name}.toml"));
    let template = std::fs::read_to_string(&template_path).map_err(|_| {
        ComponentError::new(
            "component-malformed",
            format!("component `{name}` missing template {name}.html"),
        )
    })?;
    let manifest_src = std::fs::read_to_string(&manifest_path).map_err(|_| {
        ComponentError::new(
            "component-malformed",
            format!("component `{name}` missing manifest {name}.toml"),
        )
    })?;
    let manifest = parse_manifest(&manifest_src, name)?;
    let css = std::fs::read_to_string(dir.join(format!("{name}.css"))).ok();
    let js = std::fs::read_to_string(dir.join(format!("{name}.js"))).ok();
    Ok(ResolvedComponent {
        name: name.to_string(),
        template,
        manifest,
        css,
        js,
        layer,
    })
}

fn load_from_bundled(theme: &str, name: &str, layer: Layer) -> CResult<Option<ResolvedComponent>> {
    let rel = |ext: &str| format!("components/{name}/{name}.{ext}");
    let Some(template) = crate::web::engine::bundled_template(theme, &rel("html")) else {
        return Ok(None);
    };
    let manifest_src =
        crate::web::engine::bundled_template(theme, &rel("toml")).ok_or_else(|| {
            ComponentError::new(
                "component-malformed",
                format!("bundled component `{name}` missing manifest"),
            )
        })?;
    let manifest = parse_manifest(manifest_src, name)?;
    let css = crate::web::engine::bundled_template(theme, &rel("css")).map(|s| s.to_string());
    let js = crate::web::engine::bundled_template(theme, &rel("js")).map(|s| s.to_string());
    Ok(Some(ResolvedComponent {
        name: name.to_string(),
        template: template.to_string(),
        manifest,
        css,
        js,
        layer,
    }))
}

/// Resolve a single component by name across the three tiers.
pub fn resolve_component(vault_root: &Path, theme: &str, name: &str) -> CResult<ResolvedComponent> {
    // Tier 1: vault on-disk
    let vault_dir = vault_root.join(".zetl/components").join(name);
    if vault_dir.join(format!("{name}.html")).is_file() {
        return load_from_dir(&vault_dir, name, Layer::Vault);
    }
    // Tier 2: active theme on-disk
    if theme != "default" {
        let theme_dir = vault_root
            .join(".zetl/themes")
            .join(theme)
            .join("components")
            .join(name);
        if theme_dir.join(format!("{name}.html")).is_file() {
            return load_from_dir(&theme_dir, name, Layer::Theme);
        }
    }
    // Tier 3: bundled active theme, then bundled default
    if theme != "default" {
        if let Some(c) = load_from_bundled(theme, name, Layer::Theme)? {
            return Ok(c);
        }
    }
    if let Some(c) = load_from_bundled("default", name, Layer::Default)? {
        return Ok(c);
    }
    Err(ComponentError::new(
        "component-not-found",
        format!("component `{name}` is not resolvable in vault, theme `{theme}`, or default"),
    ))
}

/// Discover every component available to a build, applying whole-directory precedence
/// (vault > theme-disk > bundled-theme > bundled-default). Used to build the synthetic
/// macro template and the static invocation graph.
pub fn discover_components(
    vault_root: &Path,
    theme: &str,
) -> CResult<BTreeMap<String, ResolvedComponent>> {
    let mut found: BTreeMap<String, ResolvedComponent> = BTreeMap::new();

    let insert_dir_tier = |base: &Path,
                           layer: Layer,
                           found: &mut BTreeMap<String, ResolvedComponent>|
     -> CResult<()> {
        let Ok(entries) = std::fs::read_dir(base) else {
            return Ok(());
        };
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(|s| s.to_string()) else {
                continue;
            };
            if found.contains_key(&name) {
                continue; // higher tier already won
            }
            if entry.path().join(format!("{name}.html")).is_file() {
                found.insert(name.clone(), load_from_dir(&entry.path(), &name, layer)?);
            }
        }
        Ok(())
    };

    insert_dir_tier(
        &vault_root.join(".zetl/components"),
        Layer::Vault,
        &mut found,
    )?;
    if theme != "default" {
        insert_dir_tier(
            &vault_root
                .join(".zetl/themes")
                .join(theme)
                .join("components"),
            Layer::Theme,
            &mut found,
        )?;
    }
    // bundled tiers
    for (t, layer) in [(theme, Layer::Theme), ("default", Layer::Default)] {
        for (path, _content) in crate::web::engine::bundled_theme_files(t) {
            let p = path.to_string_lossy();
            // components/<name>/<name>.html
            if let Some(rest) = p.strip_prefix("components/") {
                if let Some((name, file)) = rest.split_once('/') {
                    if file == format!("{name}.html") && !found.contains_key(name) {
                        if let Some(c) = load_from_bundled(t, name, layer)? {
                            found.insert(name.to_string(), c);
                        }
                    }
                }
            }
        }
    }

    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_component(dir: &Path, name: &str, body: &str, requires: &str) {
        let cdir = dir.join(name);
        fs::create_dir_all(&cdir).unwrap();
        fs::write(cdir.join(format!("{name}.html")), body).unwrap();
        fs::write(
            cdir.join(format!("{name}.toml")),
            format!("name = \"{name}\"\nrequires = [{requires}]\n"),
        )
        .unwrap();
    }

    #[test]
    fn resolves_vault_component() {
        let tmp = std::env::temp_dir().join(format!("zetl-resolve-{}", std::process::id()));
        let comp_dir = tmp.join(".zetl/components");
        fs::create_dir_all(&comp_dir).unwrap();
        write_component(
            &comp_dir,
            "callout",
            "<div>{{ caller() }}</div>",
            "\"site\"",
        );
        let c = resolve_component(&tmp, "default", "callout").unwrap();
        assert_eq!(c.name, "callout");
        assert_eq!(c.layer, Layer::Vault);
        assert!(c
            .manifest
            .requires_tier(crate::web::components::manifest::Tier::Site));
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn unknown_component_not_found() {
        let tmp = std::env::temp_dir().join(format!("zetl-resolve-none-{}", std::process::id()));
        fs::create_dir_all(&tmp).unwrap();
        assert_eq!(
            resolve_component(&tmp, "default", "ghost")
                .unwrap_err()
                .code,
            "component-not-found"
        );
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn vault_shadows_for_discovery() {
        let tmp = std::env::temp_dir().join(format!("zetl-discover-{}", std::process::id()));
        let comp_dir = tmp.join(".zetl/components");
        fs::create_dir_all(&comp_dir).unwrap();
        write_component(
            &comp_dir,
            "nav-header",
            "<nav>{{ caller() }}</nav>",
            "\"site\"",
        );
        let all = discover_components(&tmp, "default").unwrap();
        assert!(all.contains_key("nav-header"));
        assert_eq!(all["nav-header"].layer, Layer::Vault);
        fs::remove_dir_all(&tmp).ok();
    }
}
