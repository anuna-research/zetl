//! Pure temporal functions for SPEC-017.
//!
//! This module provides [`parse_time_expr`], [`resolve_snapshot`], and the
//! history-timeline helpers:
//!
//! - **[`parse_time_expr`]** — pure parser; no I/O, no VCS calls.
//!   Recognises ISO 8601 dates/datetimes, relative natural-language
//!   expressions, and git-style refs (`HEAD~N`, change-ID prefixes).
//!
//! - **[`resolve_snapshot`]** — walk a snapshot list (newest-first) and
//!   return the most recent entry at or before the resolved time.
//!
//! - **[`compute_graph_delta`]** — pure diff between two vault indexes (REQ-080).
//!
//! - **[`collapse_timeline`]** — collapse identical `vault_root_hash` entries (CON-025).
//!
//! - **[`build_vault_history`]** — load cached indexes and build a delta timeline.

use anyhow::{anyhow, bail};
use chrono::{DateTime, Datelike as _, Duration, FixedOffset, NaiveDate, TimeZone as _, Weekday};
use serde::Serialize;
use std::collections::HashSet;

use crate::history::jj_backend::ChangeInfo;
use crate::types::ParsedFile;

// ─── Public types ─────────────────────────────────────────────────────────────

/// The result of parsing a time expression.
///
/// Variants are evaluated against a snapshot list by [`resolve_snapshot`].
#[derive(Debug, Clone, PartialEq)]
pub enum TimeExpr {
    /// An absolute point in time derived from ISO 8601 or relative NL.
    Absolute(DateTime<FixedOffset>),
    /// The Nth ancestor of the most recent snapshot (`HEAD~N`; `HEAD` = `HEAD~0`).
    HeadOffset(usize),
    /// A jj change-ID prefix or bookmark/branch name.
    Ref(String),
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Parse a time expression string into a [`TimeExpr`].
///
/// `now` is used as the reference point for relative expressions.
///
/// # Supported forms
///
/// | Form | Example |
/// |---|---|
/// | ISO 8601 date | `2024-01-15` |
/// | ISO 8601 datetime | `2024-01-15T14:30:00Z` |
/// | ISO 8601 datetime with offset | `2024-01-15T14:30:00+05:30` |
/// | Today | `today` |
/// | Yesterday | `yesterday` |
/// | N units ago | `3 days ago`, `2 weeks ago`, `1 hour ago` |
/// | Last weekday | `last monday`, `last friday` |
/// | Last week | `last week` |
/// | HEAD | `HEAD`, `HEAD~0` |
/// | HEAD ancestor | `HEAD~1`, `HEAD~7` |
/// | VCS ref | `zkpqwxlmnop` (change-ID prefix), `main`, `dev` |
pub fn parse_time_expr(expr: &str, now: DateTime<FixedOffset>) -> anyhow::Result<TimeExpr> {
    let s = expr.trim();

    if s.is_empty() {
        bail!("empty time expression");
    }

    // HEAD or HEAD~N
    if let Some(offset) = parse_head_offset(s) {
        return Ok(TimeExpr::HeadOffset(offset));
    }

    // ISO 8601 datetime with timezone
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(TimeExpr::Absolute(dt));
    }

    // ISO 8601 date only (YYYY-MM-DD) → end of that day (23:59:59) so that
    // all snapshots taken during the day are included in a ≤ comparison.
    if let Ok(nd) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let dt = nd
            .and_hms_opt(23, 59, 59)
            .and_then(|ndt| now.timezone().from_local_datetime(&ndt).single())
            .ok_or_else(|| anyhow!("could not convert date {s} to datetime"))?;
        return Ok(TimeExpr::Absolute(dt));
    }

    // Relative natural-language
    if let Some(dt) = parse_relative(s, now) {
        return Ok(TimeExpr::Absolute(dt));
    }

    // Fall through: treat as a VCS ref (change-ID prefix or bookmark name)
    Ok(TimeExpr::Ref(s.to_owned()))
}

/// Return the most recent [`ChangeInfo`] at or before the time described by
/// `expr`, or an error if no matching snapshot exists.
///
/// `snapshots` must be ordered **newest-first** (as returned by
/// [`VcsBackend::list_changes`]).
///
/// # Errors
///
/// Returns `SNAPSHOT_NOT_FOUND` when:
/// - `snapshots` is empty, or
/// - no snapshot timestamp is ≤ the resolved absolute time, or
/// - a `HEAD~N` offset exceeds the number of available snapshots, or
/// - no snapshot matches the supplied ref string.
pub fn resolve_snapshot<'a>(
    expr: &str,
    now: DateTime<FixedOffset>,
    snapshots: &'a [ChangeInfo],
) -> anyhow::Result<&'a ChangeInfo> {
    if snapshots.is_empty() {
        bail!("SNAPSHOT_NOT_FOUND: no snapshots available");
    }

    let time_expr = parse_time_expr(expr, now)?;

    match time_expr {
        TimeExpr::Absolute(target) => snapshots
            .iter()
            .find(|c| c.timestamp <= target)
            .ok_or_else(|| {
                anyhow!("SNAPSHOT_NOT_FOUND: no snapshot at or before {target}")
            }),

        TimeExpr::HeadOffset(n) => snapshots.get(n).ok_or_else(|| {
            anyhow!(
                "SNAPSHOT_NOT_FOUND: HEAD~{n} is out of range (only {} snapshot(s))",
                snapshots.len()
            )
        }),

        TimeExpr::Ref(ref ref_str) => snapshots
            .iter()
            .find(|c| {
                c.change_id.starts_with(ref_str.as_str())
                    || c.commit_id.starts_with(ref_str.as_str())
                    || c.description == *ref_str
            })
            .ok_or_else(|| {
                anyhow!("SNAPSHOT_NOT_FOUND: no snapshot matching ref {ref_str:?}")
            }),
    }
}

// ─── Snapshot description helpers ────────────────────────────────────────────

