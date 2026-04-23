//! Shared build-scoped data channel for render-pipeline hooks
//! (SPEC-032 REQ-3219 / CON-3219).
//!
//! Hooks coordinate across stages (and across pages in release order)
//! through a build-scoped key/value store. A hook returning
//! `{"build_data": {"unresolved_links": [...]}}` writes into the next
//! hook's snapshot, namespaced under the emitting hook's
//! `extension_id`: `build_data[extension_id][key] = value`
//! (REQ-3219).
//!
//! ## Concurrency — sharding + finalise-stage merge
//!
//! Pages may be rendered concurrently during `ztl build`. To keep
//! hook observations race-free without serialising the whole build,
//! this module uses sharding:
//!
//! - [`BuildDataStore::begin_page`] captures the committed state as
//!   an immutable [`Arc`] snapshot and hands back a [`PageShard`].
//! - Hooks on that page read through a [`PageView`] that overlays
//!   the shard's in-page writes on that snapshot. Intra-page
//!   ordering is sequential (REQ-3206 / CON-3217) so every hook on a
//!   page sees the writes of every earlier hook on the same page.
//! - Inter-page visibility is **frozen at render-start** per
//!   CON-3219. Writes emitted by another page's in-flight pipeline
//!   are *not* visible mid-render.
//! - When the page is done, [`BuildDataStore::merge_shard`] folds the
//!   shard writes into the committed store. Cross-page write
//!   visibility is release-order-dependent — the last merger wins
//!   for a given `(extension_id, key)`.
//!
//! ## Size cap (16 MiB per build)
//!
//! [`BUILD_DATA_SIZE_CAP_BYTES`] bounds the sum of reserved bytes
//! across the committed store and every live shard. Reservations
//! are taken at [`PageShard::emit`] time so that concurrent pages
//! can't race past the cap. Writes that would exceed the cap are
//! dropped, recorded as warnings, and surfaced via
//! [`BuildDataStore::drain_warnings`]. Dropped writes do not affect
//! the AST/HTML payload.
//!
//! ## Serve mode
//!
//! `ztl serve` clears the store between page renders
//! ([`BuildDataStore::clear`]) so each render starts from an empty
//! channel (CON-3219).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;

/// Per-build byte cap for the shared `build_data` channel
/// (REQ-3219 / CON-3219): 16 MiB.
pub const BUILD_DATA_SIZE_CAP_BYTES: usize = 16 * 1024 * 1024;

/// Shape of the shared store: outer key is the writing hook's
/// `extension_id`, inner key is the caller-chosen key.
pub type BuildDataMap = BTreeMap<String, BTreeMap<String, Value>>;

/// A record of a write that was dropped because it would have
/// pushed the store past [`BUILD_DATA_SIZE_CAP_BYTES`].
///
/// Surfaced via [`BuildDataStore::drain_warnings`] so the build
/// orchestrator can emit a single warning per drop without
/// stopping the build (per REQ-3219: "oversize writes dropped
/// with a warning").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DroppedWrite {
    pub extension_id: String,
    pub key: String,
    pub size_bytes: usize,
    pub cap_bytes: usize,
}

impl std::fmt::Display for DroppedWrite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "build_data write dropped: {}.{} ({} bytes) would exceed {}-byte cap",
            self.extension_id, self.key, self.size_bytes, self.cap_bytes
        )
    }
}

/// Outcome of a single [`PageShard::emit`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmitOutcome {
    /// Write recorded in the shard; visible to later hooks on this
    /// page immediately and to other pages after merge.
    Accepted { size_bytes: usize },
    /// Write dropped because the size cap would be exceeded.
    Dropped(DroppedWrite),
}

impl EmitOutcome {
    pub fn is_accepted(&self) -> bool {
        matches!(self, EmitOutcome::Accepted { .. })
    }
}

/// Summary of a page shard's contribution after
/// [`BuildDataStore::merge_shard`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeReport {
    /// Number of `(extension_id, key)` entries folded into the
    /// committed store (including overwrites of prior values).
    pub merged_entries: usize,
    /// Total bytes reserved by the shard's accepted writes.
    pub merged_bytes: usize,
    /// Writes the shard had to drop before merge (surfaced here
    /// for per-page diagnostics; also accumulated in the store's
    /// warning log).
    pub dropped: Vec<DroppedWrite>,
}

