//! WebSocket `/ws/edit/{slug}` endpoint for collaborative editing (REQ-020-028).
//!
//! Protocol messages (JSON over WebSocket text frames):
//!
//! **Client → Server:**
//! - `{"type":"sync","doc":<base64>}` — full CRDT state push
//! - `{"type":"op","ops":[...]}` — incremental splice/mark operations
//! - `{"type":"presence","cursor":{"index":<u>,"head":<u>},"name":<s>}` — cursor position
//!
//! **Server → Client:**
//! - `{"type":"sync","doc":<base64>}` — full CRDT state (on join)
//! - `{"type":"op","ops":[...],"user_id":<s>}` — relayed operations from another user
//! - `{"type":"presence","user_id":<s>,"cursor":{"index":<u>,"head":<u>},"name":<s>}` — cursor broadcast
//! - `{"type":"error","message":<s>}` — protocol error
//!
//! **Auth:** session cookie (`zetl_session`) or one-time ticket (`?ticket=<token>`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Path as AxumPath, Query, State, WebSocketUpgrade};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::crdt::CrdtDocument;

use super::wal::WalStore;
use super::WebState;

/// CRDT document metadata returned by [`CrdtDocStore::crdt_meta`].
#[derive(Debug, Clone)]
pub struct CrdtMeta {
    /// Whether the CRDT state has diverged from the on-disk markdown.
    pub dirty: bool,
    /// Seconds elapsed since the last flush to disk.
    pub secs_since_flush: u64,
    /// Number of currently connected WebSocket clients.
    pub client_count: usize,
    /// BLAKE3 hex digest of the live CRDT markdown (for conflict detection).
    pub content_hash: Option<String>,
}

// ── Protocol messages ────────────────────────────────────────────────

/// Inbound message from a client.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    Sync {
        doc: String,
    },
    Op {
        ops: Vec<OpEntry>,
    },
    Presence {
        cursor: CursorPos,
        name: Option<String>,
    },
}

/// Outbound message to a client.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    Sync {
        doc: String,
    },
    Op {
        ops: Vec<OpEntry>,
        user_id: String,
    },
    Presence {
        user_id: String,
        cursor: CursorPos,
        name: Option<String>,
    },
    Error {
        message: String,
    },
    /// Admin notification when a user requests access to a page (REQ-020-047).
    AccessRequest {
        user: String,
        page: String,
    },
    /// Notification when an admin changes the vault's visibility mode (REQ-020-054).
    VisibilityModeChanged {
        old_mode: String,
        new_mode: String,
        changed_by: String,
    },
    /// A new comment was posted on a page (REQ-020-051).
    Comment {
        slug: String,
        user: String,
        text: String,
        at: String,
    },
    /// External edit detected — files changed outside of zetl's pipeline (REQ-020-039).
    ExternalEdit {
        files: Vec<String>,
    },
    /// ACL violation detected during post-reconciliation — admin banner (REQ-020-043).
    AclViolation {
        page: String,
        user_id: String,
        reason: String,
    },
    /// External edit merged with dirty CRDT — editors see merged state (REQ-020-040 Case 2).
    ExternalMerge {
        slug: String,
        doc: String,
    },
    /// File deleted externally — editors should close (REQ-020-040 Case 3).
    Deleted {
        page: String,
    },
}

/// A single edit operation (splice text or mark).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum OpEntry {
    Splice {
        pos: usize,
        del: usize,
        text: String,
    },
    Mark {
        name: String,
        value: serde_json::Value,
        start: usize,
        end: usize,
    },
    Unmark {
        name: String,
        start: usize,
        end: usize,
    },
}

/// Cursor position (selection anchor + head).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorPos {
    pub index: usize,
    pub head: usize,
}

// ── Per-slug editing room ────────────────────────────────────────────

/// Broadcast channel capacity per room.
const ROOM_CAPACITY: usize = 256;

/// A room for a single page slug — holds a broadcast channel for relaying
/// ops and presence, plus the latest CRDT doc bytes for late joiners.
pub struct EditRoom {
    pub tx: broadcast::Sender<ServerMsg>,
    /// Latest full CRDT doc state (base64-encoded automerge bytes).
    pub doc_state: Mutex<Option<String>>,
}

impl EditRoom {
    fn new() -> Self {
        let (tx, _) = broadcast::channel(ROOM_CAPACITY);
        Self {
            tx,
            doc_state: Mutex::new(None),
        }
    }
}

/// Hub managing all active editing rooms, keyed by page slug.
#[derive(Clone, Default)]
pub struct WsHub {
    rooms: Arc<Mutex<HashMap<String, Arc<EditRoom>>>>,
}

impl WsHub {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get or create a room for the given slug.
    pub fn room(&self, slug: &str) -> Arc<EditRoom> {
        let mut rooms = self.rooms.lock().expect("ws hub lock");
        rooms
            .entry(slug.to_string())
            .or_insert_with(|| Arc::new(EditRoom::new()))
            .clone()
    }

    /// Broadcast a message to all rooms (e.g. admin notifications).
    /// This sends to every active editing room so all connected users see it.
    pub fn broadcast_all(&self, msg: ServerMsg) {
        let rooms = self.rooms.lock().expect("ws hub lock");
        for room in rooms.values() {
            let _ = room.tx.send(msg.clone());
        }
    }

    /// Remove a room when the last subscriber leaves (optional cleanup).
    pub fn remove_if_empty(&self, slug: &str) {
        let mut rooms = self.rooms.lock().expect("ws hub lock");
        if let Some(room) = rooms.get(slug) {
            if room.tx.receiver_count() == 0 {
                rooms.remove(slug);
            }
        }
    }
}

// ── CRDT document store (REQ-020-029) ────────────────────────────────

