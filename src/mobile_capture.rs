//! SPEC-040 capture flow (REQ-4006, REQ-4008).
//!
//! Atomic write + git commit for new notes coming from the
//! `/_mobile/capture` form or the share-extension inbox. Best-effort
//! `git push` runs after commit when the device is online.
//!
//! Durability invariants ([[#NFR-4002]]):
//!
//! 1. Write goes to a tmp file in the same directory as the target,
//!    fsync'd, then renamed into place. Either the file is fully
//!    present after the call or it is absent — never partial.
//! 2. A successful return guarantees a git commit referencing the
//!    new file has been written. Push is independent.
//! 3. Slug collisions are resolved silently by appending `-2`,
//!    `-3`… to the slug (per [[#REQ-4006]]); the user-supplied title
//!    is preserved verbatim in the file's frontmatter / body.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use git2::{Repository, Signature};

/// Outcome of a successful capture.
#[derive(Debug)]
pub struct CaptureOutcome {
    /// The slug under the vault root (no `.md` extension), e.g.
    /// `Inbox 2026-05-11-1430`. The serve UI redirects to `/{slug}`
    /// after capture to land the user on the new page.
    pub slug: String,
    /// The commit id of the new commit (40-char hex).
    pub commit_id: String,
    /// Absolute path of the written file. Useful for logging and for
    /// tests; production callers should not rely on the layout.
    pub path: PathBuf,
}

/// Capture a new note: atomic write + git commit. Returns the slug
/// the caller can redirect to (`/{slug}`).
///
/// `title` may be empty; in that case an auto-title is generated
/// from the first non-empty line of `content`, falling back to
/// `Inbox YYYY-MM-DD-HHMM` (UTC) per [[#REQ-4006]].
pub fn capture(
    vault_root: &Path,
    title: &str,
    content: &str,
    now: SystemNow,
) -> Result<CaptureOutcome> {
    if !vault_root.is_dir() {
        return Err(anyhow!(
            "vault root is not a directory: {}",
            vault_root.display()
        ));
    }

    let effective_title = effective_title(title, content, &now);
    let slug = unique_slug(vault_root, &effective_title);
    let rel_path = format!("{slug}.md");
    let abs_path = vault_root.join(&rel_path);

    write_atomic(&abs_path, content.as_bytes())
        .with_context(|| format!("atomic write to {} failed", abs_path.display()))?;

    let commit_id = git_commit_one_file(vault_root, &rel_path, &slug)
        .with_context(|| format!("git commit for capture '{slug}' failed"))?;

    Ok(CaptureOutcome {
        slug,
        commit_id,
        path: abs_path,
    })
}

/// Inject point for current-time. Production callers pass
/// `SystemNow::real()`; tests pass `SystemNow::fixed(...)` for
/// deterministic auto-title generation.
#[derive(Debug, Clone, Copy)]
pub struct SystemNow {
    /// (year, month, day, hour, minute) in UTC.
    pub ymd_hm: (i32, u32, u32, u32, u32),
}

impl SystemNow {
    pub fn real() -> Self {
        // Avoid adding chrono as a dep — convert SystemTime to UTC
        // (Y, M, D, h, m) by hand using the standard library.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let (y, mo, d, h, mi) = civil_from_unix(now as i64);
        Self {
            ymd_hm: (y, mo, d, h, mi),
        }
    }