/// Shared state guarded by [`BuildDataStore`]'s mutex.
///
/// `bytes_reserved` covers **both** the committed `data` map and
/// every live shard — a shard's accepted write consumes its slice
/// of the budget at emit time, not at merge time, so concurrent
/// pages cannot race past the cap.
struct StoreInner {
    data: BuildDataMap,
    bytes_reserved: usize,
    warnings: Vec<DroppedWrite>,
}

/// Build-scoped key/value channel (REQ-3219).
///
/// Cheap to [`Clone`]: every clone shares the same underlying
/// store via [`Arc`]. The build orchestrator keeps one instance
/// and hands a clone to each render worker through the hook
/// context.
#[derive(Clone)]
pub struct BuildDataStore {
    inner: Arc<Mutex<StoreInner>>,
    cap_bytes: usize,
}

impl Default for BuildDataStore {
    fn default() -> Self {
        Self::new()
    }
}

impl BuildDataStore {
    /// Create an empty store with the spec-defined cap
    /// ([`BUILD_DATA_SIZE_CAP_BYTES`], 16 MiB).
    pub fn new() -> Self {
        Self::with_cap(BUILD_DATA_SIZE_CAP_BYTES)
    }

    /// Create an empty store with an explicit byte cap.
    ///
    /// Useful for tests that exercise the drop-on-overflow path
    /// without constructing a 16 MiB payload.
    pub fn with_cap(cap_bytes: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(StoreInner {
                data: BuildDataMap::new(),
                bytes_reserved: 0,
                warnings: Vec::new(),
            })),
            cap_bytes,
        }
    }

    /// The per-build byte cap in effect for this store.
    pub fn cap_bytes(&self) -> usize {
        self.cap_bytes
    }

    /// Bytes currently reserved across the committed store and
    /// every live shard.
    pub fn bytes_reserved(&self) -> usize {
        self.inner.lock().unwrap().bytes_reserved
    }

    /// Open a page shard seeded with a snapshot of the committed
    /// state.
    ///
    /// Each page of the build should open exactly one shard;
    /// every hook on that page reads and writes through it. The
    /// shard is cheap — the snapshot is an [`Arc`] clone, not a
    /// deep copy — but [`Self::merge_shard`] must be called to
    /// publish the shard's writes to subsequent pages.
    pub fn begin_page(&self) -> PageShard {
        let guard = self.inner.lock().unwrap();
        PageShard {
            base: Arc::new(guard.data.clone()),
            writes: BuildDataMap::new(),
            bytes_reserved: 0,
            dropped: Vec::new(),
            store: Arc::clone(&self.inner),
            cap_bytes: self.cap_bytes,
        }
    }

    /// Merge a page shard's accepted writes into the committed
    /// store and release the shard's byte reservations for
    /// downstream accounting.
    ///
    /// Later-merged shards overwrite earlier ones for the same
    /// `(extension_id, key)` — cross-page visibility is
    /// release-order-dependent (CON-3219).
    pub fn merge_shard(&self, shard: PageShard) -> MergeReport {
        let PageShard {
            writes,
            bytes_reserved,
            dropped,
            ..
        } = shard;

        let mut guard = self.inner.lock().unwrap();
        let mut merged_entries = 0;
        for (ext_id, keys) in writes {
            let bucket = guard.data.entry(ext_id).or_default();
            for (key, value) in keys {
                bucket.insert(key, value);
                merged_entries += 1;
            }
        }
        // Bytes were already reserved at emit time on the
        // shared counter; nothing further to add here.
        let _ = bytes_reserved;

        MergeReport {
            merged_entries,
            merged_bytes: bytes_reserved,
            dropped,
        }
    }

    /// Discard a shard without merging. Releases the shard's
    /// byte reservations so a failed page render doesn't
    /// permanently shrink the budget.
    ///
    /// Used by `task-failure-scoping` when a page aborts mid-
    /// pipeline (REQ-3207) — the reverted fragment's in-flight
    /// writes should not count against later pages.
    pub fn discard_shard(&self, shard: PageShard) {
        let bytes = shard.bytes_reserved;
        let mut guard = self.inner.lock().unwrap();
        guard.bytes_reserved = guard.bytes_reserved.saturating_sub(bytes);
    }

    /// Snapshot the committed store as a JSON value suitable for
    /// direct embedding in a hook context payload or a template
    /// `vault.ext` view.
    pub fn snapshot_json(&self) -> Value {
        let guard = self.inner.lock().unwrap();
        build_data_to_json(&guard.data)
    }

    /// Clone the committed store as an owned map.
    pub fn snapshot_map(&self) -> BuildDataMap {
        self.inner.lock().unwrap().data.clone()
    }

    /// Drain accumulated drop warnings. Each call returns only the
    /// warnings added since the last drain.
    pub fn drain_warnings(&self) -> Vec<DroppedWrite> {
        std::mem::take(&mut self.inner.lock().unwrap().warnings)
    }

    /// Reset the store to empty — used between renders under
    /// `ztl serve` (CON-3219).
    ///
    /// Also clears the byte reservation counter. Any outstanding
    /// shards keep their snapshots (they cloned the map), but
    /// subsequent [`Self::begin_page`] calls will see an empty
    /// base.
    pub fn clear(&self) {
        let mut guard = self.inner.lock().unwrap();
        guard.data.clear();
        guard.bytes_reserved = 0;
        guard.warnings.clear();
    }
}

