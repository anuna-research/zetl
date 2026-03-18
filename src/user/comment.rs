//! Page comment storage (REQ-020-051).
//!
//! Comments are stored as sidecar JSON files at `.zetl/comments/<slug>.json`.
//! They are NOT part of the page content — they don't affect markdown, merkle,
//! git history, or the link graph.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

const COMMENTS_DIR: &str = ".zetl/comments";

/// 30 days in seconds (default auto-prune threshold).
const DEFAULT_PRUNE_AGE_SECS: u64 = 30 * 24 * 3600;

/// A single page comment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Comment {
    /// User ID of the commenter (e.g. "alice-abc123").
    pub user: String,
    /// Comment text.
    pub text: String,
    /// ISO-8601 timestamp of when the comment was posted.
    pub at: String,
}

/// Load comments for a page slug from `.zetl/comments/<slug>.json`.
///
/// Returns an empty vector if the file does not exist.
pub fn load_comments(vault_root: &Path, slug: &str) -> Result<Vec<Comment>> {
    let path = comments_path(vault_root, slug);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    if content.trim().is_empty() {
        return Ok(Vec::new());
    }
    let comments: Vec<Comment> = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(comments)
}

/// Save the full list of comments for a page slug.
pub fn save_comments(vault_root: &Path, slug: &str, comments: &[Comment]) -> Result<()> {
    let dir = vault_root.join(COMMENTS_DIR);
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create {}", dir.display()))?;
    let path = comments_path(vault_root, slug);
    if comments.is_empty() {
        // Remove the file if no comments remain after pruning
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
        }
        return Ok(());
    }
    let json = serde_json::to_string_pretty(comments)
        .context("failed to serialize comments")?;
    fs::write(&path, json)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

/// Append a comment to the page's comment file.
pub fn append_comment(
    vault_root: &Path,
    slug: &str,
    user_id: &str,
    text: &str,
) -> Result<Comment> {
    let mut comments = load_comments(vault_root, slug)?;
    let comment = Comment {
        user: user_id.to_string(),
        text: text.to_string(),
        at: super::access_request::now_iso8601(),
    };
    comments.push(comment.clone());
    save_comments(vault_root, slug, &comments)?;
    Ok(comment)
}

/// Prune comments older than `max_age_secs` from a single slug file.
/// Returns the number of comments removed.
pub fn prune_comments(vault_root: &Path, slug: &str, max_age_secs: u64) -> Result<usize> {
    let comments = load_comments(vault_root, slug)?;
    if comments.is_empty() {
        return Ok(0);
    }
    let cutoff = cutoff_iso8601(max_age_secs);
    let before = comments.len();
    let kept: Vec<Comment> = comments.into_iter().filter(|c| c.at >= cutoff).collect();
    let removed = before - kept.len();
    if removed > 0 {
        save_comments(vault_root, slug, &kept)?;
    }
    Ok(removed)
}

/// Prune all comment files in `.zetl/comments/` that have entries older than 30 days.
/// Returns total number of comments removed.
pub fn prune_all_comments(vault_root: &Path) -> Result<usize> {
    prune_all_comments_with_age(vault_root, DEFAULT_PRUNE_AGE_SECS)
}

/// Prune all comment files with a configurable max age.
pub fn prune_all_comments_with_age(vault_root: &Path, max_age_secs: u64) -> Result<usize> {
    let dir = vault_root.join(COMMENTS_DIR);
    if !dir.exists() {
        return Ok(0);
    }
    let mut total = 0;
    for entry in fs::read_dir(&dir)
        .with_context(|| format!("failed to read {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            let slug = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if !slug.is_empty() {
                total += prune_comments(vault_root, &slug, max_age_secs)?;
            }
        }
    }
    Ok(total)
}

/// Compute the ISO-8601 cutoff timestamp for `max_age_secs` ago.
fn cutoff_iso8601(max_age_secs: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let cutoff_secs = now.saturating_sub(max_age_secs);
    let days = cutoff_secs / 86400;
    let time_of_day = cutoff_secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;
    let (year, month, day) = super::access_request::days_to_ymd(days);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

/// Resolve the file path for a slug's comments.
fn comments_path(vault_root: &Path, slug: &str) -> std::path::PathBuf {
    vault_root.join(COMMENTS_DIR).join(format!("{slug}.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn load_empty_returns_empty_vec() {
        let tmp = TempDir::new().unwrap();
        let comments = load_comments(tmp.path(), "readme").unwrap();
        assert!(comments.is_empty());
    }

    #[test]
    fn append_and_load_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let c = append_comment(tmp.path(), "readme", "alice-abc123", "looks good").unwrap();
        assert_eq!(c.user, "alice-abc123");
        assert_eq!(c.text, "looks good");
        assert!(!c.at.is_empty());

        let comments = load_comments(tmp.path(), "readme").unwrap();
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].text, "looks good");
    }

    #[test]
    fn multiple_comments_appended() {
        let tmp = TempDir::new().unwrap();
        append_comment(tmp.path(), "page", "alice", "first").unwrap();
        append_comment(tmp.path(), "page", "bob", "second").unwrap();
        append_comment(tmp.path(), "page", "alice", "third").unwrap();

        let comments = load_comments(tmp.path(), "page").unwrap();
        assert_eq!(comments.len(), 3);
        assert_eq!(comments[0].text, "first");
        assert_eq!(comments[2].text, "third");
    }

    #[test]
    fn prune_removes_old_comments() {
        let tmp = TempDir::new().unwrap();
        // Manually write a comment with a very old timestamp
        let dir = tmp.path().join(COMMENTS_DIR);
        fs::create_dir_all(&dir).unwrap();
        let comments = vec![
            Comment {
                user: "alice".into(),
                text: "old".into(),
                at: "2020-01-01T00:00:00Z".into(),
            },
            Comment {
                user: "bob".into(),
                text: "recent".into(),
                at: super::super::access_request::now_iso8601(),
            },
        ];
        save_comments(tmp.path(), "page", &comments).unwrap();

        let removed = prune_comments(tmp.path(), "page", DEFAULT_PRUNE_AGE_SECS).unwrap();
        assert_eq!(removed, 1);

        let remaining = load_comments(tmp.path(), "page").unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].text, "recent");
    }

    #[test]
    fn prune_all_walks_directory() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join(COMMENTS_DIR);
        fs::create_dir_all(&dir).unwrap();

        // Old comment in page-a
        let old = vec![Comment {
            user: "alice".into(),
            text: "ancient".into(),
            at: "2020-01-01T00:00:00Z".into(),
        }];
        save_comments(tmp.path(), "page-a", &old).unwrap();

        // Recent comment in page-b
        append_comment(tmp.path(), "page-b", "bob", "fresh").unwrap();

        let removed = prune_all_comments(tmp.path()).unwrap();
        assert_eq!(removed, 1);

        // page-a file should be removed (empty after prune)
        assert!(!comments_path(tmp.path(), "page-a").exists());
        // page-b should still have its comment
        assert_eq!(load_comments(tmp.path(), "page-b").unwrap().len(), 1);
    }

    #[test]
    fn save_empty_removes_file() {
        let tmp = TempDir::new().unwrap();
        append_comment(tmp.path(), "page", "alice", "hi").unwrap();
        assert!(comments_path(tmp.path(), "page").exists());

        save_comments(tmp.path(), "page", &[]).unwrap();
        assert!(!comments_path(tmp.path(), "page").exists());
    }
}