/// Extract the `vault_root_hash` value embedded in a jj snapshot description
/// of the form `"zetl-snapshot vault_root_hash=<64-hex-char-hash>"`.
///
/// Returns `None` when the description does not contain a 64-character
/// lowercase hex hash.
///
/// # Examples
///
/// ```
/// use zetl::history::core::extract_vault_root_hash_from_description;
///
/// // Exactly 64 hex digits after the key.
/// let hash = "a".repeat(64);
/// let desc = format!("zetl-snapshot vault_root_hash={hash}");
/// assert!(extract_vault_root_hash_from_description(&desc).is_some());
///
/// assert!(extract_vault_root_hash_from_description("zetl-snapshot").is_none());
/// ```
pub fn extract_vault_root_hash_from_description(description: &str) -> Option<String> {
    for part in description.split_whitespace() {
        if let Some(hash) = part.strip_prefix("vault_root_hash=") {
            if hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
                return Some(hash.to_owned());
            }
        }
    }
    None
}

// ─── Graph delta types ────────────────────────────────────────────────────────

/// Graph-level difference between two consecutive vault snapshots (REQ-080).
///
/// Computed by [`compute_graph_delta`] and embedded in each [`HistoryEntry`].
#[derive(Debug, Clone, Serialize)]
pub struct GraphDelta {
    /// Pages that exist in the newer snapshot but not in the older.
    pub pages_added: Vec<String>,
    /// Pages that existed in the older snapshot but are absent in the newer.
    pub pages_removed: Vec<String>,
    /// Net increase in total link count (`max(0, after − before)`).
    pub links_added: usize,
    /// Net decrease in total link count (`max(0, before − after)`).
    pub links_removed: usize,
}

/// A single entry in the reverse-chronological vault history timeline.
///
/// Returned by [`build_vault_history`].
#[derive(Debug, Clone, Serialize)]
pub struct HistoryEntry {
    /// jj change ID (12-char prefix).
    pub change_id: String,
    /// Snapshot timestamp as an RFC 3339 string.
    pub timestamp: String,
    /// `vault_root_hash` embedded in the snapshot description, if present.
    pub vault_root_hash: Option<String>,
    /// Number of pages in the snapshot (0 when no cached index is available).
    pub total_pages: usize,
    /// Total link count in the snapshot (0 when no cached index is available).
    pub total_links: usize,
    /// Graph-level delta vs. the next-older snapshot. `None` for the oldest
    /// entry or when the adjacent snapshot has no cached index.
    pub delta: Option<GraphDelta>,
}

// ─── Public API: graph delta & timeline ──────────────────────────────────────

/// Compute the graph-level diff between two vault index snapshots.
///
/// `before` is the older state; `after` is the newer state. Both are slices of
/// [`ParsedFile`] as loaded from the [`HistoricalIndexCache`].
///
/// This is a **pure function**: no I/O, no VCS calls.
///
/// [`HistoricalIndexCache`]: crate::history::cache::HistoricalIndexCache
pub fn compute_graph_delta(before: &[ParsedFile], after: &[ParsedFile]) -> GraphDelta {
    let before_pages: HashSet<&str> = before.iter().map(|f| f.page_name.as_str()).collect();
    let after_pages: HashSet<&str> = after.iter().map(|f| f.page_name.as_str()).collect();

    let mut pages_added: Vec<String> = after_pages
        .difference(&before_pages)
        .map(|s| s.to_string())
        .collect();
    pages_added.sort();

    let mut pages_removed: Vec<String> = before_pages
        .difference(&after_pages)
        .map(|s| s.to_string())
        .collect();
    pages_removed.sort();

    let before_links: usize = before.iter().map(|f| f.links.len()).sum();
    let after_links: usize = after.iter().map(|f| f.links.len()).sum();

    GraphDelta {
        pages_added,
        pages_removed,
        links_added: after_links.saturating_sub(before_links),
        links_removed: before_links.saturating_sub(after_links),
    }
}

/// Collapse consecutive [`HistoryEntry`] items that share the same
/// `vault_root_hash`, keeping only the newest of each duplicated run.
///
/// Entries with `vault_root_hash = None` are never collapsed.
/// Both the input and output are **newest-first**.
///
/// This is a **pure function**: no I/O, no VCS calls.
pub fn collapse_timeline(entries: Vec<HistoryEntry>) -> Vec<HistoryEntry> {
    let mut result: Vec<HistoryEntry> = Vec::with_capacity(entries.len());
    let mut seen: HashSet<String> = HashSet::new();
    for entry in entries {
        match &entry.vault_root_hash {
            Some(hash) => {
                if seen.insert(hash.clone()) {
                    result.push(entry);
                }
                // duplicate hash: skip
            }
            None => result.push(entry),
        }
    }
    result
}