/// Convert a `serde_json::Value` to an automerge `ScalarValue`.
fn json_to_scalar(v: &serde_json::Value) -> automerge::ScalarValue {
    match v {
        serde_json::Value::Bool(b) => automerge::ScalarValue::from(*b),
        serde_json::Value::String(s) => automerge::ScalarValue::from(s.clone()),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                automerge::ScalarValue::from(i)
            } else if let Some(f) = n.as_f64() {
                automerge::ScalarValue::from(f)
            } else {
                automerge::ScalarValue::from(true)
            }
        }
        _ => automerge::ScalarValue::from(true),
    }
}

/// Default eviction TTL: 10 minutes after last client disconnects.
const DEFAULT_EVICTION_TTL: Duration = Duration::from_secs(10 * 60);

/// Default quiescence delay: 5 seconds of no edits triggers a flush.
const DEFAULT_QUIESCENCE_DELAY: Duration = Duration::from_secs(5);

/// Default maximum number of concurrently loaded CRDT documents.
const DEFAULT_MAX_DOCS: usize = 50;

/// A loaded CRDT document with lifecycle metadata.
pub struct CrdtDocEntry {
    pub doc: CrdtDocument,
    /// Last time any edit was received.
    pub last_edit: Instant,
    /// Last time the document was flushed to disk.
    pub last_flush: Instant,
    /// Whether the CRDT state has diverged from the on-disk markdown.
    pub dirty: bool,
    /// Number of currently connected WebSocket clients.
    pub client_count: usize,
    /// When the last client disconnected (None if clients are still connected).
    pub disconnected_at: Option<Instant>,
    /// Last time this document was accessed (for LRU eviction).
    pub last_access: Instant,
}

impl CrdtDocEntry {
    fn new(doc: CrdtDocument) -> Self {
        let now = Instant::now();
        Self {
            doc,
            last_edit: now,
            last_flush: now,
            dirty: false,
            client_count: 0,
            disconnected_at: None,
            last_access: now,
        }
    }

    /// Mark that an edit was received.
    pub fn touch_edit(&mut self) {
        let now = Instant::now();
        self.last_edit = now;
        self.last_access = now;
        self.dirty = true;
    }

    /// Mark that the document was flushed to disk.
    pub fn mark_flushed(&mut self) {
        self.last_flush = Instant::now();
        self.dirty = false;
    }
}

/// Manages CRDT document lifecycle: load, eviction, memory bounds (REQ-020-029).
///
/// Documents are loaded from disk on first WebSocket connection and held in
/// memory. After all clients disconnect, the document remains cached for
/// `eviction_ttl` before being flushed (if dirty) and evicted. A memory
/// bound (`max_docs`) enforces LRU eviction when capacity is reached.
#[derive(Clone)]
pub struct CrdtDocStore {
    docs: Arc<Mutex<HashMap<String, CrdtDocEntry>>>,
    vault_root: Arc<PathBuf>,
    pub max_docs: usize,
    pub eviction_ttl: Duration,
    pub quiescence_delay: Duration,
}

impl CrdtDocStore {
    pub fn new(vault_root: Arc<PathBuf>) -> Self {
        Self {
            docs: Arc::new(Mutex::new(HashMap::new())),
            vault_root,
            max_docs: DEFAULT_MAX_DOCS,
            eviction_ttl: DEFAULT_EVICTION_TTL,
            quiescence_delay: DEFAULT_QUIESCENCE_DELAY,
        }
    }

    /// Get or load a CRDT document for the given slug.
    ///
    /// If the document is already in memory, returns the existing entry.
    /// Otherwise, reads the `.md` file from disk and parses it into a
    /// `CrdtDocument`. If the file doesn't exist, creates an empty document.
    ///
    /// Enforces the memory bound by evicting the least-recently-used
    /// document (that has no active clients) when at capacity.
    pub fn load_or_get(
        &self,
        slug: &str,
    ) -> Result<(), anyhow::Error> {
        let mut docs = self.docs.lock().expect("crdt store lock");

        if let Some(entry) = docs.get_mut(slug) {
            entry.last_access = Instant::now();
            return Ok(());
        }

        // Enforce memory bound before inserting
        if docs.len() >= self.max_docs {
            self.evict_lru(&mut docs)?;
        }

        // Load from disk
        let doc = self.load_from_disk(slug)?;
        docs.insert(slug.to_string(), CrdtDocEntry::new(doc));
        Ok(())
    }

    /// Read a page's markdown file and parse it into a `CrdtDocument`.
    fn load_from_disk(&self, slug: &str) -> Result<CrdtDocument, anyhow::Error> {
        let md_path = self.md_path_for_slug(slug);
        if md_path.exists() {
            let markdown = std::fs::read_to_string(&md_path)
                .map_err(|e| anyhow::anyhow!("read {}: {e}", md_path.display()))?;
            CrdtDocument::from_markdown(&markdown)
        } else {
            CrdtDocument::new()
        }
    }

    /// Resolve a slug to its `.md` file path.
    pub fn md_path_for_slug(&self, slug: &str) -> PathBuf {
        self.vault_root.join(format!("{slug}.md"))
    }

    /// Increment client count for a document. Must be loaded first.
    pub fn client_connected(&self, slug: &str) {
        let mut docs = self.docs.lock().expect("crdt store lock");
        if let Some(entry) = docs.get_mut(slug) {
            entry.client_count += 1;
            entry.disconnected_at = None;
            entry.last_access = Instant::now();
        }
    }

    /// Decrement client count. When it reaches 0, records the disconnect time
    /// for eviction TTL tracking.
    pub fn client_disconnected(&self, slug: &str) {
        let mut docs = self.docs.lock().expect("crdt store lock");
        if let Some(entry) = docs.get_mut(slug) {
            entry.client_count = entry.client_count.saturating_sub(1);
            if entry.client_count == 0 {
                entry.disconnected_at = Some(Instant::now());
            }
        }
    }

    /// Record an edit on the document (marks dirty, updates timestamps).
    pub fn record_edit(&self, slug: &str) {
        let mut docs = self.docs.lock().expect("crdt store lock");
        if let Some(entry) = docs.get_mut(slug) {
            entry.touch_edit();
        }
    }