/// Per-page mutable state: immutable snapshot of the committed
/// store + the writes emitted by hooks on this page so far.
///
/// Hooks on a single page observe a [`PageView`] overlaying
/// `writes` on `base`; later hooks on the same page thus see
/// earlier hooks' writes (intra-page serial per CON-3219).
pub struct PageShard {
    base: Arc<BuildDataMap>,
    writes: BuildDataMap,
    bytes_reserved: usize,
    dropped: Vec<DroppedWrite>,
    store: Arc<Mutex<StoreInner>>,
    cap_bytes: usize,
}

impl PageShard {
    /// Read the current page view: base snapshot with any in-page
    /// writes overlaid.
    pub fn view(&self) -> PageView<'_> {
        PageView {
            base: &self.base,
            writes: &self.writes,
        }
    }

    /// Emit a write on behalf of a hook. The write is namespaced
    /// under `extension_id` (REQ-3219) and becomes visible to
    /// later hooks on this page immediately.
    ///
    /// If accepting the write would push the store's total byte
    /// reservation past [`BuildDataStore::cap_bytes`], the write
    /// is dropped and the drop is recorded both on the shard
    /// (via [`Self::dropped_writes`]) and on the parent store's
    /// warning log ([`BuildDataStore::drain_warnings`]).
    pub fn emit(
        &mut self,
        extension_id: impl Into<String>,
        key: impl Into<String>,
        value: Value,
    ) -> EmitOutcome {
        let extension_id = extension_id.into();
        let key = key.into();
        let size_bytes = estimate_value_size(&value);

        // Reserve bytes against the shared counter first so that
        // concurrent shards cannot collectively exceed the cap.
        let reserved = {
            let mut guard = self.store.lock().unwrap();
            let next = guard.bytes_reserved.saturating_add(size_bytes);
            if next > self.cap_bytes {
                let dropped = DroppedWrite {
                    extension_id: extension_id.clone(),
                    key: key.clone(),
                    size_bytes,
                    cap_bytes: self.cap_bytes,
                };
                guard.warnings.push(dropped.clone());
                self.dropped.push(dropped.clone());
                return EmitOutcome::Dropped(dropped);
            }
            guard.bytes_reserved = next;
            true
        };
        debug_assert!(reserved);

        self.bytes_reserved = self.bytes_reserved.saturating_add(size_bytes);
        self.writes
            .entry(extension_id)
            .or_default()
            .insert(key, value);
        EmitOutcome::Accepted { size_bytes }
    }

    /// Drops recorded on this shard so far.
    pub fn dropped_writes(&self) -> &[DroppedWrite] {
        &self.dropped
    }

    /// Bytes reserved by this shard's accepted writes.
    pub fn bytes_reserved(&self) -> usize {
        self.bytes_reserved
    }

    /// Number of accepted `(extension_id, key)` writes this
    /// shard holds.
    pub fn write_count(&self) -> usize {
        self.writes.values().map(|m| m.len()).sum()
    }
}