/// Build the vault history timeline.
///
/// Walks `snapshots` (newest-first), applies the optional `since_expr` filter,
/// loads cached indexes from disk, collapses identical `vault_root_hash`
/// entries, computes graph-level deltas between consecutive snapshots, and
/// returns up to `limit` entries.
///
/// Snapshots whose description carries no `vault_root_hash` (or whose cached
/// index cannot be loaded) are still included in the timeline with
/// `total_pages = 0`, `total_links = 0`, and `delta = None`.
pub fn build_vault_history(
    snapshots: &[ChangeInfo],
    vault_root: &std::path::Path,
    since_expr: Option<&str>,
    limit: usize,
    now: DateTime<FixedOffset>,
) -> anyhow::Result<Vec<HistoryEntry>> {
    use crate::history::cache::HistoricalIndexCache;
    use std::collections::HashMap;
    use std::path::PathBuf;

    // Filter by --since if provided.
    let filtered: &[ChangeInfo] = if let Some(expr) = since_expr {
        let te = parse_time_expr(expr, now)?;
        match te {
            // Keep snapshots whose timestamp is at or after the cutoff.
            // Snapshots are newest-first so the in-range entries form a prefix.
            TimeExpr::Absolute(cutoff) => {
                let end = snapshots.partition_point(|s| s.timestamp >= cutoff);
                &snapshots[..end]
            }
            // HEAD~n → keep the n+1 most recent snapshots.
            TimeExpr::HeadOffset(n) => &snapshots[..snapshots.len().min(n + 1)],
            // Ref → find the matching snapshot; keep everything newer (inclusive).
            TimeExpr::Ref(ref ref_str) => {
                match snapshots.iter().position(|s| {
                    s.change_id.starts_with(ref_str.as_str()) || s.description == *ref_str
                }) {
                    Some(idx) => &snapshots[..=idx],
                    None => bail!("SNAPSHOT_NOT_FOUND: no snapshot matching ref {ref_str:?}"),
                }
            }
        }
    } else {
        snapshots
    };

    let cache = HistoricalIndexCache::with_default_capacity();

    // Load the index for each snapshot (if available).
    // files_per_snapshot[i] corresponds to filtered[i].
    let files_per_snapshot: Vec<Option<Vec<ParsedFile>>> = filtered
        .iter()
        .map(|snap| {
            let hash = extract_vault_root_hash_from_description(&snap.description)?;
            let file_map: HashMap<PathBuf, ParsedFile> = cache
                .load(vault_root, &hash)
                .ok()
                .flatten()?;
            Some(file_map.into_values().collect())
        })
        .collect();

    // Build HistoryEntry list (newest-first, without deltas yet).
    let mut entries: Vec<HistoryEntry> = filtered
        .iter()
        .zip(&files_per_snapshot)
        .map(|(snap, files_opt)| {
            let (total_pages, total_links) = match files_opt {
                Some(files) => (files.len(), files.iter().map(|f| f.links.len()).sum()),
                None => (0, 0),
            };
            HistoryEntry {
                change_id: snap.change_id.clone(),
                timestamp: snap.timestamp.to_rfc3339(),
                vault_root_hash: extract_vault_root_hash_from_description(&snap.description),
                total_pages,
                total_links,
                delta: None,
            }
        })
        .collect();

    // Collapse identical vault_root_hash entries.
    entries = collapse_timeline(entries);

    // Recompute files_per_snapshot to match the collapsed list.
    // We need the files for delta computation; rebuild by matching change_id.
    let change_id_to_files: HashMap<&str, &Option<Vec<ParsedFile>>> = filtered
        .iter()
        .zip(&files_per_snapshot)
        .map(|(s, f)| (s.change_id.as_str(), f))
        .collect();

    // Assign deltas: entry[i].delta = diff(entry[i+1], entry[i]).
    for i in 0..entries.len() {
        if i + 1 >= entries.len() {
            break; // oldest entry: no previous to diff against
        }
        let newer_files = change_id_to_files
            .get(entries[i].change_id.as_str())
            .and_then(|o| o.as_ref());
        let older_files = change_id_to_files
            .get(entries[i + 1].change_id.as_str())
            .and_then(|o| o.as_ref());
        if let (Some(newer), Some(older)) = (newer_files, older_files) {
            entries[i].delta = Some(compute_graph_delta(older, newer));
        }
    }

    // Apply limit.
    entries.truncate(limit);

    Ok(entries)
}

// ─── Vault history template context (REQ-085, CON-026, ADR-049) ─────────────

/// A single trend point for the `vault.history.trend` array.
#[derive(Debug, Clone, Serialize)]
pub struct TrendPoint {
    /// RFC 3339 timestamp of the sampled snapshot.
    pub timestamp: String,
    /// Number of pages in the vault at this point in time.
    pub total_pages: usize,
    /// Total link count in the vault at this point in time.
    pub total_links: usize,
}

/// The `vault.history` object injected into Minijinja template context (REQ-085, CON-026, ADR-049).
///
/// Summarises the vault's snapshot history for use in templates.
/// Available as `vault.history` in all templates; `null` when no history exists.
#[derive(Debug, Clone, Serialize)]
pub struct VaultHistoryContext {
    /// Up to 30 uniformly sampled trend points, oldest-first.
    pub trend: Vec<TrendPoint>,
    /// Up to 10 most recent history entries with full delta info.
    pub recent_changes: Vec<HistoryEntry>,
    /// RFC 3339 timestamp of the oldest snapshot, if any.
    pub oldest: Option<String>,
    /// RFC 3339 timestamp of the newest snapshot, if any.
    pub newest: Option<String>,
    /// RFC 3339 timestamp of the first snapshot ever made (epoch), if any.
    pub epoch: Option<String>,
    /// Total number of raw snapshots (before deduplication).
    pub snapshot_count: usize,
    /// Number of unique vault states (distinct `vault_root_hash` values after collapsing).
    pub unique_states: usize,
}

/// Sample up to `max_points` uniformly spaced entries from a newest-first history list.
///
/// Returns trend points in **oldest-first** order suitable for chart rendering.
/// Always includes both the newest and oldest entries when `max_points >= 2`.
///
/// This is a **pure function**: no I/O, no VCS calls.
pub fn sample_trend(entries: &[HistoryEntry], max_points: usize) -> Vec<TrendPoint> {
    if max_points == 0 || entries.is_empty() {
        return Vec::new();
    }

    // Convert to oldest-first view.
    let reversed: Vec<&HistoryEntry> = entries.iter().rev().collect();
    let n = reversed.len();

    let selected: Vec<&HistoryEntry> = if n <= max_points {
        reversed.iter().copied().collect()
    } else {
        // Uniformly sample max_points indices spanning the full range.
        (0..max_points)
            .map(|i| {
                let idx = if max_points == 1 {
                    0
                } else {
                    i * (n - 1) / (max_points - 1)
                };
                reversed[idx]
            })
            .collect()
    };

    selected
        .into_iter()
        .map(|e| TrendPoint {
            timestamp: e.timestamp.clone(),
            total_pages: e.total_pages,
            total_links: e.total_links,
        })
        .collect()
}