    /// Get the serialized CRDT state (automerge bytes, base64-encoded) for a slug.
    pub fn get_doc_state_b64(&self, slug: &str) -> Option<String> {
        let mut docs = self.docs.lock().expect("crdt store lock");
        if let Some(entry) = docs.get_mut(slug) {
            entry.last_access = Instant::now();
            let bytes = entry.doc.save();
            Some(base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &bytes,
            ))
        } else {
            None
        }
    }

    /// Apply a sync message (full CRDT state from a client) to the stored doc.
    pub fn apply_sync(&self, slug: &str, doc_b64: &str) -> Result<(), anyhow::Error> {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(doc_b64)
            .map_err(|e| anyhow::anyhow!("invalid base64: {e}"))?;
        let incoming = CrdtDocument::load(&bytes)?;

        let mut docs = self.docs.lock().expect("crdt store lock");
        if let Some(entry) = docs.get_mut(slug) {
            // Replace with the incoming state (client push)
            entry.doc = incoming;
            entry.touch_edit();
        }
        Ok(())
    }

    /// Apply incremental operations to the stored CRDT document.
    pub fn apply_ops(&self, slug: &str, ops: &[OpEntry]) -> Result<(), anyhow::Error> {
        let mut docs = self.docs.lock().expect("crdt store lock");
        if let Some(entry) = docs.get_mut(slug) {
            for op in ops {
                match op {
                    OpEntry::Splice { pos, del, text } => {
                        entry.doc.splice_text(*pos, *del as isize, text)?;
                    }
                    OpEntry::Mark {
                        name,
                        value,
                        start,
                        end,
                    } => {
                        let sv = json_to_scalar(value);
                        if let Some(mt) =
                            crate::crdt::marks::MarkType::from_mark(name, &sv)
                        {
                            entry.doc.mark(&mt, *start, *end)?;
                        }
                    }
                    OpEntry::Unmark { name, start, end } => {
                        if let Some(mt) = crate::crdt::marks::MarkType::from_name(name) {
                            entry.doc.unmark(&mt, *start, *end)?;
                        }
                    }
                }
            }
            entry.touch_edit();
        }
        Ok(())
    }

    /// Collect slugs that need quiescence flush (dirty + idle for >= quiescence_delay).
    pub fn slugs_needing_flush(&self) -> Vec<String> {
        let docs = self.docs.lock().expect("crdt store lock");
        let now = Instant::now();
        docs.iter()
            .filter(|(_, entry)| {
                entry.dirty && now.duration_since(entry.last_edit) >= self.quiescence_delay
            })
            .map(|(slug, _)| slug.clone())
            .collect()
    }

    /// Serialize a document to markdown for flushing. Returns None if not loaded or not dirty.
    pub fn serialize_for_flush(&self, slug: &str) -> Option<String> {
        let mut docs = self.docs.lock().expect("crdt store lock");
        let entry = docs.get_mut(slug)?;
        if !entry.dirty {
            return None;
        }
        entry.doc.to_markdown().ok()
    }

    /// Mark a document as flushed after successfully writing to disk.
    pub fn mark_flushed(&self, slug: &str) {
        let mut docs = self.docs.lock().expect("crdt store lock");
        if let Some(entry) = docs.get_mut(slug) {
            entry.mark_flushed();
        }
    }

    /// Collect slugs eligible for eviction (no clients + past eviction TTL).
    pub fn slugs_for_eviction(&self) -> Vec<String> {
        let docs = self.docs.lock().expect("crdt store lock");
        let now = Instant::now();
        docs.iter()
            .filter(|(_, entry)| {
                entry.client_count == 0
                    && entry
                        .disconnected_at
                        .map_or(false, |t| now.duration_since(t) >= self.eviction_ttl)
            })
            .map(|(slug, _)| slug.clone())
            .collect()
    }

    /// Evict a document from the store. If dirty, serializes it first and
    /// returns the markdown for the caller to flush to disk.
    pub fn evict(&self, slug: &str) -> Option<String> {
        let mut docs = self.docs.lock().expect("crdt store lock");
        if let Some(entry) = docs.get(slug) {
            let markdown = if entry.dirty {
                entry.doc.to_markdown().ok()
            } else {
                None
            };
            docs.remove(slug);
            markdown
        } else {
            None
        }
    }

    /// Evict the least-recently-used document that has no active clients.
    /// If the evicted doc is dirty, returns its slug and markdown for flushing.
    fn evict_lru(
        &self,
        docs: &mut HashMap<String, CrdtDocEntry>,
    ) -> Result<(), anyhow::Error> {
        // Find the LRU entry with no active clients
        let lru_slug = docs
            .iter()
            .filter(|(_, entry)| entry.client_count == 0)
            .min_by_key(|(_, entry)| entry.last_access)
            .map(|(slug, _)| slug.clone());

        if let Some(slug) = lru_slug {
            if let Some(entry) = docs.get(&slug) {
                if entry.dirty {
                    // Flush before evicting
                    if let Ok(md) = entry.doc.to_markdown() {
                        let path = self.md_path_for_slug(&slug);
                        std::fs::write(&path, &md).map_err(|e| {
                            anyhow::anyhow!("flush on evict {}: {e}", path.display())
                        })?;
                    }
                }
            }
            docs.remove(&slug);
        } else {
            // All docs have active clients — cannot evict, allow over-capacity
            eprintln!(
                "warning: CRDT store at capacity ({}) with all docs active",
                self.max_docs
            );
        }

        Ok(())
    }

    /// Return all currently loaded slugs.
    pub fn loaded_slugs(&self) -> Vec<String> {
        self.docs
            .lock()
            .expect("crdt store lock")
            .keys()
            .cloned()
            .collect()
    }

    /// Check if a document is loaded.
    pub fn is_loaded(&self, slug: &str) -> bool {
        self.docs.lock().expect("crdt store lock").contains_key(slug)
    }

    /// Return CRDT metadata for a loaded document: `(dirty, secs_since_flush, content_hash)`.
    ///
    /// `content_hash` is a BLAKE3 hex digest of the live CRDT markdown.  When
    /// the document is dirty, this hash differs from the on-disk merkle hash,
    /// enabling API clients to detect conflicts with active CRDT sessions.
    ///
    /// Returns `None` if the document is not loaded.
    pub fn crdt_meta(&self, slug: &str) -> Option<CrdtMeta> {
        let mut docs = self.docs.lock().expect("crdt store lock");
        let entry = docs.get_mut(slug)?;
        let secs_since_flush = entry.last_flush.elapsed().as_secs();
        let dirty = entry.dirty;
        let client_count = entry.client_count;
        // Compute BLAKE3 hash of the live CRDT markdown content.
        let content_hash = entry
            .doc
            .to_markdown()
            .ok()
            .map(|md| blake3::hash(md.as_bytes()).to_hex().to_string());
        Some(CrdtMeta {
            dirty,
            secs_since_flush,
            client_count,
            content_hash,
        })
    }

    /// Number of currently loaded documents.
    pub fn doc_count(&self) -> usize {
        self.docs.lock().expect("crdt store lock").len()
    }

    /// Flush all dirty documents and clear the store (for shutdown).
    pub fn flush_all(&self) {
        let mut docs = self.docs.lock().expect("crdt store lock");
        for (slug, entry) in docs.iter() {
            if entry.dirty {
                if let Ok(md) = entry.doc.to_markdown() {
                    let path = self.md_path_for_slug(slug);
                    if let Err(e) = std::fs::write(&path, &md) {
                        eprintln!("error flushing {slug} on shutdown: {e}");
                    }
                }
            }
        }
        docs.clear();
    }

    /// Reconcile external edits against loaded CRDT documents (REQ-020-040).
    ///
    /// For each changed slug, checks whether a CRDT session is active and handles:
    /// - **Case 1 (clean):** Discard in-memory CRDT, reload from disk.
    /// - **Case 2 (dirty):** Parse external file into CRDT, merge with live CRDT.
    /// - **Case 3 (deleted):** If file no longer exists and CRDT was dirty, write
    ///   recovery file to `.zetl/recovery/<slug>.md`, then evict.
    ///
    /// Returns a list of `ServerMsg` to broadcast per slug.
    pub fn reconcile_crdt(
        &self,
        slug: &str,
        file_exists: bool,
    ) -> Vec<ServerMsg> {
        let mut msgs = Vec::new();
        let mut docs = self.docs.lock().expect("crdt store lock");

        let entry = match docs.get_mut(slug) {
            Some(e) => e,
            None => return msgs, // No active CRDT session for this slug
        };

        if !file_exists {
            // ── Case 3: File deleted externally ──────────────────────────
            if entry.dirty {
                // Write recovery file with unflushed edits.
                if let Ok(md) = entry.doc.to_markdown() {
                    let recovery_dir = self.vault_root.join(".zetl").join("recovery");
                    if let Err(e) = std::fs::create_dir_all(&recovery_dir) {
                        eprintln!("crdt-reconcile: failed to create recovery dir: {e}");
                    } else {
                        let flat_slug = slug.replace('/', "--");
                        let recovery_path = recovery_dir.join(format!("{flat_slug}.md"));
                        match std::fs::write(&recovery_path, &md) {
                            Ok(()) => {
                                eprintln!(
                                    "warning: file deleted externally with unflushed CRDT edits — \
                                     recovery written to {}",
                                    recovery_path.display()
                                );
                            }
                            Err(e) => {
                                eprintln!(
                                    "crdt-reconcile: failed to write recovery file {}: {e}",
                                    recovery_path.display()
                                );
                            }
                        }
                    }
                }
            }
            // Evict the CRDT session.
            docs.remove(slug);
            msgs.push(ServerMsg::Deleted {
                page: slug.to_string(),
            });
            return msgs;
        }

        if !entry.dirty {
            // ── Case 1: Clean CRDT — reload from disk ────────────────────
            let md_path = self.md_path_for_slug(slug);
            match std::fs::read_to_string(&md_path) {
                Ok(markdown) => match CrdtDocument::from_markdown(&markdown) {
                    Ok(new_doc) => {
                        entry.doc = new_doc;
                        entry.last_access = Instant::now();
                        entry.last_flush = Instant::now();
                        // Send fresh sync to all connected editors.
                        let bytes = entry.doc.save();
                        let b64 = base64::Engine::encode(
                            &base64::engine::general_purpose::STANDARD,
                            &bytes,
                        );
                        msgs.push(ServerMsg::Sync { doc: b64 });
                    }
                    Err(e) => {
                        eprintln!("crdt-reconcile: reload parse error for {slug}: {e}");
                    }
                },
                Err(e) => {
                    eprintln!("crdt-reconcile: reload read error for {slug}: {e}");
                }
            }
        } else {
            // ── Case 2: Dirty CRDT — merge external with live ────────────
            let md_path = self.md_path_for_slug(slug);
            match std::fs::read_to_string(&md_path) {
                Ok(markdown) => match CrdtDocument::from_markdown(&markdown) {
                    Ok(mut external_doc) => {
                        if let Err(e) = entry.doc.merge(&mut external_doc) {
                            eprintln!("crdt-reconcile: merge error for {slug}: {e}");
                        } else {
                            entry.last_access = Instant::now();
                            // Stays dirty — merged state will flush on quiescence.
                            let bytes = entry.doc.save();
                            let b64 = base64::Engine::encode(
                                &base64::engine::general_purpose::STANDARD,
                                &bytes,
                            );
                            msgs.push(ServerMsg::ExternalMerge {
                                slug: slug.to_string(),
                                doc: b64,
                            });
                        }
                    }
                    Err(e) => {
                        eprintln!("crdt-reconcile: merge parse error for {slug}: {e}");
                    }
                },
                Err(e) => {
                    eprintln!("crdt-reconcile: merge read error for {slug}: {e}");
                }
            }
        }

        msgs
    }
}

