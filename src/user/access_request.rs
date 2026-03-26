//! Access request storage for the collab access-request flow (REQ-020-047).
//!
//! Requests are stored in `.zetl/collab/access-requests.json` as a JSON array
//! of `AccessRequest` objects.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

const COLLAB_DIR: &str = ".zetl/collab";
const ACCESS_REQUESTS_FILE: &str = ".zetl/collab/access-requests.json";

/// A single access request record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AccessRequest {
    /// User ID of the requester (e.g. "bob-d4e5f6").
    pub user: String,
    /// Display name of the requester.
    pub name: String,
    /// Page slug that was requested.
    pub page: String,
    /// ISO-8601 timestamp of when the request was made.
    pub requested_at: String,
    /// Status of the request: "pending", "approved", or "denied".
    pub status: String,
}

/// Load all access requests from `.zetl/collab/access-requests.json`.
///
/// Returns an empty vector if the file does not exist.
pub fn load_access_requests(vault_root: &Path) -> Result<Vec<AccessRequest>> {
    let path = vault_root.join(ACCESS_REQUESTS_FILE);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    if content.trim().is_empty() {
        return Ok(Vec::new());
    }
    let requests: Vec<AccessRequest> = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(requests)
}

/// Save the full list of access requests to `.zetl/collab/access-requests.json`.
pub fn save_access_requests(vault_root: &Path, requests: &[AccessRequest]) -> Result<()> {
    let dir = vault_root.join(COLLAB_DIR);
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    let path = vault_root.join(ACCESS_REQUESTS_FILE);
    let json =
        serde_json::to_string_pretty(requests).context("failed to serialize access requests")?;
    fs::write(&path, json).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

/// Append a new access request. Returns `true` if a new request was created,
/// `false` if a pending request already exists for this user+page.
pub fn append_access_request(
    vault_root: &Path,
    user_id: &str,
    user_name: &str,
    page_slug: &str,
) -> Result<bool> {
    let mut requests = load_access_requests(vault_root)?;

    // Check for existing pending request from same user for same page
    let already_pending = requests
        .iter()
        .any(|r| r.user == user_id && r.page == page_slug && r.status == "pending");
    if already_pending {
        return Ok(false);
    }

    let now = now_iso8601();
    requests.push(AccessRequest {
        user: user_id.to_string(),
        name: user_name.to_string(),
        page: page_slug.to_string(),
        requested_at: now,
        status: "pending".to_string(),
    });

    save_access_requests(vault_root, &requests)?;
    Ok(true)
}

/// Return current UTC time as ISO-8601 string (without external chrono dep).
pub fn now_iso8601() -> String {
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    // Simple UTC timestamp without chrono dependency
    // Format: seconds since epoch as a rough ISO string
    // Use the same approach as other parts of the codebase
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Compute year/month/day from days since epoch (1970-01-01)
    let (year, month, day) = days_to_ymd(days);

    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

/// Convert days since Unix epoch to (year, month, day).
pub fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Civil calendar algorithm
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn load_empty_returns_empty_vec() {
        let tmp = TempDir::new().unwrap();
        let requests = load_access_requests(tmp.path()).unwrap();
        assert!(requests.is_empty());
    }

    #[test]
    fn append_and_load_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let created =
            append_access_request(tmp.path(), "bob-d4e5f6", "Bob", "secret-project").unwrap();
        assert!(created);

        let requests = load_access_requests(tmp.path()).unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].user, "bob-d4e5f6");
        assert_eq!(requests[0].name, "Bob");
        assert_eq!(requests[0].page, "secret-project");
        assert_eq!(requests[0].status, "pending");
        assert!(!requests[0].requested_at.is_empty());
    }

    #[test]
    fn duplicate_pending_not_appended() {
        let tmp = TempDir::new().unwrap();
        assert!(append_access_request(tmp.path(), "bob-d4e5f6", "Bob", "secret").unwrap());
        assert!(!append_access_request(tmp.path(), "bob-d4e5f6", "Bob", "secret").unwrap());

        let requests = load_access_requests(tmp.path()).unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[test]
    fn different_page_is_separate_request() {
        let tmp = TempDir::new().unwrap();
        assert!(append_access_request(tmp.path(), "bob-d4e5f6", "Bob", "page-a").unwrap());
        assert!(append_access_request(tmp.path(), "bob-d4e5f6", "Bob", "page-b").unwrap());

        let requests = load_access_requests(tmp.path()).unwrap();
        assert_eq!(requests.len(), 2);
    }

    #[test]
    fn iso8601_format() {
        let ts = now_iso8601();
        // Should match YYYY-MM-DDTHH:MM:SSZ
        assert!(ts.ends_with('Z'));
        assert_eq!(ts.len(), 20);
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[7..8], "-");
        assert_eq!(&ts[10..11], "T");
    }
}