/// Build the `vault.history` template context object (REQ-085, CON-026, ADR-049).
///
/// Calls [`build_vault_history`] with no filter to get the full collapsed
/// timeline, then derives summary fields. Returns `None` when `snapshots` is
/// empty or the resulting timeline is empty.
///
/// This is a **pure function** aside from the cache reads performed by
/// [`build_vault_history`] internally.
pub fn build_vault_history_context(
    snapshots: &[ChangeInfo],
    vault_root: &std::path::Path,
    now: DateTime<FixedOffset>,
) -> anyhow::Result<Option<VaultHistoryContext>> {
    if snapshots.is_empty() {
        return Ok(None);
    }

    let snapshot_count = snapshots.len();

    // Build the full collapsed timeline (no since filter, no limit).
    let all_entries = build_vault_history(snapshots, vault_root, None, usize::MAX, now)?;

    if all_entries.is_empty() {
        return Ok(None);
    }

    // Count unique states: entries that carry a vault_root_hash (after collapse each hash appears once).
    let unique_states = all_entries
        .iter()
        .filter(|e| e.vault_root_hash.is_some())
        .count();

    let newest = all_entries.first().map(|e| e.timestamp.clone());
    let oldest = all_entries.last().map(|e| e.timestamp.clone());
    let epoch = oldest.clone();

    let trend = sample_trend(&all_entries, 30);
    let recent_changes = all_entries.into_iter().take(10).collect();

    Ok(Some(VaultHistoryContext {
        trend,
        recent_changes,
        oldest,
        newest,
        epoch,
        snapshot_count,
        unique_states,
    }))
}

// ─── Per-page history ─────────────────────────────────────────────────────────

/// Neighbourhood delta between two consecutive snapshots for a single page (REQ-081).
///
/// Embedded in [`PageHistoryEntry`] and produced by [`extract_page_history`].
#[derive(Debug, Clone, Serialize)]
pub struct PageNeighborhoodDelta {
    /// The page appeared in this snapshot (was absent in the previous).
    pub appeared: bool,
    /// The page disappeared in this snapshot (was present in the previous).
    pub disappeared: bool,
    /// Forward-link targets added since the previous snapshot (sorted).
    pub links_added: Vec<String>,
    /// Forward-link targets removed since the previous snapshot (sorted).
    pub links_removed: Vec<String>,
    /// Pages that started linking to this page (sorted).
    pub backlinks_added: Vec<String>,
    /// Pages that stopped linking to this page (sorted).
    pub backlinks_removed: Vec<String>,
}

/// A single entry in the per-page evolution timeline.
///
/// Returned by [`extract_page_history`].
#[derive(Debug, Clone, Serialize)]
pub struct PageHistoryEntry {
    /// jj change ID (12-char prefix).
    pub change_id: String,
    /// Snapshot timestamp as an RFC 3339 string.
    pub timestamp: String,
    /// Number of forward links from the page in this snapshot.
    pub link_count: usize,
    /// Number of pages that link to this page in this snapshot.
    pub backlink_count: usize,
    /// True when the page has neither forward links nor backlinks.
    pub is_orphan: bool,
    /// Neighbourhood delta vs. the previous snapshot that had cached data.
    pub delta: Option<PageNeighborhoodDelta>,
}

/// Extract the per-page evolution timeline for `page_name` (REQ-081, CON-025).
///
/// Walks `snapshots` (newest-first) paired with `files_per_snapshot` (same
/// order), selects only those snapshots where the page's *neighbourhood*
/// changed (forward links, backlinks, or existence), and returns up to `limit`
/// entries in newest-first order.
///
/// Each entry's `delta` describes the change relative to the immediately
/// preceding (older) snapshot that had cached data.
///
/// Snapshots whose cached index cannot be loaded (`files_per_snapshot[i] =
/// None`) are skipped entirely.
///
/// This is a **pure function**: no I/O, no VCS calls.
pub fn extract_page_history(
    page_name: &str,
    snapshots: &[ChangeInfo],
    files_per_snapshot: &[Option<Vec<ParsedFile>>],
    limit: usize,
) -> Vec<PageHistoryEntry> {
    use std::collections::BTreeSet;

    let page_lc = page_name.to_lowercase();
    let n = snapshots.len().min(files_per_snapshot.len());

    // Walk oldest-to-newest (snapshots are newest-first, so reverse-iterate).
    // Track the last known neighbourhood; include a snapshot only when the
    // neighbourhood changed.
    //
    // prev_state: (exists, forward_links, backlinks) for the most recently
    // processed snapshot that had cached data.
    let mut prev_state: Option<(bool, BTreeSet<String>, BTreeSet<String>)> = None;
    let mut included: Vec<PageHistoryEntry> = Vec::new();

    for raw_idx in (0..n).rev() {
        let files = match &files_per_snapshot[raw_idx] {
            Some(f) => f,
            None => continue,
        };

        // Compute this snapshot's neighbourhood.
        let this_page = files.iter().find(|f| f.page_name.to_lowercase() == page_lc);
        let exists = this_page.is_some();

        let forward: BTreeSet<String> = this_page
            .map(|f| f.links.iter().map(|l| l.target_page.to_lowercase()).collect())
            .unwrap_or_default();

        let backlinks: BTreeSet<String> = files
            .iter()
            .filter(|f| f.page_name.to_lowercase() != page_lc)
            .filter(|f| f.links.iter().any(|l| l.target_page.to_lowercase() == page_lc))
            .map(|f| f.page_name.to_lowercase())
            .collect();

        // Detect neighbourhood change vs. previous snapshot.
        let changed = match &prev_state {
            None => exists, // first data point: include only when page exists
            Some((pe, pf, pb)) => exists != *pe || &forward != pf || &backlinks != pb,
        };

        if changed {
            let delta = match &prev_state {
                None => PageNeighborhoodDelta {
                    appeared: true,
                    disappeared: false,
                    links_added: pgh_sorted_vec(&forward),
                    links_removed: vec![],
                    backlinks_added: pgh_sorted_vec(&backlinks),
                    backlinks_removed: vec![],
                },
                Some((pe, pf, pb)) => PageNeighborhoodDelta {
                    appeared: !pe && exists,
                    disappeared: *pe && !exists,
                    links_added: pgh_sorted_diff(&forward, pf),
                    links_removed: pgh_sorted_diff(pf, &forward),
                    backlinks_added: pgh_sorted_diff(&backlinks, pb),
                    backlinks_removed: pgh_sorted_diff(pb, &backlinks),
                },
            };

            let snap = &snapshots[raw_idx];
            included.push(PageHistoryEntry {
                change_id: snap.change_id.clone(),
                timestamp: snap.timestamp.to_rfc3339(),
                link_count: forward.len(),
                backlink_count: backlinks.len(),
                is_orphan: forward.is_empty() && backlinks.is_empty(),
                delta: Some(delta),
            });
        }

        prev_state = Some((exists, forward, backlinks));
    }

    // Convert oldest-to-newest result to newest-first, then apply limit.
    included.reverse();
    included.truncate(limit);
    included
}