/// Spawn a background task that periodically checks for quiescence flushes
/// and TTL evictions. Returns a `JoinHandle` that can be aborted on shutdown.
pub fn spawn_crdt_lifecycle_task(
    store: CrdtDocStore,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
        loop {
            interval.tick().await;

            // Quiescence flush: serialize dirty docs that have been idle
            let to_flush = store.slugs_needing_flush();
            for slug in &to_flush {
                if let Some(md) = store.serialize_for_flush(slug) {
                    let path = store.md_path_for_slug(slug);
                    match std::fs::write(&path, &md) {
                        Ok(()) => {
                            store.mark_flushed(slug);
                        }
                        Err(e) => {
                            eprintln!("error: quiescence flush for {slug}: {e}");
                        }
                    }
                }
            }

            // TTL eviction: remove docs where all clients left > eviction_ttl ago
            let to_evict = store.slugs_for_eviction();
            for slug in &to_evict {
                if let Some(md) = store.evict(slug) {
                    // Dirty doc was evicted — write it out
                    let path = store.md_path_for_slug(slug);
                    if let Err(e) = std::fs::write(&path, &md) {
                        eprintln!("error: eviction flush for {slug}: {e}");
                    }
                }
            }
        }
    })
}

// ── Ticket auth ──────────────────────────────────────────────────────

