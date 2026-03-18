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
use std::sync::{Arc, Mutex};

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Path, Query, State, WebSocketUpgrade};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use super::WebState;

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
    Path(slug): Path<String>,
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
    let room = state.ws_hub.room(&slug);
    let mut rx = room.tx.subscribe();

    // Send current doc state to joiner (if any previous state exists)
    let initial_doc = {
        let doc = room.doc_state.lock().expect("doc lock");
        doc.clone()
    };
    if let Some(doc_b64) = initial_doc {
        let msg = ServerMsg::Sync { doc: doc_b64 };
        if let Ok(json) = serde_json::to_string(&msg) {
            if socket.send(Message::Text(json.into())).await.is_err() {
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
                        if let Err(e) = process_client_msg(text.as_str(), &user_id, &room) {
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
    state.ws_hub.remove_if_empty(&slug);
}

/// Process a single inbound client message.
fn process_client_msg(
    text: &str,
    user_id: &str,
    room: &EditRoom,
) -> Result<(), anyhow::Error> {
    let msg: ClientMsg =
        serde_json::from_str(text).map_err(|e| anyhow::anyhow!("invalid message: {e}"))?;

    match msg {
        ClientMsg::Sync { doc } => {
            // Client pushes full doc state — store it and broadcast
            {
                let mut state = room.doc_state.lock().expect("doc lock");
                *state = Some(doc.clone());
            }
            let _ = room.tx.send(ServerMsg::Sync { doc });
        }
        ClientMsg::Op { ops } => {
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

    #[test]
    fn process_sync_stores_doc() {
        let room = EditRoom::new();
        process_client_msg(
            r#"{"type":"sync","doc":"AQID"}"#,
            "alice",
            &room,
        )
        .unwrap();
        let doc = room.doc_state.lock().unwrap();
        assert_eq!(*doc, Some("AQID".to_string()));
    }

    #[test]
    fn process_op_broadcasts() {
        let room = EditRoom::new();
        let mut rx = room.tx.subscribe();
        process_client_msg(
            r#"{"type":"op","ops":[{"action":"splice","pos":0,"del":0,"text":"hi"}]}"#,
            "alice",
            &room,
        )
        .unwrap();
        let msg = rx.try_recv().unwrap();
        assert!(matches!(msg, ServerMsg::Op { user_id, .. } if user_id == "alice"));
    }

    #[test]
    fn process_presence_broadcasts() {
        let room = EditRoom::new();
        let mut rx = room.tx.subscribe();
        process_client_msg(
            r#"{"type":"presence","cursor":{"index":5,"head":5},"name":"Alice"}"#,
            "alice",
            &room,
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
        let result = process_client_msg("not json", "alice", &room);
        assert!(result.is_err());
    }
}