fn pgh_sorted_vec(set: &std::collections::BTreeSet<String>) -> Vec<String> {
    set.iter().cloned().collect()
}

fn pgh_sorted_diff(
    a: &std::collections::BTreeSet<String>,
    b: &std::collections::BTreeSet<String>,
) -> Vec<String> {
    a.difference(b).cloned().collect()
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

/// Parse `HEAD` → `Some(0)` and `HEAD~N` → `Some(N)`.
fn parse_head_offset(s: &str) -> Option<usize> {
    if s.eq_ignore_ascii_case("head") {
        return Some(0);
    }
    let rest = s.strip_prefix("HEAD~").or_else(|| s.strip_prefix("head~"))?;
    rest.parse::<usize>().ok()
}

/// Parse relative natural-language time expressions.
///
/// Returns `None` when `s` is not a recognised relative form.
fn parse_relative(s: &str, now: DateTime<FixedOffset>) -> Option<DateTime<FixedOffset>> {
    let lower = s.to_lowercase();

    if lower == "now" {
        return Some(now);
    }

    if lower == "today" {
        return Some(start_of_day(now));
    }

    if lower == "yesterday" {
        return Some(start_of_day(now) - Duration::days(1));
    }

    // "last week" → start of Monday of the previous calendar week
    if lower == "last week" {
        let today = now.date_naive();
        let days_since_monday = today.weekday().num_days_from_monday() as i64;
        let last_monday = today - Duration::days(days_since_monday + 7);
        return last_monday
            .and_hms_opt(0, 0, 0)
            .and_then(|ndt| now.timezone().from_local_datetime(&ndt).single());
    }

    // "last <weekday>"
    if let Some(rest) = lower.strip_prefix("last ") {
        if let Some(wd) = parse_weekday(rest) {
            return Some(last_weekday(now, wd));
        }
    }

    // "N unit(s) ago"  — must have at least 3 whitespace-separated tokens
    let words: Vec<&str> = lower.split_whitespace().collect();
    if words.len() >= 3 && words[words.len() - 1] == "ago" {
        if let Ok(n) = words[0].parse::<i64>() {
            let unit = words[1].trim_end_matches('s');
            let delta = match unit {
                "second" => Duration::seconds(n),
                "minute" => Duration::minutes(n),
                "hour" => Duration::hours(n),
                "day" => Duration::days(n),
                "week" => Duration::weeks(n),
                "month" => Duration::days(n * 30),
                "year" => Duration::days(n * 365),
                _ => return None,
            };
            return Some(now - delta);
        }
    }

    None
}

/// Return midnight of the day containing `dt`, preserving the fixed offset.
fn start_of_day(dt: DateTime<FixedOffset>) -> DateTime<FixedOffset> {
    dt.date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|ndt| dt.timezone().from_local_datetime(&ndt).single())
        .unwrap_or(dt)
}

/// Return the most recent past occurrence of `weekday` before `now`
/// (never today itself: at least 1 day back, at most 7 days back).
fn last_weekday(now: DateTime<FixedOffset>, weekday: Weekday) -> DateTime<FixedOffset> {
    let today = now.date_naive();
    let target_num = weekday.num_days_from_monday() as i64;
    let today_num = today.weekday().num_days_from_monday() as i64;
    let mut days_back = today_num - target_num;
    if days_back <= 0 {
        days_back += 7;
    }
    let past = today - Duration::days(days_back);
    past.and_hms_opt(0, 0, 0)
        .and_then(|ndt| now.timezone().from_local_datetime(&ndt).single())
        .unwrap_or_else(|| now - Duration::days(days_back))
}