/// One-time ticket store for WebSocket auth (agents can't send cookies).
/// Tickets are single-use and expire after 30 seconds.
#[derive(Clone, Default)]
pub struct TicketStore {
    tickets: Arc<Mutex<HashMap<String, TicketEntry>>>,
}

struct TicketEntry {
    user_id: String,
    created: std::time::Instant,
}

/// Ticket lifetime (30 seconds).
const TICKET_TTL: std::time::Duration = std::time::Duration::from_secs(30);

impl TicketStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Issue a ticket for a user. Returns the opaque ticket token.
    pub fn issue(&self, user_id: &str) -> String {
        let token = blake3::hash(uuid::Uuid::new_v4().as_bytes())
            .to_hex()
            .to_string();
        let mut tickets = self.tickets.lock().expect("ticket lock");
        tickets.insert(
            token.clone(),
            TicketEntry {
                user_id: user_id.to_string(),
                created: std::time::Instant::now(),
            },
        );
        token
    }

    /// Consume a ticket, returning the user_id if valid and not expired.
    pub fn redeem(&self, token: &str) -> Option<String> {
        let mut tickets = self.tickets.lock().expect("ticket lock");
        let entry = tickets.remove(token)?;
        if entry.created.elapsed() > TICKET_TTL {
            return None;
        }
        Some(entry.user_id)
    }
}

// ── Handler ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct WsQuery {
    ticket: Option<String>,
}

/// Axum handler for `GET /ws/edit/{slug}` — upgrades to WebSocket.
pub async fn ws_edit_handler(
    ws: WebSocketUpgrade,
    AxumPath(slug): AxumPath<String>,
    Query(query): Query<WsQuery>,
    State(state): State<WebState>,
) -> impl IntoResponse {
    // Authenticate: ticket param or fall through (collab_gate already ran)
    let user_id = authenticate(&state, &query);

    match user_id {
        Some(uid) => ws.on_upgrade(move |socket| handle_socket(socket, slug, uid, state)),
        None => axum::http::StatusCode::UNAUTHORIZED.into_response(),
    }
}

/// Authenticate the WebSocket upgrade request.
fn authenticate(state: &WebState, query: &WsQuery) -> Option<String> {
    // Non-collab mode: no auth required, use anonymous identity
    if !state.collab {
        return Some("anonymous".to_string());
    }

    // Try ticket auth (primary path for WebSocket since cookies may not
    // be forwarded by all WebSocket clients)
    if let Some(ticket) = &query.ticket {
        if let Some(uid) = state.ticket_store.redeem(ticket) {
            return Some(uid);
        }
    }

    None
}

