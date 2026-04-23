//! Asset metadata types and helpers.
//!
//! Pure core — no I/O, no shared state.

use serde::{Deserialize, Serialize};

/// Metadata sidecar for an uploaded asset.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AssetMeta {
    pub slug: String,
    pub original_filename: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub uploaded_by: String,
    pub uploaded_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaced_at: Option<String>,
    #[serde(default = "default_version")]
    pub version: u8,
}

fn default_version() -> u8 {
    1
}

impl AssetMeta {
    /// Human-readable size for git commit messages.
    pub fn size_human(&self) -> String {
        human_size(self.size_bytes)
    }
}

/// Return the correct `Cache-Control` header value per REQ-3515.
pub fn cache_control_for(meta: &AssetMeta) -> &'static str {
    if meta.replaced_at.is_some() {
        "public, max-age=300"
    } else {
        "public, max-age=31536000, immutable"
    }
}

/// Format a byte count as human-readable (e.g. `48 KiB`, `1.2 MiB`).
pub fn human_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    if bytes == 0 {
        return "0 B".to_string();
    }
    let mut value = bytes as f64;
    let mut unit_idx = 0;
    while value >= 1024.0 && unit_idx + 1 < UNITS.len() {
        value /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{:.0} {}", value, UNITS[unit_idx])
    } else {
        format!("{:.1} {}", value, UNITS[unit_idx])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_control_new_asset() {
        let meta = AssetMeta {
            slug: "a.png".into(),
            original_filename: "a.png".into(),
            mime_type: "image/png".into(),
            size_bytes: 100,
            sha256: "abc".into(),
            uploaded_by: "u1".into(),
            uploaded_at: "2026-04-22T10:00:00Z".into(),
            replaced_at: None,
            version: 1,
        };
        assert_eq!(cache_control_for(&meta), "public, max-age=31536000, immutable");
    }

    #[test]
    fn cache_control_replaced_asset() {
        let meta = AssetMeta {
            slug: "a.png".into(),
            original_filename: "a.png".into(),
            mime_type: "image/png".into(),
            size_bytes: 100,
            sha256: "abc".into(),
            uploaded_by: "u1".into(),
            uploaded_at: "2026-04-22T10:00:00Z".into(),
            replaced_at: Some("2026-04-22T11:00:00Z".into()),
            version: 1,
        };
        assert_eq!(cache_control_for(&meta), "public, max-age=300");
    }

    #[test]
    fn human_size_cases() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(100), "100 B");
        assert_eq!(human_size(1024), "1.0 KiB");
        assert_eq!(human_size(1536), "1.5 KiB");
        assert_eq!(human_size(1024 * 1024), "1.0 MiB");
        assert_eq!(human_size(10 * 1024 * 1024), "10.0 MiB");
    }
}