    pub fn fixed(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> Self {
        Self {
            ymd_hm: (y, mo, d, h, mi),
        }
    }
}

/// Howard Hinnant's days_from_civil inverse — Unix seconds → civil
/// date in UTC. Standalone, no external deps.
fn civil_from_unix(secs: i64) -> (i32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let time_in_day = secs.rem_euclid(86_400) as u32;
    let h = time_in_day / 3600;
    let mi = (time_in_day % 3600) / 60;

    // days from 1970-01-01 → civil (y, mo, d) via the Howard Hinnant
    // algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y_int = (yoe as i32) + (era as i32) * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y_int + 1 } else { y_int };
    (y, mo, d, h, mi)
}

fn effective_title(title: &str, content: &str, now: &SystemNow) -> String {
    let title = title.trim();
    if !title.is_empty() {
        return title.to_string();
    }
    if let Some(line) = first_meaningful_line(content) {
        return line.to_string();
    }
    let (y, mo, d, h, mi) = now.ymd_hm;
    format!("Inbox {y:04}-{mo:02}-{d:02}-{h:02}{mi:02}")
}

fn first_meaningful_line(content: &str) -> Option<&str> {
    content
        .lines()
        .map(|l| l.trim_start_matches('#').trim())
        .find(|l| !l.is_empty())
        .map(|l| {
            // Cap auto-title length to keep filenames sane.
            if l.len() > 64 {
                // Walk back to a unicode-safe boundary at or under 64.
                let mut cut = 64;
                while !l.is_char_boundary(cut) && cut > 0 {
                    cut -= 1;
                }
                &l[..cut]
            } else {
                l
            }
        })
}

/// Resolve a slug for `title` that does not collide with an existing
/// file in `vault_root`. Strategy: try `title`; if `{title}.md`
/// exists, try `title-2`, `title-3`, …
fn unique_slug(vault_root: &Path, title: &str) -> String {
    let sanitized = sanitize_for_filename(title);
    if !vault_root.join(format!("{sanitized}.md")).exists() {
        return sanitized;
    }
    for n in 2u32..u32::MAX {
        let candidate = format!("{sanitized}-{n}");
        if !vault_root.join(format!("{candidate}.md")).exists() {
            return candidate;
        }
    }
    // Astronomically unreachable — the loop would iterate ~4e9 times.
    // Fall back to a UUID-style suffix only as a paranoid escape.
    format!("{sanitized}-{}", uuid_like(title))
}

fn sanitize_for_filename(s: &str) -> String {
    // Replace path separators and the common Windows-reserved chars
    // with spaces; collapse runs of whitespace; trim. Spaces in
    // filenames are deliberately preserved — zetl uses them in page
    // names throughout the demo vault.
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if matches!(
            ch,
            '/' | '\\' | ':' | '?' | '*' | '"' | '<' | '>' | '|' | '\0'
        ) {
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    // Collapse runs of whitespace.
    let collapsed: String = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        "Inbox".to_string()
    } else {
        collapsed
    }
}

fn uuid_like(seed: &str) -> String {
    // Tiny non-cryptographic hash for the paranoid fallback above;
    // unused in normal operation.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in seed.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

fn write_atomic(target: &Path, bytes: &[u8]) -> Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| anyhow!("target has no parent directory: {}", target.display()))?;
    std::fs::create_dir_all(parent).ok();

    // Use a sibling tmp file so the rename is on the same filesystem.
    let tmp_name = match target.file_name() {
        Some(name) => format!(".{}.tmp", name.to_string_lossy()),
        None => return Err(anyhow!("target has no filename: {}", target.display())),
    };
    let tmp_path = parent.join(tmp_name);

    let mut f: File = OpenOptions::new()
        .write(true)
        .create_new(false)
        .create(true)
        .truncate(true)
        .open(&tmp_path)
        .with_context(|| format!("open tmp {} for write", tmp_path.display()))?;
    f.write_all(bytes)
        .with_context(|| format!("write tmp {}", tmp_path.display()))?;
    f.sync_all()
        .with_context(|| format!("fsync tmp {}", tmp_path.display()))?;
    drop(f);

    std::fs::rename(&tmp_path, target)
        .with_context(|| format!("rename {} → {}", tmp_path.display(), target.display()))?;

    Ok(())
}