/// Read-only overlay of a [`PageShard`]'s in-page writes on its
/// base snapshot.
///
/// Construction is zero-allocation; [`Self::to_json`] materialises
/// the merged view when a hook context payload is needed.
pub struct PageView<'a> {
    base: &'a BuildDataMap,
    writes: &'a BuildDataMap,
}

impl PageView<'_> {
    /// Look up a value at `build_data[extension_id][key]`,
    /// preferring the in-page write over the snapshot.
    pub fn get(&self, extension_id: &str, key: &str) -> Option<&Value> {
        if let Some(v) = self.writes.get(extension_id).and_then(|m| m.get(key)) {
            return Some(v);
        }
        self.base.get(extension_id).and_then(|m| m.get(key))
    }

    /// Whether any value exists at `build_data[extension_id][key]`.
    pub fn contains(&self, extension_id: &str, key: &str) -> bool {
        self.get(extension_id, key).is_some()
    }

    /// Materialise the merged view as a JSON object suitable for
    /// direct use as the `build_data` field of a hook context
    /// payload.
    pub fn to_json(&self) -> Value {
        let mut merged: BuildDataMap = self.base.clone();
        for (ext_id, keys) in self.writes {
            let bucket = merged.entry(ext_id.clone()).or_default();
            for (key, value) in keys {
                bucket.insert(key.clone(), value.clone());
            }
        }
        build_data_to_json(&merged)
    }
}

/// Convert the internal representation to the nested JSON object
/// the SPEC-032 wire format expects:
/// `{ "<extension_id>": { "<key>": <value>, ... }, ... }`.
fn build_data_to_json(map: &BuildDataMap) -> Value {
    let mut out = serde_json::Map::with_capacity(map.len());
    for (ext_id, keys) in map {
        let mut inner = serde_json::Map::with_capacity(keys.len());
        for (key, value) in keys {
            inner.insert(key.clone(), value.clone());
        }
        out.insert(ext_id.clone(), Value::Object(inner));
    }
    Value::Object(out)
}