/// Handle an authenticated WebSocket connection.
async fn handle_socket(mut socket: WebSocket, slug: String, user_id: String, state: WebState) {
    // Load CRDT document on first connection (REQ-020-029)
    if let Err(e) = state.crdt_store.load_or_get(&slug) {
        eprintln!("error loading CRDT for {slug}: {e}");
        let err_json = serde_json::to_string(&ServerMsg::Error {
            message: format!("failed to load document: {e}"),
        })
        .unwrap_or_default();
        let _ = socket.send(Message::Text(err_json.into())).await;
        return;
    }

    // Track client connection
    state.crdt_store.client_connected(&slug);

    let room = state.ws_hub.room(&slug);
    let mut rx = room.tx.subscribe();

    // Send current CRDT state to joiner from the doc store
    let initial_doc = state.crdt_store.get_doc_state_b64(&slug)
        .or_else(|| {
            let doc = room.doc_state.lock().expect("doc lock");
            doc.clone()
        });
    if let Some(doc_b64) = initial_doc {
        let msg = ServerMsg::Sync { doc: doc_b64 };
        if let Ok(json) = serde_json::to_string(&msg) {
            if socket.send(Message::Text(json.into())).await.is_err() {
                state.crdt_store.client_disconnected(&slug);
                return;
            }
        }
    }

    // Use a tokio mpsc channel to allow both the relay task and the read loop
    // to send outbound messages through the single WebSocket sender.
    let (outbound_tx, mut outbound_rx) = tokio::sync::mpsc::channel::<ServerMsg>(64);

    // Spawn task: relay broadcast messages to the outbound channel
    let relay_user_id = user_id.clone();
    let relay_tx = outbound_tx.clone();
    let relay_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(server_msg) => {
                    // Don't echo back to sender
                    let is_self = match &server_msg {
                        ServerMsg::Op { user_id: uid, .. } => *uid == relay_user_id,
                        ServerMsg::Presence { user_id: uid, .. } => *uid == relay_user_id,
                        _ => false,
                    };
                    if is_self {
                        continue;
                    }
                    if relay_tx.send(server_msg).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Main loop: multiplex inbound WS messages and outbound relay messages.
    loop {
        tokio::select! {
            // Outbound: relay → WebSocket
            Some(server_msg) = outbound_rx.recv() => {
                if let Ok(json) = serde_json::to_string(&server_msg) {
                    if socket.send(Message::Text(json.into())).await.is_err() {
                        break;
                    }
                }
            }
            // Inbound: WebSocket → process
            ws_msg = socket.recv() => {
                match ws_msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Err(e) = process_client_msg(text.as_str(), &user_id, &slug, &room, &state.crdt_store, &state.wal_store) {
                            eprintln!("ws error for {user_id} on {slug}: {e}");
                            let err_json = serde_json::to_string(&ServerMsg::Error {
                                message: e.to_string(),
                            }).unwrap_or_default();
                            let _ = socket.send(Message::Text(err_json.into())).await;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {} // ignore binary/ping/pong
                }
            }
        }
    }

    relay_task.abort();
    state.crdt_store.client_disconnected(&slug);
    state.ws_hub.remove_if_empty(&slug);
}

/// Process a single inbound client message.
fn process_client_msg(
    text: &str,
    user_id: &str,
    slug: &str,
    room: &EditRoom,
    crdt_store: &CrdtDocStore,
    wal_store: &WalStore,
) -> Result<(), anyhow::Error> {
    let msg: ClientMsg =
        serde_json::from_str(text).map_err(|e| anyhow::anyhow!("invalid message: {e}"))?;

    match msg {
        ClientMsg::Sync { doc } => {
            // Client pushes full doc state — apply to CRDT store and broadcast
            if let Err(e) = crdt_store.apply_sync(slug, &doc) {
                eprintln!("warning: CRDT sync apply failed for {slug}: {e}");
            }
            {
                let mut state = room.doc_state.lock().expect("doc lock");
                *state = Some(doc.clone());
            }
            let _ = room.tx.send(ServerMsg::Sync { doc });
        }
        ClientMsg::Op { ops } => {
            // Append to WAL before applying (REQ-020-044)
            if let Err(e) = wal_store.append_ops(slug, &ops) {
                eprintln!("warning: WAL append failed for {slug}: {e}");
            }
            // Apply operations to the server-side CRDT document
            if let Err(e) = crdt_store.apply_ops(slug, &ops) {
                eprintln!("warning: CRDT op apply failed for {slug}: {e}");
            }
            crdt_store.record_edit(slug);
            let _ = room.tx.send(ServerMsg::Op {
                ops,
                user_id: user_id.to_string(),
            });
        }
        ClientMsg::Presence { cursor, name } => {
            let _ = room.tx.send(ServerMsg::Presence {
                user_id: user_id.to_string(),
                cursor,
                name,
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sync_msg() {
        let json = r#"{"type":"sync","doc":"AQID"}"#;
        let msg: ClientMsg = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, ClientMsg::Sync { doc } if doc == "AQID"));
    }

    #[test]
    fn parse_op_splice_msg() {
        let json = r#"{"type":"op","ops":[{"action":"splice","pos":5,"del":0,"text":"hello"}]}"#;
        let msg: ClientMsg = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, ClientMsg::Op { ops } if ops.len() == 1));
    }

    #[test]
    fn parse_op_mark_msg() {
        let json = r#"{"type":"op","ops":[{"action":"mark","name":"bold","value":true,"start":0,"end":5}]}"#;
        let msg: ClientMsg = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, ClientMsg::Op { ops } if ops.len() == 1));
    }

    #[test]
    fn parse_op_unmark_msg() {
        let json = r#"{"type":"op","ops":[{"action":"unmark","name":"bold","start":0,"end":5}]}"#;
        let msg: ClientMsg = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, ClientMsg::Op { ops } if ops.len() == 1));
    }

    #[test]
    fn parse_presence_msg() {
        let json = r#"{"type":"presence","cursor":{"index":10,"head":15},"name":"Alice"}"#;
        let msg: ClientMsg = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, ClientMsg::Presence { cursor, name } if cursor.index == 10 && name == Some("Alice".into())));
    }

    #[test]
    fn serialize_server_sync() {
        let msg = ServerMsg::Sync {
            doc: "AQID".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"sync""#));
        assert!(json.contains(r#""doc":"AQID""#));
    }

    #[test]
    fn serialize_server_op() {
        let msg = ServerMsg::Op {
            ops: vec![OpEntry::Splice {
                pos: 0,
                del: 0,
                text: "hi".into(),
            }],
            user_id: "alice".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"op""#));
        assert!(json.contains(r#""user_id":"alice""#));
    }

    #[test]
    fn serialize_server_presence() {
        let msg = ServerMsg::Presence {
            user_id: "bob".into(),
            cursor: CursorPos {
                index: 5,
                head: 10,
            },
            name: Some("Bob".into()),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"presence""#));
        assert!(json.contains(r#""user_id":"bob""#));
    }

    #[test]
    fn serialize_server_error() {
        let msg = ServerMsg::Error {
            message: "bad".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"error""#));
    }

    #[test]
    fn ws_hub_creates_room() {
        let hub = WsHub::new();
        let r1 = hub.room("readme");
        let r2 = hub.room("readme");
        // Same room instance
        assert!(Arc::ptr_eq(&r1, &r2));
    }

    #[test]
    fn ws_hub_different_slugs_different_rooms() {
        let hub = WsHub::new();
        let r1 = hub.room("readme");
        let r2 = hub.room("notes");
        assert!(!Arc::ptr_eq(&r1, &r2));
    }

    #[test]
    fn ticket_store_issue_and_redeem() {
        let store = TicketStore::new();
        let ticket = store.issue("alice");
        assert_eq!(store.redeem(&ticket), Some("alice".to_string()));
    }

    #[test]
    fn ticket_store_single_use() {
        let store = TicketStore::new();
        let ticket = store.issue("alice");
        assert!(store.redeem(&ticket).is_some());
        assert!(store.redeem(&ticket).is_none()); // consumed
    }

    #[test]
    fn ticket_store_invalid_token() {
        let store = TicketStore::new();
        assert!(store.redeem("bogus").is_none());
    }

    fn test_crdt_store() -> CrdtDocStore {
        let tmp = tempfile::tempdir().unwrap();
        CrdtDocStore::new(Arc::new(tmp.into_path()))
    }

    fn test_wal_store() -> (tempfile::TempDir, WalStore) {
        let tmp = tempfile::tempdir().unwrap();
        let wal = WalStore::new(tmp.path());
        (tmp, wal)
    }

    #[test]
    fn process_sync_stores_doc() {
        let room = EditRoom::new();
        let store = test_crdt_store();
        let (_wal_dir, wal) = test_wal_store();
        store.load_or_get("test").unwrap();
        process_client_msg(
            r#"{"type":"sync","doc":"AQID"}"#,
            "alice",
            "test",
            &room,
            &store,
            &wal,
        )
        .unwrap();
        let doc = room.doc_state.lock().unwrap();
        assert_eq!(*doc, Some("AQID".to_string()));
    }

    #[test]
    fn process_op_broadcasts() {
        let room = EditRoom::new();
        let mut rx = room.tx.subscribe();
        let store = test_crdt_store();
        let (_wal_dir, wal) = test_wal_store();
        store.load_or_get("test").unwrap();
        process_client_msg(
            r#"{"type":"op","ops":[{"action":"splice","pos":0,"del":0,"text":"hi"}]}"#,
            "alice",
            "test",
            &room,
            &store,
            &wal,
        )
        .unwrap();
        let msg = rx.try_recv().unwrap();
        assert!(matches!(msg, ServerMsg::Op { user_id, .. } if user_id == "alice"));
    }

    #[test]
    fn process_presence_broadcasts() {
        let room = EditRoom::new();
        let mut rx = room.tx.subscribe();
        let store = test_crdt_store();
        let (_wal_dir, wal) = test_wal_store();
        process_client_msg(
            r#"{"type":"presence","cursor":{"index":5,"head":5},"name":"Alice"}"#,
            "alice",
            "test",
            &room,
            &store,
            &wal,
        )
        .unwrap();
        let msg = rx.try_recv().unwrap();
        assert!(
            matches!(msg, ServerMsg::Presence { user_id, cursor, name } if user_id == "alice" && cursor.index == 5 && name == Some("Alice".into()))
        );
    }

    #[test]
    fn process_invalid_json_returns_error() {
        let room = EditRoom::new();
        let store = test_crdt_store();
        let (_wal_dir, wal) = test_wal_store();
        let result = process_client_msg("not json", "alice", "test", &room, &store, &wal);
        assert!(result.is_err());
    }

    // ── CrdtDocStore lifecycle tests (TEST-020-029) ──────────────────

    #[test]
    fn crdt_store_load_from_disk() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("readme.md"), "# Hello\n").unwrap();
        let store = CrdtDocStore::new(Arc::new(tmp.path().to_path_buf()));

        store.load_or_get("readme").unwrap();
        assert!(store.is_loaded("readme"));
        assert_eq!(store.doc_count(), 1);
    }

    #[test]
    fn crdt_store_load_missing_file_creates_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let store = CrdtDocStore::new(Arc::new(tmp.path().to_path_buf()));

        store.load_or_get("nonexistent").unwrap();
        assert!(store.is_loaded("nonexistent"));
    }

    #[test]
    fn crdt_store_client_tracking() {
        let store = test_crdt_store();
        store.load_or_get("page").unwrap();

        store.client_connected("page");
        store.client_connected("page");
        {
            let docs = store.docs.lock().unwrap();
            assert_eq!(docs["page"].client_count, 2);
            assert!(docs["page"].disconnected_at.is_none());
        }

        store.client_disconnected("page");
        {
            let docs = store.docs.lock().unwrap();
            assert_eq!(docs["page"].client_count, 1);
            assert!(docs["page"].disconnected_at.is_none());
        }

        store.client_disconnected("page");
        {
            let docs = store.docs.lock().unwrap();
            assert_eq!(docs["page"].client_count, 0);
            assert!(docs["page"].disconnected_at.is_some());
        }
    }

    #[test]
    fn crdt_store_dirty_after_edit() {
        let store = test_crdt_store();
        store.load_or_get("page").unwrap();

        {
            let docs = store.docs.lock().unwrap();
            assert!(!docs["page"].dirty);
        }

        store.record_edit("page");

        {
            let docs = store.docs.lock().unwrap();
            assert!(docs["page"].dirty);
        }
    }

    #[test]
    fn crdt_store_quiescence_flush_detection() {
        let store = test_crdt_store();
        store.load_or_get("page").unwrap();
        store.record_edit("page");

        // Just recorded edit — should not need flush yet (quiescence_delay = 5s)
        assert!(store.slugs_needing_flush().is_empty());

        // Manually set last_edit to past to simulate quiescence
        {
            let mut docs = store.docs.lock().unwrap();
            docs.get_mut("page").unwrap().last_edit =
                Instant::now() - Duration::from_secs(10);
        }

        let to_flush = store.slugs_needing_flush();
        assert_eq!(to_flush, vec!["page".to_string()]);
    }

    #[test]
    fn crdt_store_eviction_ttl() {
        let mut store = test_crdt_store();
        store.eviction_ttl = Duration::from_millis(1);
        store.load_or_get("page").unwrap();
        store.client_connected("page");
        store.client_disconnected("page");

        // Should not evict immediately (ttl is very short but let's just test the logic)
        std::thread::sleep(Duration::from_millis(5));

        let to_evict = store.slugs_for_eviction();
        assert_eq!(to_evict, vec!["page".to_string()]);

        store.evict("page");
        assert!(!store.is_loaded("page"));
    }

    #[test]
    fn crdt_store_memory_bound_lru_eviction() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = CrdtDocStore::new(Arc::new(tmp.path().to_path_buf()));
        store.max_docs = 3;

        // Load 3 docs
        for i in 0..3 {
            let slug = format!("page{i}");
            std::fs::write(tmp.path().join(format!("{slug}.md")), "# Page\n").unwrap();
            store.load_or_get(&slug).unwrap();
        }
        assert_eq!(store.doc_count(), 3);

        // Give page1 an older last_access
        {
            let mut docs = store.docs.lock().unwrap();
            docs.get_mut("page0").unwrap().last_access =
                Instant::now() - Duration::from_secs(100);
        }

        // Load 4th doc — should evict page0 (oldest LRU with no clients)
        std::fs::write(tmp.path().join("page3.md"), "# Page 3\n").unwrap();
        store.load_or_get("page3").unwrap();

        assert_eq!(store.doc_count(), 3);
        assert!(!store.is_loaded("page0"));
        assert!(store.is_loaded("page3"));
    }

    #[test]
    fn crdt_store_lru_skips_active_clients() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = CrdtDocStore::new(Arc::new(tmp.path().to_path_buf()));
        store.max_docs = 2;

        for i in 0..2 {
            let slug = format!("page{i}");
            std::fs::write(tmp.path().join(format!("{slug}.md")), "text\n").unwrap();
            store.load_or_get(&slug).unwrap();
        }

        // page0 has active clients — should not be evicted
        store.client_connected("page0");
        // page1 is the only one eligible
        {
            let mut docs = store.docs.lock().unwrap();
            docs.get_mut("page1").unwrap().last_access =
                Instant::now() - Duration::from_secs(100);
        }

        std::fs::write(tmp.path().join("page2.md"), "new\n").unwrap();
        store.load_or_get("page2").unwrap();

        assert!(store.is_loaded("page0")); // active client — kept
        assert!(!store.is_loaded("page1")); // evicted
        assert!(store.is_loaded("page2")); // newly loaded
    }

    #[test]
    fn crdt_store_flush_all_on_shutdown() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("readme.md"), "# Old\n").unwrap();
        let store = CrdtDocStore::new(Arc::new(tmp.path().to_path_buf()));

        store.load_or_get("readme").unwrap();

        // Apply an edit to make it dirty
        let ops = vec![OpEntry::Splice {
            pos: 0,
            del: 0,
            text: "Hello ".to_string(),
        }];
        store.apply_ops("readme", &ops).unwrap();

        store.flush_all();

        // Verify flushed to disk
        let content = std::fs::read_to_string(tmp.path().join("readme.md")).unwrap();
        assert!(content.contains("Hello"));
        assert_eq!(store.doc_count(), 0);
    }

    // ── CrdtMeta tests (TEST-020-CRDT-HISTORY) ─────────────────────────

    #[test]
    fn crdt_meta_none_for_unloaded_doc() {
        let store = test_crdt_store();
        assert!(store.crdt_meta("nonexistent").is_none());
    }

    #[test]
    fn crdt_meta_clean_doc() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("page.md"), "# Hello\n").unwrap();
        let store = CrdtDocStore::new(Arc::new(tmp.path().to_path_buf()));
        store.load_or_get("page").unwrap();

        let meta = store.crdt_meta("page").unwrap();
        assert!(!meta.dirty);
        assert_eq!(meta.client_count, 0);
        assert!(meta.content_hash.is_some());
        // Hash should be 64 hex chars (BLAKE3)
        let hash = meta.content_hash.unwrap();
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn crdt_meta_dirty_after_edit() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("page.md"), "# Hello\n").unwrap();
        let store = CrdtDocStore::new(Arc::new(tmp.path().to_path_buf()));
        store.load_or_get("page").unwrap();

        let before = store.crdt_meta("page").unwrap();
        assert!(!before.dirty);

        store.record_edit("page");

        let after = store.crdt_meta("page").unwrap();
        assert!(after.dirty);
    }

    #[test]
    fn crdt_meta_hash_changes_after_splice() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("page.md"), "# Hello\n").unwrap();
        let store = CrdtDocStore::new(Arc::new(tmp.path().to_path_buf()));
        store.load_or_get("page").unwrap();

        let hash_before = store.crdt_meta("page").unwrap().content_hash.unwrap();

        // Apply a splice operation
        let ops = vec![OpEntry::Splice {
            pos: 0,
            del: 0,
            text: "NEW ".to_string(),
        }];
        store.apply_ops("page", &ops).unwrap();

        let hash_after = store.crdt_meta("page").unwrap().content_hash.unwrap();
        assert_ne!(hash_before, hash_after, "hash should change after edit");
    }

    #[test]
    fn crdt_meta_tracks_client_count() {
        let store = test_crdt_store();
        store.load_or_get("page").unwrap();

        assert_eq!(store.crdt_meta("page").unwrap().client_count, 0);

        store.client_connected("page");
        assert_eq!(store.crdt_meta("page").unwrap().client_count, 1);

        store.client_connected("page");
        assert_eq!(store.crdt_meta("page").unwrap().client_count, 2);

        store.client_disconnected("page");
        assert_eq!(store.crdt_meta("page").unwrap().client_count, 1);
    }

    #[test]
    fn crdt_meta_flush_resets_dirty() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("page.md"), "# Hello\n").unwrap();
        let store = CrdtDocStore::new(Arc::new(tmp.path().to_path_buf()));
        store.load_or_get("page").unwrap();
        store.record_edit("page");

        assert!(store.crdt_meta("page").unwrap().dirty);

        store.mark_flushed("page");

        let meta = store.crdt_meta("page").unwrap();
        assert!(!meta.dirty);
        // secs_since_flush should be very small (just marked)
        assert!(meta.secs_since_flush < 2);
    }
}
