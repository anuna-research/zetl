//! MCP resource handlers (page directory listing).

use std::path::PathBuf;

use rmcp::model::{
    ListResourcesResult, RawResource, ReadResourceResult, Resource, ResourceContents,
};

/// URI prefix for vault page resources.
const PAGE_URI_PREFIX: &str = "ztl://vault/pages/";

/// Build a `Resource` entry for a single vault page.
fn page_resource(page_name: &str) -> Resource {
    Resource {
        raw: RawResource {
            uri: format!("{PAGE_URI_PREFIX}{}", urlencoding::encode(page_name)),
            name: page_name.to_string(),
            title: None,
            description: Some(format!("Vault page: {page_name}")),
            mime_type: Some("text/markdown".to_string()),
            size: None,
            icons: None,
            meta: None,
        },
        annotations: None,
    }
}

/// List all vault pages as MCP resources.
pub fn list_resources(page_names: &[String]) -> ListResourcesResult {
    let resources: Vec<Resource> = page_names.iter().map(|n| page_resource(n)).collect();
    ListResourcesResult::with_all_items(resources)
}

/// Read a single page resource by its `ztl://vault/pages/<name>` URI.
///
/// Returns the raw Markdown content of the page, or an error if the URI
/// is unrecognised or the page doesn't exist on disk.
pub fn read_resource(
    uri: &str,
    file_index: &[(String, PathBuf)],
    vault_root: &std::path::Path,
) -> Result<ReadResourceResult, rmcp::ErrorData> {
    // Strip URI prefix.
    let encoded_name = uri.strip_prefix(PAGE_URI_PREFIX).ok_or_else(|| {
        rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INVALID_PARAMS,
            format!("unrecognised resource URI: {uri}"),
            None,
        )
    })?;

    let page_name = urlencoding::decode(encoded_name)
        .map_err(|e| {
            rmcp::ErrorData::new(
                rmcp::model::ErrorCode::INVALID_PARAMS,
                format!("invalid URI encoding: {e}"),
                None,
            )
        })?
        .into_owned();

    // Case-insensitive lookup in file_index.
    let lower = page_name.to_lowercase();
    let entry = file_index
        .iter()
        .find(|(name, _)| name.to_lowercase() == lower);

    let (resolved_name, rel_path) = match entry {
        Some((n, p)) => (n.clone(), p.clone()),
        None => {
            return Err(rmcp::ErrorData::new(
                rmcp::model::ErrorCode::INVALID_PARAMS,
                format!("page not found: {page_name}"),
                None,
            ));
        }
    };

    let abs_path = vault_root.join(&rel_path);
    let content = std::fs::read_to_string(&abs_path).map_err(|e| {
        rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INTERNAL_ERROR,
            format!("failed to read {}: {e}", abs_path.display()),
            None,
        )
    })?;

    let resource_uri = format!("{PAGE_URI_PREFIX}{}", urlencoding::encode(&resolved_name));

    Ok(ReadResourceResult::new(vec![
        ResourceContents::TextResourceContents {
            uri: resource_uri,
            mime_type: Some("text/markdown".to_string()),
            text: content,
            meta: None,
        },
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_resources_returns_all_pages() {
        let pages = vec!["README".to_string(), "Architecture".to_string()];
        let result = list_resources(&pages);
        assert_eq!(result.resources.len(), 2);
        assert_eq!(result.resources[0].raw.name, "README");
        assert!(result.resources[0]
            .raw
            .uri
            .starts_with("ztl://vault/pages/"));
        assert_eq!(result.resources[1].raw.name, "Architecture");
    }

    #[test]
    fn list_resources_empty() {
        let result = list_resources(&[]);
        assert!(result.resources.is_empty());
    }

    #[test]
    fn read_resource_unknown_uri() {
        let file_index = vec![];
        let err = read_resource(
            "https://example.com/foo",
            &file_index,
            std::path::Path::new("/tmp"),
        );
        assert!(err.is_err());
        let e = err.unwrap_err();
        assert!(e.message.contains("unrecognised resource URI"));
    }

    #[test]
    fn read_resource_page_not_found() {
        let file_index = vec![("README".to_string(), PathBuf::from("README.md"))];
        let err = read_resource(
            "ztl://vault/pages/Nonexistent",
            &file_index,
            std::path::Path::new("/tmp"),
        );
        assert!(err.is_err());
        let e = err.unwrap_err();
        assert!(e.message.contains("page not found"));
    }

    #[test]
    fn read_resource_success() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("README.md"), "# Hello\n").unwrap();
        let file_index = vec![("README".to_string(), PathBuf::from("README.md"))];
        let result = read_resource("ztl://vault/pages/README", &file_index, dir.path());
        assert!(result.is_ok());
        let res = result.unwrap();
        assert_eq!(res.contents.len(), 1);
        match &res.contents[0] {
            ResourceContents::TextResourceContents { text, uri, .. } => {
                assert_eq!(text, "# Hello\n");
                assert!(uri.contains("README"));
            }
            _ => panic!("expected TextResourceContents"),
        }
    }
}