/// Rough byte estimate for a JSON value — the length of its
/// compact serialisation. Good enough for a size cap: it
/// includes structural overhead (braces, quotes, commas) that a
/// naive field-sum would miss, and costs O(n) in the payload
/// size, which we'd pay anyway if the write ends up on the hook
/// wire.
fn estimate_value_size(value: &Value) -> usize {
    serde_json::to_vec(value).map(|v| v.len()).unwrap_or(0)
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Barrier;
    use std::thread;

    // ── Basics ──────────────────────────────────────────────────────

    #[test]
    fn empty_store_has_empty_snapshot() {
        let store = BuildDataStore::new();
        assert_eq!(store.bytes_reserved(), 0);
        assert_eq!(store.snapshot_json(), json!({}));
        assert!(store.drain_warnings().is_empty());
    }

    #[test]
    fn cap_bytes_defaults_to_sixteen_mib() {
        let store = BuildDataStore::new();
        assert_eq!(store.cap_bytes(), 16 * 1024 * 1024);
    }

    #[test]
    fn emit_writes_visible_in_page_view() {
        let store = BuildDataStore::new();
        let mut shard = store.begin_page();

        let outcome = shard.emit("citations", "keys", json!([1, 2, 3]));
        assert!(outcome.is_accepted());

        let view = shard.view();
        assert_eq!(view.get("citations", "keys"), Some(&json!([1, 2, 3])));
        assert!(view.contains("citations", "keys"));
        assert_eq!(view.get("citations", "missing"), None);
    }

    #[test]
    fn emit_namespaces_writes_under_extension_id() {
        let store = BuildDataStore::new();
        let mut shard = store.begin_page();

        shard.emit("a", "k", json!(1));
        shard.emit("b", "k", json!(2));

        let view = shard.view();
        assert_eq!(view.get("a", "k"), Some(&json!(1)));
        assert_eq!(view.get("b", "k"), Some(&json!(2)));
        // Merged JSON keeps the namespace structure.
        assert_eq!(view.to_json(), json!({"a": {"k": 1}, "b": {"k": 2}}));
    }

    // ── Intra-page serial (CON-3219) ─────────────────────────────────

    #[test]
    fn later_hook_on_same_page_sees_earlier_writes() {
        let store = BuildDataStore::new();
        let mut shard = store.begin_page();

        // Simulate the first hook emitting a write.
        shard.emit("citations", "keys", json!(["a", "b"]));

        // A later hook on the same page reads the store and
        // extends it — it must see ["a","b"], not an empty view.
        let prior = shard
            .view()
            .get("citations", "keys")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert_eq!(prior, vec![json!("a"), json!("b")]);

        let mut extended = prior.clone();
        extended.push(json!("c"));
        shard.emit("citations", "keys", Value::Array(extended.clone()));

        // The overwrite is now visible to any further hook on
        // this page.
        assert_eq!(
            shard.view().get("citations", "keys"),
            Some(&Value::Array(extended))
        );
    }

    // ── Inter-page snapshot freeze (CON-3219) ────────────────────────

    #[test]
    fn inter_page_snapshot_frozen_at_begin_page() {
        let store = BuildDataStore::new();

        // Page A starts, emits, but has not merged yet.
        let mut page_a = store.begin_page();
        page_a.emit("citations", "keys", json!(["from-a"]));

        // Page B starts *before* A merges — it must see an empty
        // snapshot, not A's pending write.
        let page_b = store.begin_page();
        assert_eq!(page_b.view().get("citations", "keys"), None);

        // After A merges, a *new* page C does see the committed
        // write. Page B still sees its frozen (empty) snapshot.
        store.merge_shard(page_a);
        assert_eq!(page_b.view().get("citations", "keys"), None);

        let page_c = store.begin_page();
        assert_eq!(
            page_c.view().get("citations", "keys"),
            Some(&json!(["from-a"]))
        );
    }

    // ── Merge semantics ──────────────────────────────────────────────

    #[test]
    fn merge_publishes_shard_writes_to_store() {
        let store = BuildDataStore::new();
        let mut shard = store.begin_page();
        shard.emit("a", "k", json!(1));
        shard.emit("b", "j", json!([true, false]));

        let report = store.merge_shard(shard);
        assert_eq!(report.merged_entries, 2);
        assert!(report.merged_bytes > 0);
        assert!(report.dropped.is_empty());

        assert_eq!(
            store.snapshot_json(),
            json!({"a": {"k": 1}, "b": {"j": [true, false]}})
        );
    }

    #[test]
    fn later_merger_overwrites_earlier_for_same_key() {
        // Documents the release-order-dependent semantics of
        // CON-3219: "cross-page write visibility being
        // release-order-dependent."
        let store = BuildDataStore::new();
        let mut p1 = store.begin_page();
        p1.emit("shared", "k", json!("from-1"));
        let mut p2 = store.begin_page();
        p2.emit("shared", "k", json!("from-2"));

        store.merge_shard(p1);
        store.merge_shard(p2);

        assert_eq!(store.snapshot_json(), json!({"shared": {"k": "from-2"}}));
    }

    #[test]
    fn totals_accumulate_across_pages() {
        // Acceptance criterion: "Fixture: hook accumulates totals
        // across pages". Each page emits its own page count and
        // the final store carries every page's write.
        let store = BuildDataStore::new();
        for i in 0..5 {
            let mut shard = store.begin_page();
            shard.emit("pages", format!("page{i}"), json!({"word_count": i * 10}));
            store.merge_shard(shard);
        }

        let snap = store.snapshot_json();
        let pages = snap.get("pages").unwrap().as_object().unwrap();
        assert_eq!(pages.len(), 5);
        assert_eq!(pages.get("page3").unwrap(), &json!({"word_count": 30}));
    }

    // ── Size cap (REQ-3219) ──────────────────────────────────────────

    #[test]
    fn oversize_write_dropped_with_warning() {
        // Tiny cap so we can overflow deterministically.
        let store = BuildDataStore::with_cap(64);
        let mut shard = store.begin_page();

        // Fits.
        let fits = shard.emit("ext", "small", json!("a"));
        assert!(fits.is_accepted());

        // Obviously too large for the 64-byte cap.
        let huge = "x".repeat(200);
        let outcome = shard.emit("ext", "big", Value::String(huge));

        match outcome {
            EmitOutcome::Dropped(d) => {
                assert_eq!(d.extension_id, "ext");
                assert_eq!(d.key, "big");
                assert_eq!(d.cap_bytes, 64);
                assert!(d.size_bytes > 64);
            }
            other => panic!("expected Dropped, got {other:?}"),
        }

        // Dropped write is observable on the shard…
        assert_eq!(shard.dropped_writes().len(), 1);
        // …and on the store's warning log, drained exactly once.
        let warnings = store.drain_warnings();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].key, "big");
        assert!(store.drain_warnings().is_empty());

        // The small write is still there.
        assert_eq!(shard.view().get("ext", "small"), Some(&json!("a")));
    }

    #[test]
    fn size_cap_is_build_wide_across_shards() {
        // Two pages, each trying to consume most of a small cap.
        // The second page's large write should be dropped even
        // though the first page hasn't merged yet — the cap is
        // enforced on reservation (CON-3219: "enforced on each
        // write").
        let store = BuildDataStore::with_cap(64);
        let mut p1 = store.begin_page();
        let mut p2 = store.begin_page();

        let payload = Value::String("x".repeat(40));
        assert!(p1.emit("p1", "k", payload.clone()).is_accepted());
        // 42+ bytes reserved; a second 42-byte write must fail.
        let outcome = p2.emit("p2", "k", payload);
        assert!(matches!(outcome, EmitOutcome::Dropped(_)));
    }

    #[test]
    fn discard_shard_releases_reserved_bytes() {
        let store = BuildDataStore::with_cap(64);
        let mut shard = store.begin_page();
        shard.emit("ext", "k", Value::String("x".repeat(40)));
        let before = store.bytes_reserved();
        assert!(before >= 40);

        store.discard_shard(shard);
        assert_eq!(store.bytes_reserved(), 0);

        // A fresh shard can now use the budget again.
        let mut fresh = store.begin_page();
        assert!(fresh
            .emit("ext", "k", Value::String("x".repeat(40)))
            .is_accepted());
    }

    // ── Serve mode ────────────────────────────────────────────────────

    #[test]
    fn clear_resets_store_for_serve_mode() {
        let store = BuildDataStore::new();
        let mut shard = store.begin_page();
        shard.emit("ext", "k", json!(1));
        store.merge_shard(shard);
        assert_ne!(store.snapshot_json(), json!({}));

        store.clear();
        assert_eq!(store.snapshot_json(), json!({}));
        assert_eq!(store.bytes_reserved(), 0);
        assert!(store.drain_warnings().is_empty());

        // Subsequent page shards start from the empty base.
        let fresh = store.begin_page();
        assert_eq!(fresh.view().get("ext", "k"), None);
    }

    // ── Finalise merged view ─────────────────────────────────────────

    #[test]
    fn finalise_view_contains_every_page_merge() {
        // Acceptance criterion: "finalise stage sees merged view."
        // After every page merges, the store's snapshot carries
        // every page's writes, even with overlapping keys.
        let store = BuildDataStore::new();

        let mut p1 = store.begin_page();
        p1.emit("links", "a.md", json!(["b.md"]));
        store.merge_shard(p1);

        let mut p2 = store.begin_page();
        p2.emit("links", "b.md", json!(["c.md"]));
        // p2 reads the snapshot: a.md is present.
        assert!(p2.view().contains("links", "a.md"));
        store.merge_shard(p2);

        let snap = store.snapshot_json();
        let links = snap.get("links").unwrap().as_object().unwrap();
        assert_eq!(links.len(), 2);
        assert_eq!(links.get("a.md").unwrap(), &json!(["b.md"]));
        assert_eq!(links.get("b.md").unwrap(), &json!(["c.md"]));
    }

    // ── Cross-page concurrency (CON-3219) ────────────────────────────

    #[test]
    fn concurrent_pages_merge_without_data_loss() {
        // Acceptance criterion: "cross-page concurrency test (race
        // detector) passes". Every page's write must end up in
        // the final store; each page's mid-render read sees only
        // its own frozen snapshot.
        let store = BuildDataStore::new();
        let workers = 16;
        let barrier = Arc::new(Barrier::new(workers));
        let page_count = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for i in 0..workers {
            let store = store.clone();
            let barrier = Arc::clone(&barrier);
            let page_count = Arc::clone(&page_count);
            handles.push(thread::spawn(move || {
                let mut shard = store.begin_page();
                // Force all threads past begin_page() before any
                // merges — exercises the inter-page snapshot
                // freeze under real contention.
                barrier.wait();

                // Mid-render, this page cannot see any other
                // page's not-yet-merged write.
                assert!(
                    shard.view().to_json().as_object().unwrap().is_empty(),
                    "page {i} saw another page's pre-merge write"
                );

                shard.emit("pages", format!("p{i}"), json!({"id": i}));
                let report = store.merge_shard(shard);
                assert_eq!(report.merged_entries, 1);
                page_count.fetch_add(1, Ordering::SeqCst);
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(page_count.load(Ordering::SeqCst), workers);

        // Every page's write survives the concurrent merges.
        let snap = store.snapshot_json();
        let pages = snap.get("pages").unwrap().as_object().unwrap();
        assert_eq!(pages.len(), workers);
        for i in 0..workers {
            let key = format!("p{i}");
            assert_eq!(pages.get(&key).unwrap(), &json!({"id": i}));
        }
    }

    #[test]
    fn concurrent_size_cap_never_exceeded() {
        // Many threads race to fill a small cap; the final
        // reservation must never sit above the cap.
        let cap = 256;
        let store = BuildDataStore::with_cap(cap);
        let workers = 32;
        let mut handles = Vec::new();
        for i in 0..workers {
            let store = store.clone();
            handles.push(thread::spawn(move || {
                let mut shard = store.begin_page();
                for _ in 0..8 {
                    let _ = shard.emit("ext", format!("k{i}"), Value::String("x".repeat(16)));
                }
                store.merge_shard(shard);
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert!(store.bytes_reserved() <= cap);
        // At least one write was dropped (workers * 8 writes of
        // ≥18 bytes each cannot all fit in 256 bytes).
        assert!(!store.drain_warnings().is_empty());
    }

    // ── JSON wire shape ──────────────────────────────────────────────

    #[test]
    fn to_json_matches_spec_wire_shape() {
        // SPEC-032 §REQ-3219: `build_data[extension_id][key] =
        // value`. The context payload passed to a hook must be a
        // nested object keyed first by extension_id, then by key.
        let store = BuildDataStore::new();
        let mut shard = store.begin_page();
        shard.emit("citations", "keys", json!(["k1"]));
        shard.emit("citations", "count", json!(1));
        shard.emit("tasks", "open", json!(3));

        let payload = shard.view().to_json();
        assert_eq!(
            payload,
            json!({
                "citations": {"count": 1, "keys": ["k1"]},
                "tasks": {"open": 3},
            })
        );
    }

    #[test]
    fn dropped_write_display_is_informative() {
        let d = DroppedWrite {
            extension_id: "ext".into(),
            key: "big".into(),
            size_bytes: 5000,
            cap_bytes: 1024,
        };
        let s = d.to_string();
        assert!(s.contains("ext.big"));
        assert!(s.contains("5000"));
        assert!(s.contains("1024"));
    }

    #[test]
    fn store_is_clone_shares_state() {
        // Clones share the same underlying channel — the
        // orchestrator hands a clone to each worker.
        let store = BuildDataStore::new();
        let clone = store.clone();
        let mut shard = clone.begin_page();
        shard.emit("ext", "k", json!("v"));
        clone.merge_shard(shard);

        assert_eq!(store.snapshot_json(), json!({"ext": {"k": "v"}}));
    }
}
