//! Runs the same two-user flush path the integration test exercises, against
//! a persistent directory so you can inspect the git log and jj log yourself.
//!
//! cargo run --example verify_author_attribution --features history

use std::sync::{Arc, Mutex, RwLock};

use zetl::search_index::SearchIndex;
use zetl::web::engine::TemplateEngine;
use zetl::web::flush::flush_pipeline;
use zetl::web::fs_watch::PendingWrites;
use zetl::web::ws::{CrdtDocStore, OpEntry};
use zetl::web::WebState;

fn main() -> anyhow::Result<()> {
    let dir = std::path::PathBuf::from("/tmp/zetl-verify-attribution");
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("note.md"), "# Note\n\nseed content\n")?;

    // git init + initial commit so auto_commit has a parent.
    let repo = git2::Repository::init(&dir)?;
    {
        let mut config = repo.config()?;
        config.set_str("user.name", "test")?;
        config.set_str("user.email", "test@test")?;
    }
    {
        let sig = git2::Signature::now("test", "test@test")?;
        let mut index = repo.index()?;
        index.add_path(std::path::Path::new("note.md"))?;
        index.write()?;
        let tree_oid = index.write_tree()?;
        let tree = repo.find_tree(tree_oid)?;
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])?;
    }

    // Two user profiles.
    let alice = zetl::user::UserProfile {
        id: "alice-11111111".to_string(),
        name: "Alice Example".to_string(),
        created_at: "2026-04-18T00:00:00Z".to_string(),
        invited_by: None,
        owner: true,
        credentials: Vec::new(),
        recovery_pubkey: String::new(),
        agent_token_generation: 0,
    };
    let bob = zetl::user::UserProfile {
        id: "bob-22222222".to_string(),
        name: "Bob Example".to_string(),
        created_at: "2026-04-18T00:00:00Z".to_string(),
        invited_by: Some(alice.id.clone()),
        owner: false,
        credentials: Vec::new(),
        recovery_pubkey: String::new(),
        agent_token_generation: 0,
    };
    zetl::user::save_profile(&dir, &alice)?;
    zetl::user::save_profile(&dir, &bob)?;

    // Build a minimal WebState.
    let vault_root = Arc::new(dir.clone());
    let data = zetl::web::reindex(&dir)?;
    let search_index = SearchIndex::build(&dir, &data.files)?;
    let engine = TemplateEngine::new(&dir, "default", false, false);
    let state = WebState {
        data: Arc::new(RwLock::new(data)),
        vault_root: vault_root.clone(),
        search_index: Arc::new(search_index),
        engine: Arc::new(engine),
        theme: "default".to_string(),
        verbose: false,
        collab: false,
        tls: false,
        trust_proxy: false,
        sessions: zetl::web::session::SessionStore::new(),
        recovery_challenges: Arc::new(zetl::user::recovery::RecoveryChallengeStore::new()),
        mnemonic_shown: Arc::new(Mutex::new(std::collections::HashSet::new())),
        bootstrap_used: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        rate_limiters: zetl::web::rate_limit::AuthRateLimiters::new(),
        #[cfg(feature = "reason")]
        acl_cache: Arc::new(Mutex::new(zetl::web::AclCache::new())),
        git_commit_lock: Some(Arc::new(Mutex::new(repo))),
        ws_hub: zetl::web::ws::WsHub::new(),
        ticket_store: zetl::web::ws::TicketStore::new(),
        crdt_store: CrdtDocStore::new(vault_root.clone()),
        wal_store: Arc::new(zetl::web::wal::WalStore::new(&vault_root)),
        pending_writes: PendingWrites::new(),
        passkey_mgr: None,
        public_dir: None,
        scan_options: zetl::scanner::ScanOptions::default(),
        #[cfg(feature = "semantic")]
        vector_index: None,
    };

    // Simulate two users editing the same doc within one quiescence window.
    state.crdt_store.load_or_get("note")?;
    let op = vec![OpEntry::Splice {
        pos: 0,
        del: 0,
        text: "edit-".to_string(),
    }];
    state.crdt_store.apply_ops("note", &alice.id, &op)?;
    state.crdt_store.apply_ops("note", &bob.id, &op)?;

    println!("→ applied ops as alice then bob; flushing...");
    let _ = flush_pipeline(&state, "note");

    // Dump the git commit and jj snapshot so you can see the attribution.
    println!("\n── git log -1 --format=full ────────────────────────────");
    std::process::Command::new("git")
        .args(["-C", dir.to_str().unwrap(), "log", "-1", "--format=full"])
        .status()?;

    #[cfg(feature = "history")]
    {
        use zetl::history::jj_backend::VcsBackend;
        let backend = zetl::history::jj_backend::JjBackend::open_or_init_at_vault_root(&dir)?;
        let changes = backend.list_changes(1)?;
        if let Some(latest) = changes.first() {
            println!("\n── jj latest snapshot ─────────────────────────────────");
            println!("change_id:    {}", latest.change_id);
            println!("author_name:  {}", latest.author_name);
            println!("author_email: {}", latest.author_email);
            println!("description:");
            for line in latest.description.lines() {
                println!("  {line}");
            }
            let co = zetl::history::core::parse_co_authored_by(&latest.description);
            println!("\nparsed co_authors: {co:?}");
        }
    }

    println!("\nDone. Vault at: {}", dir.display());
    Ok(())
}