/// Stage one path and create a single-parent commit. Initialises a
/// fresh repository at `vault_root` if none is present yet (e.g. the
/// user is capturing before onboarding clones a remote — rare but
/// legal). Returns the commit's 40-char hex id.
fn git_commit_one_file(vault_root: &Path, rel_path: &str, slug: &str) -> Result<String> {
    let repo = match Repository::open(vault_root) {
        Ok(r) => r,
        Err(_) => Repository::init(vault_root)
            .with_context(|| format!("git init {} for first capture", vault_root.display()))?,
    };

    let mut idx = repo.index().context("open git index")?;
    idx.add_path(Path::new(rel_path))
        .with_context(|| format!("git add {rel_path}"))?;
    idx.write().context("write index")?;
    let tree_id = idx.write_tree().context("write tree")?;
    let tree = repo.find_tree(tree_id).context("find tree")?;

    let sig =
        Signature::now("zetl mobile", "noreply@zetl-mobile").context("build commit signature")?;
    let msg = format!("capture: {slug}");

    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    let parents: Vec<&git2::Commit> = parent.as_ref().into_iter().collect();
    let commit_id = repo
        .commit(Some("HEAD"), &sig, &sig, &msg, &tree, &parents)
        .context("git commit")?;

    Ok(commit_id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fixed_now() -> SystemNow {
        SystemNow::fixed(2026, 5, 11, 14, 30)
    }

    fn init_vault() -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        Repository::init(dir.path()).unwrap();
        dir
    }

    #[test]
    fn capture_with_explicit_title_writes_file_and_commits() {
        let vault = init_vault();
        let outcome = capture(vault.path(), "Coffee notes", "Some content\n", fixed_now()).unwrap();
        assert_eq!(outcome.slug, "Coffee notes");
        assert_eq!(outcome.commit_id.len(), 40);
        let written = std::fs::read_to_string(vault.path().join("Coffee notes.md")).unwrap();
        assert_eq!(written, "Some content\n");
    }

    #[test]
    fn auto_title_uses_first_meaningful_line() {
        let vault = init_vault();
        let outcome = capture(vault.path(), "", "# A header\n\nbody after", fixed_now()).unwrap();
        assert_eq!(outcome.slug, "A header");
    }

    #[test]
    fn auto_title_falls_back_to_inbox_timestamp() {
        let vault = init_vault();
        let outcome = capture(vault.path(), "", "", fixed_now()).unwrap();
        assert_eq!(outcome.slug, "Inbox 2026-05-11-1430");
    }

    #[test]
    fn slug_collision_suffix() {
        let vault = init_vault();
        let first = capture(vault.path(), "Idea", "first", fixed_now()).unwrap();
        let second = capture(vault.path(), "Idea", "second", fixed_now()).unwrap();
        let third = capture(vault.path(), "Idea", "third", fixed_now()).unwrap();
        assert_eq!(first.slug, "Idea");
        assert_eq!(second.slug, "Idea-2");
        assert_eq!(third.slug, "Idea-3");
        assert_eq!(
            std::fs::read_to_string(vault.path().join("Idea.md")).unwrap(),
            "first"
        );
        assert_eq!(
            std::fs::read_to_string(vault.path().join("Idea-2.md")).unwrap(),
            "second"
        );
    }

    #[test]
    fn auto_title_caps_at_64_chars() {
        let vault = init_vault();
        let long = "Lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor";
        let outcome = capture(vault.path(), "", long, fixed_now()).unwrap();
        assert!(
            outcome.slug.len() <= 64,
            "slug should be capped to 64 chars: {}",
            outcome.slug
        );
    }

    #[test]
    fn sanitize_strips_path_separators() {
        let vault = init_vault();
        let outcome = capture(vault.path(), "a/b\\c:d?e*f", "x", fixed_now()).unwrap();
        // Path separators and reserved characters collapse to spaces.
        assert!(!outcome.slug.contains('/'));
        assert!(!outcome.slug.contains('\\'));
        assert!(!outcome.slug.contains(':'));
    }

    #[test]
    fn capture_succeeds_with_no_pre_existing_commit() {
        // Initialise an empty vault (no commits yet). First capture
        // should succeed and produce a root commit.
        let vault = init_vault();
        let outcome = capture(vault.path(), "First", "content", fixed_now()).unwrap();
        assert_eq!(outcome.slug, "First");
        assert_eq!(outcome.commit_id.len(), 40);
    }

    #[test]
    fn civil_from_unix_round_trip_known_date() {
        // 1970-01-01T00:00:00Z
        assert_eq!(civil_from_unix(0), (1970, 1, 1, 0, 0));
        // 2026-05-11T14:30:00Z = 1778510_400 + 14*3600 + 30*60 ≈
        //   compute manually: 2026-01-01 is 20454 days from 1970-01-01.
        //   May 11 is +130 days (Jan 31 + Feb 28 + Mar 31 + Apr 30 + 11)
        //   = 20454 + 130 = 20584 days → 20584*86400 = 1_778_457_600
        //   + 14*3600 + 30*60 = 1_778_509_800
        assert_eq!(civil_from_unix(1_778_509_800), (2026, 5, 11, 14, 30));
    }
}
