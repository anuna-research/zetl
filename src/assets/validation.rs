//! Pure-core asset validation and metadata types.
//!
//! No std::fs, no std::env, no randomness. Enforced by clippy lint in CI.

use std::fmt;

/// Error type for slug validation failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlugError {
    Empty,
    NullByte,
    LeadingSlash,
    TrailingSlash,
    DotComponent,
    DotDotComponent,
    EmptyComponent,
    DeviceName { name: String },
}

impl fmt::Display for SlugError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SlugError::Empty => write!(f, "slug is empty"),
            SlugError::NullByte => write!(f, "slug contains null bytes"),
            SlugError::LeadingSlash => write!(f, "slug starts with '/'"),
            SlugError::TrailingSlash => write!(f, "slug ends with '/'"),
            SlugError::DotComponent => write!(f, "slug contains '.' component"),
            SlugError::DotDotComponent => write!(f, "slug contains '..' component"),
            SlugError::EmptyComponent => write!(f, "slug contains empty component ('//')"),
            SlugError::DeviceName { name } => write!(f, "slug component is a device name: {name}"),
        }
    }
}

impl std::error::Error for SlugError {}

/// Windows device names (case-insensitive) rejected in any path component.
static DEVICE_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL",
    "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
    "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Validate an asset slug per REQ-3510.
///
/// Rules:
/// 1. Non-empty.
/// 2. No null bytes.
/// 3. No leading `/`.
/// 4. No trailing `/`.
/// 5. No `.` or `..` components.
/// 6. No empty components (`//`).
/// 7. No Windows device names (case-insensitive).
pub fn validate_slug(slug: &str) -> Result<(), SlugError> {
    if slug.is_empty() {
        return Err(SlugError::Empty);
    }
    if slug.contains('\0') {
        return Err(SlugError::NullByte);
    }
    if slug.starts_with('/') {
        return Err(SlugError::LeadingSlash);
    }
    if slug.ends_with('/') {
        return Err(SlugError::TrailingSlash);
    }

    for component in slug.split('/') {
        if component.is_empty() {
            return Err(SlugError::EmptyComponent);
        }
        if component == "." {
            return Err(SlugError::DotComponent);
        }
        if component == ".." {
            return Err(SlugError::DotDotComponent);
        }
        let upper = component.to_ascii_uppercase();
        if DEVICE_NAMES.contains(&upper.as_str()) {
            return Err(SlugError::DeviceName {
                name: component.to_string(),
            });
        }
    }

    Ok(())
}

/// MIME type allowlist per REQ-3507.
static ALLOWED_MIMES: &[&str] = &[
    // Images
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/svg+xml",
    "image/avif",
    "image/ico",
    "image/x-icon",
    // Documents
    "application/pdf",
    // Text/markup
    "text/html",
    "text/plain",
    "text/markdown",
    "text/css",
    // Data
    "application/json",
    "text/csv",
    // Web scripts
    "application/javascript",
    "text/javascript",
    // Fonts
    "font/woff2",
    "font/woff",
    "font/ttf",
    "application/font-woff2",
    // Audio
    "audio/mpeg",
    "audio/ogg",
    "audio/wav",
    "audio/webm",
    // Video
    "video/mp4",
    "video/webm",
    "video/ogg",
];

/// Check whether a MIME type is in the allowlist.
///
/// Strips parameters (e.g. `; charset=utf-8`) before checking.
pub fn check_mime_allowlist(mime: &str) -> bool {
    let base = mime.split(';').next().unwrap_or(mime).trim();
    ALLOWED_MIMES.contains(&base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_slugs() {
        assert!(validate_slug("foo.png").is_ok());
        assert!(validate_slug("diagrams/architecture.png").is_ok());
        assert!(validate_slug("a/b/c/d.txt").is_ok());
    }

    #[test]
    fn empty_slug_rejected() {
        assert_eq!(validate_slug(""), Err(SlugError::Empty));
    }

    #[test]
    fn null_byte_rejected() {
        assert_eq!(validate_slug("foo\0bar"), Err(SlugError::NullByte));
    }

    #[test]
    fn leading_slash_rejected() {
        assert_eq!(validate_slug("/foo"), Err(SlugError::LeadingSlash));
    }

    #[test]
    fn trailing_slash_rejected() {
        assert_eq!(validate_slug("foo/"), Err(SlugError::TrailingSlash));
    }

    #[test]
    fn dot_component_rejected() {
        assert_eq!(validate_slug("./foo"), Err(SlugError::DotComponent));
        assert_eq!(validate_slug("foo/./bar"), Err(SlugError::DotComponent));
    }

    #[test]
    fn dotdot_component_rejected() {
        assert_eq!(validate_slug("../foo"), Err(SlugError::DotDotComponent));
        assert_eq!(validate_slug("foo/../bar"), Err(SlugError::DotDotComponent));
    }

    #[test]
    fn empty_component_rejected() {
        assert_eq!(validate_slug("foo//bar"), Err(SlugError::EmptyComponent));
    }

    #[test]
    fn device_names_rejected() {
        for name in DEVICE_NAMES {
            assert!(
                validate_slug(name).is_err(),
                "device name {name} should be rejected"
            );
            let mixed = format!("{}/foo", name.to_lowercase());
            assert!(
                validate_slug(&mixed).is_err(),
                "device name {name} in path should be rejected"
            );
        }
    }

    #[test]
    fn allowed_mimes() {
        assert!(check_mime_allowlist("image/png"));
        assert!(check_mime_allowlist("text/html"));
        assert!(check_mime_allowlist("application/json"));
    }

    #[test]
    fn allowed_mimes_strip_params() {
        assert!(check_mime_allowlist("text/html; charset=utf-8"));
    }

    #[test]
    fn disallowed_mimes() {
        assert!(!check_mime_allowlist("application/x-msdownload"));
        assert!(!check_mime_allowlist("application/zip"));
    }
}