/// Parse a weekday name (long or three-letter abbreviation).
fn parse_weekday(s: &str) -> Option<Weekday> {
    match s {
        "monday" | "mon" => Some(Weekday::Mon),
        "tuesday" | "tue" => Some(Weekday::Tue),
        "wednesday" | "wed" => Some(Weekday::Wed),
        "thursday" | "thu" => Some(Weekday::Thu),
        "friday" | "fri" => Some(Weekday::Fri),
        "saturday" | "sat" => Some(Weekday::Sat),
        "sunday" | "sun" => Some(Weekday::Sun),
        _ => None,
    }
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod time_expression_parser {
    use chrono::{FixedOffset, TimeZone as _, Timelike as _};

    use super::*;

    /// A fixed "now" for deterministic tests: 2026-03-04 15:30:00 UTC.
    fn now_utc() -> DateTime<FixedOffset> {
        FixedOffset::east_opt(0)
            .unwrap()
            .with_ymd_and_hms(2026, 3, 4, 15, 30, 0)
            .unwrap()
    }

    fn make_snapshot(change_id: &str, description: &str, ts: DateTime<FixedOffset>) -> ChangeInfo {
        ChangeInfo {
            change_id: change_id.to_owned(),
            commit_id: "deadbeef0000".to_owned(),
            timestamp: ts,
            description: description.to_owned(),
        }
    }

    fn ts(y: i32, m: u32, d: u32, h: u32, min: u32, s: u32) -> DateTime<FixedOffset> {
        FixedOffset::east_opt(0)
            .unwrap()
            .with_ymd_and_hms(y, m, d, h, min, s)
            .unwrap()
    }

    // ── ISO 8601 ────────────────────────────────────────────────────────────

    #[test]
    fn iso_date_only() {
        // Date-only expressions resolve to end-of-day so all snapshots on
        // that day are included in a ≤ comparison.
        let expr = parse_time_expr("2024-01-15", now_utc()).unwrap();
        let TimeExpr::Absolute(dt) = expr else {
            panic!("expected Absolute, got {expr:?}");
        };
        assert_eq!(dt.year(), 2024);
        assert_eq!(dt.month(), 1);
        assert_eq!(dt.day(), 15);
        assert_eq!(dt.hour(), 23);
        assert_eq!(dt.minute(), 59);
    }

    #[test]
    fn iso_datetime_utc() {
        let expr = parse_time_expr("2024-06-01T12:00:00Z", now_utc()).unwrap();
        let TimeExpr::Absolute(dt) = expr else {
            panic!("expected Absolute, got {expr:?}");
        };
        assert_eq!(dt.year(), 2024);
        assert_eq!(dt.month(), 6);
        assert_eq!(dt.day(), 1);
        assert_eq!(dt.hour(), 12);
    }

    #[test]
    fn iso_datetime_with_offset() {
        let expr = parse_time_expr("2024-06-01T14:30:00+05:30", now_utc()).unwrap();
        let TimeExpr::Absolute(dt) = expr else {
            panic!("expected Absolute, got {expr:?}");
        };
        // Stored with the +05:30 offset, hour = 14.
        assert_eq!(dt.hour(), 14);
    }

    // ── Relative natural language ────────────────────────────────────────────

    #[test]
    fn relative_today() {
        let now = now_utc(); // 2026-03-04 15:30:00
        let expr = parse_time_expr("today", now).unwrap();
        let TimeExpr::Absolute(dt) = expr else {
            panic!("expected Absolute");
        };
        assert_eq!((dt.year(), dt.month(), dt.day()), (2026, 3, 4));
        assert_eq!(dt.hour(), 0);
    }

    #[test]
    fn relative_yesterday() {
        let now = now_utc(); // 2026-03-04
        let expr = parse_time_expr("yesterday", now).unwrap();
        let TimeExpr::Absolute(dt) = expr else {
            panic!("expected Absolute");
        };
        assert_eq!((dt.year(), dt.month(), dt.day()), (2026, 3, 3));
    }

    #[test]
    fn relative_n_days_ago() {
        let now = now_utc(); // 2026-03-04 15:30:00
        let expr = parse_time_expr("3 days ago", now).unwrap();
        let TimeExpr::Absolute(dt) = expr else {
            panic!("expected Absolute");
        };
        // 3 days before 2026-03-04 15:30:00 = 2026-03-01 15:30:00
        assert_eq!((dt.year(), dt.month(), dt.day()), (2026, 3, 1));
    }

    #[test]
    fn relative_2_weeks_ago() {
        let now = now_utc(); // 2026-03-04
        let expr = parse_time_expr("2 weeks ago", now).unwrap();
        let TimeExpr::Absolute(dt) = expr else {
            panic!("expected Absolute");
        };
        // 14 days before 2026-03-04 = 2026-02-18
        assert_eq!((dt.year(), dt.month(), dt.day()), (2026, 2, 18));
    }

    #[test]
    fn relative_1_hour_ago() {
        let now = now_utc(); // 15:30:00
        let expr = parse_time_expr("1 hour ago", now).unwrap();
        let TimeExpr::Absolute(dt) = expr else {
            panic!("expected Absolute");
        };
        assert_eq!(dt.hour(), 14);
        assert_eq!(dt.minute(), 30);
    }

    #[test]
    fn relative_last_monday() {
        // 2026-03-04 is a Wednesday.
        let now = now_utc();
        let expr = parse_time_expr("last monday", now).unwrap();
        let TimeExpr::Absolute(dt) = expr else {
            panic!("expected Absolute");
        };
        // Most recent Monday before Wednesday 2026-03-04 = 2026-03-02.
        assert_eq!((dt.year(), dt.month(), dt.day()), (2026, 3, 2));
    }

    #[test]
    fn relative_last_friday() {
        // 2026-03-04 is a Wednesday; last Friday = 2026-02-27.
        let now = now_utc();
        let expr = parse_time_expr("last friday", now).unwrap();
        let TimeExpr::Absolute(dt) = expr else {
            panic!("expected Absolute");
        };
        assert_eq!((dt.year(), dt.month(), dt.day()), (2026, 2, 27));
    }

    #[test]
    fn relative_last_week() {
        // 2026-03-04 (Wednesday): current week starts 2026-03-02 (Mon),
        // so last week starts 2026-02-23 (Mon).
        let now = now_utc();
        let expr = parse_time_expr("last week", now).unwrap();
        let TimeExpr::Absolute(dt) = expr else {
            panic!("expected Absolute");
        };
        assert_eq!((dt.year(), dt.month(), dt.day()), (2026, 2, 23));
    }

    // ── HEAD refs ────────────────────────────────────────────────────────────

    #[test]
    fn head_ref() {
        let expr = parse_time_expr("HEAD", now_utc()).unwrap();
        assert_eq!(expr, TimeExpr::HeadOffset(0));
    }

    #[test]
    fn head_tilde_zero() {
        let expr = parse_time_expr("HEAD~0", now_utc()).unwrap();
        assert_eq!(expr, TimeExpr::HeadOffset(0));
    }

    #[test]
    fn head_tilde_n() {
        let expr = parse_time_expr("HEAD~3", now_utc()).unwrap();
        assert_eq!(expr, TimeExpr::HeadOffset(3));
    }

    #[test]
    fn head_lowercase() {
        let expr = parse_time_expr("head~5", now_utc()).unwrap();
        assert_eq!(expr, TimeExpr::HeadOffset(5));
    }

    // ── VCS ref passthrough ──────────────────────────────────────────────────

    #[test]
    fn vcs_ref_change_id_prefix() {
        let expr = parse_time_expr("zkpqwxlmnopq", now_utc()).unwrap();
        assert_eq!(expr, TimeExpr::Ref("zkpqwxlmnopq".to_owned()));
    }

    #[test]
    fn vcs_ref_branch_name() {
        let expr = parse_time_expr("main", now_utc()).unwrap();
        assert_eq!(expr, TimeExpr::Ref("main".to_owned()));
    }

    // ── resolve_snapshot ────────────────────────────────────────────────────

    /// Build a small snapshot list (newest-first) for resolver tests.
    fn sample_snapshots() -> Vec<ChangeInfo> {
        vec![
            make_snapshot("aaaaaaaaaaaa", "snap-c", ts(2026, 3, 4, 12, 0, 0)),
            make_snapshot("bbbbbbbbbbbb", "snap-b", ts(2026, 3, 2, 8, 0, 0)),
            make_snapshot("cccccccccccc", "snap-a", ts(2026, 3, 1, 0, 0, 0)),
        ]
    }

    #[test]
    fn resolve_iso_date_exact_match() {
        let snaps = sample_snapshots();
        let result = resolve_snapshot("2026-03-04", now_utc(), &snaps).unwrap();
        assert_eq!(result.description, "snap-c");
    }

    #[test]
    fn resolve_iso_date_before_newest() {
        let snaps = sample_snapshots();
        // 2026-03-03 is after snap-b (03-02) but before snap-c (03-04 12:00).
        let result = resolve_snapshot("2026-03-03", now_utc(), &snaps).unwrap();
        assert_eq!(result.description, "snap-b");
    }

    #[test]
    fn resolve_iso_before_all_returns_error() {
        let snaps = sample_snapshots();
        let err = resolve_snapshot("2025-01-01", now_utc(), &snaps).unwrap_err();
        assert!(
            err.to_string().contains("SNAPSHOT_NOT_FOUND"),
            "expected SNAPSHOT_NOT_FOUND, got {err}"
        );
    }

    #[test]
    fn resolve_head_zero_is_newest() {
        let snaps = sample_snapshots();
        let result = resolve_snapshot("HEAD", now_utc(), &snaps).unwrap();
        assert_eq!(result.description, "snap-c");
    }

    #[test]
    fn resolve_head_tilde_1() {
        let snaps = sample_snapshots();
        let result = resolve_snapshot("HEAD~1", now_utc(), &snaps).unwrap();
        assert_eq!(result.description, "snap-b");
    }

    #[test]
    fn resolve_head_out_of_range() {
        let snaps = sample_snapshots();
        let err = resolve_snapshot("HEAD~10", now_utc(), &snaps).unwrap_err();
        assert!(
            err.to_string().contains("SNAPSHOT_NOT_FOUND"),
            "expected SNAPSHOT_NOT_FOUND, got {err}"
        );
    }

    #[test]
    fn resolve_empty_snapshots_errors() {
        let err = resolve_snapshot("yesterday", now_utc(), &[]).unwrap_err();
        assert!(
            err.to_string().contains("SNAPSHOT_NOT_FOUND"),
            "expected SNAPSHOT_NOT_FOUND, got {err}"
        );
    }

    #[test]
    fn resolve_ref_by_change_id_prefix() {
        let snaps = sample_snapshots();
        let result = resolve_snapshot("bbbb", now_utc(), &snaps).unwrap();
        assert_eq!(result.description, "snap-b");
    }

    #[test]
    fn resolve_ref_by_description() {
        let snaps = sample_snapshots();
        let result = resolve_snapshot("snap-a", now_utc(), &snaps).unwrap();
        assert_eq!(result.description, "snap-a");
    }

    #[test]
    fn resolve_ref_no_match_errors() {
        let snaps = sample_snapshots();
        let err = resolve_snapshot("unknown-ref", now_utc(), &snaps).unwrap_err();
        assert!(
            err.to_string().contains("SNAPSHOT_NOT_FOUND"),
            "expected SNAPSHOT_NOT_FOUND, got {err}"
        );
    }

    #[test]
    fn resolve_ref_by_commit_id_prefix() {
        // Build snapshots with distinct commit_ids (git SHAs via jj git backend).
        let mut snaps = sample_snapshots();
        snaps[0].commit_id = "cafe0000ffff".to_owned(); // newest: snap-c
        snaps[1].commit_id = "babe1111eeee".to_owned(); // snap-b
        snaps[2].commit_id = "dead2222dddd".to_owned(); // oldest: snap-a

        // Resolve by full git commit SHA prefix.
        let result = resolve_snapshot("babe", now_utc(), &snaps).unwrap();
        assert_eq!(result.description, "snap-b");

        // Resolve by shorter prefix (just 4 hex chars).
        let result2 = resolve_snapshot("dead", now_utc(), &snaps).unwrap();
        assert_eq!(result2.description, "snap-a");
    }

    #[test]
    fn error_on_empty_expr() {
        let err = parse_time_expr("", now_utc()).unwrap_err();
        assert!(err.to_string().contains("empty time expression"));
    }

    #[test]
    fn whitespace_trimmed() {
        let expr = parse_time_expr("  HEAD~2  ", now_utc()).unwrap();
        assert_eq!(expr, TimeExpr::HeadOffset(2));
    }
}

// ─── Unit tests: extract_page_history ────────────────────────────────────────

#[cfg(test)]
mod page_history_tests {
    use std::path::PathBuf;
    use std::time::SystemTime;

    use chrono::{FixedOffset, TimeZone as _};

    use super::*;
    use crate::types::{ParsedFile, WikiLink};

    fn make_snap(change_id: &str, ts_offset: i64) -> ChangeInfo {
        let base = FixedOffset::east_opt(0)
            .unwrap()
            .with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
            .unwrap();
        ChangeInfo {
            change_id: change_id.to_owned(),
            commit_id: format!("{change_id}commit"),
            timestamp: base + chrono::Duration::hours(ts_offset),
            description: format!("zetl-snapshot vault_root_hash={}", "a".repeat(64)),
        }
    }

    fn make_file(page_name: &str, links: &[&str]) -> ParsedFile {
        ParsedFile {
            path: PathBuf::from(format!("{page_name}.md")),
            page_name: page_name.to_owned(),
            links: links
                .iter()
                .map(|t| WikiLink {
                    target_page: t.to_string(),
                    raw_target: t.to_string(),
                    heading: None,
                    block_ref: None,
                    alias: None,
                    is_embed: false,
                    line: 1,
                    column: 1,
                })
                .collect(),
            spl_blocks: vec![],
            diagnostics: vec![],
            mtime: SystemTime::UNIX_EPOCH,
            merkle_leaves: vec![],
            file_merkle: None,
        }
    }

    // Snapshots are stored newest-first.
    // Timestamps: snap0=T+3h (newest), snap1=T+2h, snap2=T+1h, snap3=T+0h (oldest).

    /// Basic: links added then later removed — only two changed snapshots returned.
    #[test]
    fn basic_link_change_detected() {
        // snapshots newest-first
        let snaps = vec![
            make_snap("snap3", 3), // newest: target has 0 links
            make_snap("snap2", 2), // target has 1 link (A)
            make_snap("snap1", 1), // target has 0 links
            make_snap("snap0", 0), // oldest: target appeared
        ];
        // files newest-first (parallel to snaps)
        let files: Vec<Option<Vec<ParsedFile>>> = vec![
            // snap3: target exists, link removed back to 0
            Some(vec![make_file("target", &[]), make_file("other", &[])]),
            // snap2: target has link to alpha
            Some(vec![
                make_file("target", &["alpha"]),
                make_file("other", &[]),
            ]),
            // snap1: target exists, 0 links
            Some(vec![make_file("target", &[]), make_file("other", &[])]),
            // snap0: target first appeared, 0 links
            Some(vec![make_file("target", &[])]),
        ];

        let entries = extract_page_history("target", &snaps, &files, 20);

        // snap0: appeared (0 links)
        // snap1: no change (0 links) → skipped
        // snap2: link added (alpha) → included
        // snap3: link removed → included
        // Total: snap0(appeared), snap2(+alpha), snap3(-alpha)  → 3 entries newest-first
        assert_eq!(entries.len(), 3, "expected 3 entries; got {entries:#?}");

        // Newest first: snap3
        let e0 = &entries[0];
        assert_eq!(e0.change_id, "snap3");
        assert_eq!(e0.link_count, 0);
        let d0 = e0.delta.as_ref().unwrap();
        assert!(!d0.appeared);
        assert!(!d0.disappeared);
        assert!(d0.links_added.is_empty());
        assert_eq!(d0.links_removed, vec!["alpha"]);

        // snap2
        let e1 = &entries[1];
        assert_eq!(e1.change_id, "snap2");
        assert_eq!(e1.link_count, 1);
        let d1 = e1.delta.as_ref().unwrap();
        assert!(!d1.appeared);
        assert_eq!(d1.links_added, vec!["alpha"]);
        assert!(d1.links_removed.is_empty());

        // snap0 (oldest included): appeared
        let e2 = &entries[2];
        assert_eq!(e2.change_id, "snap0");
        let d2 = e2.delta.as_ref().unwrap();
        assert!(d2.appeared);
        assert!(d2.links_added.is_empty()); // appeared with 0 links
    }

    /// Page appearance and disappearance are both recorded.
    #[test]
    fn appearance_and_disappearance() {
        let snaps = vec![
            make_snap("snap2", 2), // newest: target gone
            make_snap("snap1", 1), // target exists
            make_snap("snap0", 0), // oldest: before page existed
        ];
        let files: Vec<Option<Vec<ParsedFile>>> = vec![
            Some(vec![make_file("other", &[])]),       // snap2: no target
            Some(vec![make_file("target", &["x"])]),   // snap1: target with link
            Some(vec![make_file("unrelated", &[])]),   // snap0: target absent
        ];

        let entries = extract_page_history("target", &snaps, &files, 20);

        // Expected: snap1 (appeared), snap2 (disappeared) → 2 entries newest-first
        assert_eq!(entries.len(), 2, "got {entries:#?}");

        let newest = &entries[0];
        assert_eq!(newest.change_id, "snap2");
        let d = newest.delta.as_ref().unwrap();
        assert!(d.disappeared);
        assert_eq!(d.links_removed, vec!["x"]);

        let older = &entries[1];
        assert_eq!(older.change_id, "snap1");
        let d2 = older.delta.as_ref().unwrap();
        assert!(d2.appeared);
        assert_eq!(d2.links_added, vec!["x"]);
    }

    /// Backlink changes are detected independently of forward-link changes.
    #[test]
    fn backlink_change_detected() {
        let snaps = vec![
            make_snap("snap1", 1), // other now links to target
            make_snap("snap0", 0), // target exists but no backlinks
        ];
        let files: Vec<Option<Vec<ParsedFile>>> = vec![
            // snap1: other links to target
            Some(vec![
                make_file("target", &[]),
                make_file("other", &["target"]),
            ]),
            // snap0: target alone, no backlinks
            Some(vec![make_file("target", &[])]),
        ];

        let entries = extract_page_history("target", &snaps, &files, 20);

        // snap0: appeared, snap1: backlink added → 2 entries
        assert_eq!(entries.len(), 2, "got {entries:#?}");

        let e0 = &entries[0]; // snap1
        assert_eq!(e0.change_id, "snap1");
        assert_eq!(e0.backlink_count, 1);
        let d = e0.delta.as_ref().unwrap();
        assert_eq!(d.backlinks_added, vec!["other"]);
        assert!(d.backlinks_removed.is_empty());

        let e1 = &entries[1]; // snap0
        let d1 = e1.delta.as_ref().unwrap();
        assert!(d1.appeared);
        assert!(d1.backlinks_added.is_empty());
    }

    /// The limit parameter truncates to the newest N entries.
    #[test]
    fn limit_applied_newest_first() {
        let snaps = vec![
            make_snap("snap3", 3),
            make_snap("snap2", 2),
            make_snap("snap1", 1),
            make_snap("snap0", 0),
        ];
        // Each snapshot changes the neighbourhood: target has 0, 1, 2, 3 links.
        let files: Vec<Option<Vec<ParsedFile>>> = vec![
            Some(vec![make_file("target", &["a", "b", "c"])]),
            Some(vec![make_file("target", &["a", "b"])]),
            Some(vec![make_file("target", &["a"])]),
            Some(vec![make_file("target", &[])]),
        ];

        let entries = extract_page_history("target", &snaps, &files, 2);

        // Without limit: 4 entries (appeared + 3 changes)
        // With limit=2: newest 2 → snap3 and snap2
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].change_id, "snap3");
        assert_eq!(entries[1].change_id, "snap2");
    }

    /// Snapshots with no cached data are silently skipped.
    #[test]
    fn skips_uncached_snapshots() {
        let snaps = vec![
            make_snap("snap1", 1),
            make_snap("snap0", 0),
        ];
        let files: Vec<Option<Vec<ParsedFile>>> = vec![
            Some(vec![make_file("target", &["x"])]),
            None, // no cached data
        ];

        // snap0 is skipped; snap1 is the first data → appeared
        let entries = extract_page_history("target", &snaps, &files, 20);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].change_id, "snap1");
        let d = entries[0].delta.as_ref().unwrap();
        assert!(d.appeared);
    }
}
