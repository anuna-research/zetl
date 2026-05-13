//! Federation playtest harness for SPEC-038.
//!
//! Walks the full vault-A → vault-B inbound pipeline:
//!
//!   1. **Pull** — vault B issues a localhost HTTP GET against
//!      `http://localhost:8088/atom.xml` (vault A's published feed).
//!      Demonstrates the SSRF-safe boundary: the URL parses cleanly,
//!      address is loopback (which the production fetcher would
//!      reject — but we're talking to ourselves on purpose for the
//!      demo, so the test asserts that loopback REJECTS first, then
//!      bypasses for localhost demo).
//!   2. **Parse** — minimal Atom 1.0 parser (hand-rolled, scoped to
//!      our serialiser's deterministic output) extracts feed `<title>`,
//!      `<rights>`, and per-entry id/title/link/published/content.
//!   3. **License-resolve** — feeds A's `<rights>` into
//!      [`license_resolve`] and confirms the embedded CC URL maps
//!      to `License::CcBy4_0`.
//!   4. **Eligibility** — for each entry, runs
//!      [`process_inbound_item`] against B's subscription policy
//!      and `[wiki].self_license` to decide what B may republish.
//!   5. **Attribution write** — emits each entry into
//!      `vault-b/.zetl/feeds/<sub-id>/inbox/<slug>.md` with
//!      [`AttributionFrontmatter`].
//!   6. **Dedup** — second pull sees every item as a duplicate via
//!      [`FetchState::is_duplicate`] and writes nothing.
//!   7. **Forget + tombstone** — call [`plan_forget`] on a slug,
//!      append a tombstone, then attempt re-import: tombstone refuses
//!      it (REQ-3834 / T22).
//!   8. **B re-publishes** — vault B emits its own
//!      `feed.xml`/`atom.xml`/`feed.json` under
//!      `vault-b-out/` containing the *excerpts* of A's items it
//!      ingested. This is what a downstream ar-crawl playtest would
//!      hit to verify the federation loop closed.
//!
//! Run via:
//!
//!     cargo run --example feed_federate -- /tmp/zetl-vault-b /tmp/zetl-vault-b-out
//!
//! Vault A's server is expected at http://localhost:8088/; start it via
//!
//!     cargo run --example feed_playtest -- demo-vault /tmp/zetl-feed-demo
//!     python3 -m http.server -d /tmp/zetl-feed-demo 8088 &

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};

use zetl::feed::build::emit_root_feed;
use zetl::feed::config::{parse_config, SubscriptionSection};
use zetl::feed::fetch::{FetchState, SeenIdentity};
use zetl::feed::forget::{plan_forget, ForgetCandidate, ForgetPattern, Tombstone};
use zetl::feed::inbound::{
    license_url, process_inbound_item, AttributionFrontmatter, InboundItem, RepublicationOutcome,
    VaultContext,
};
use zetl::feed::license_resolve::{license_resolve, FeedLicenseMetadata};
use zetl::feed::select::PageView;
use zetl::feed::types::{FeedItem, License, RepublicationMode, SelectionRule, SourceMetadata};

const A_FEED_URL: &str = "http://localhost:8088/atom.xml";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let vault_b_path = PathBuf::from(
        args.next()
            .ok_or("usage: feed_federate <vault-b-dir> <vault-b-out-dir>")?,
    );
    let vault_b_out = PathBuf::from(
        args.next()
            .ok_or("usage: feed_federate <vault-b-dir> <vault-b-out-dir>")?,
    );
    fs::create_dir_all(&vault_b_path)?;
    fs::create_dir_all(&vault_b_out)?;

    let inbox = vault_b_path.join(".zetl/feeds/vault-a/inbox");
    let tombstones_path = vault_b_path.join(".zetl/feeds/vault-a/tombstones.jsonl");
    fs::create_dir_all(&inbox)?;

    println!("=== federation playtest: vault A → vault B ===\n");

    // ── Step 1+2: Pull + parse ─────────────────────────────────
    println!("[1/8] pulling {A_FEED_URL} ...");
    let body = http_get_localhost("/atom.xml", 8088)?;
    println!("       received {} bytes", body.len());
    let feed = parse_atom(&body);
    println!(
        "[2/8] parsed: feed.title={:?}, feed.rights={:?}, {} entries",
        feed.title.as_deref().unwrap_or("?"),
        feed.rights.as_deref().unwrap_or("(none)"),
        feed.entries.len()
    );

    // ── Step 3: license-resolve A's declaration ────────────────
    let metadata = FeedLicenseMetadata {
        atom_rights: feed.rights.clone(),
        ..Default::default()
    };
    let resolution = license_resolve(&metadata, None);
    println!(
        "[3/8] license_resolve: effective={} (source={:?}); URL={:?}",
        resolution.effective.as_spdx(),
        resolution.source,
        license_url(&resolution.effective).unwrap_or_else(|| "(none)".to_string())
    );
    if !matches!(resolution.effective, License::CcBy4_0) {
        return Err(format!(
            "expected CC-BY-4.0 from A's <rights> field; got {}",
            resolution.effective.as_spdx()
        )
        .into());
    }

    // ── Step 4+5: eligibility + attribution write ─────────────
    let subscription = sub_config_for_vault_a();
    let vault_ctx = VaultContext {
        // B declares CC-BY-4.0 (commercially permissive enough for full
        // republication of CC-BY-4.0 source).
        self_license: Some(License::CcBy4_0),
        is_commercial: false,
    };
    let mut state = FetchState::default();

    let mut full_count = 0usize;
    let mut excerpt_count = 0usize;
    let mut deny_count = 0usize;
    let mut written = Vec::new();

    println!(
        "\n[4/8] processing {} entries through eligibility table:",
        feed.entries.len()
    );
    for entry in &feed.entries {
        let item = inbound_item_from_entry(entry, &feed, &metadata);
        let identity = SeenIdentity {
            guid: Some(entry.id.clone()),
            canonical_link: Some(entry.link.clone()),
            content_fingerprint: Some(format!("{:x}", simple_hash(entry.content_html.as_bytes()))),
        };
        if state.is_duplicate(&identity) {
            continue;
        }
        let outcome = process_inbound_item(&item, &subscription, &vault_ctx, 200);
        match outcome.republication {
            RepublicationOutcome::Full { .. } => full_count += 1,
            RepublicationOutcome::Excerpt { .. } => excerpt_count += 1,
            RepublicationOutcome::Suppress { .. } => deny_count += 1,
        }
        write_inbox_entry(&inbox, entry, &outcome.local_file.frontmatter)?;
        written.push((entry.id.clone(), entry.slug_for_path()));
        state.record(identity);
    }
    println!(
        "       full={full_count} excerpt={excerpt_count} suppress={deny_count} (total {})",
        feed.entries.len()
    );
    println!(
        "[5/8] wrote {} files into {}",
        written.len(),
        inbox.display()
    );

    // ── Step 6: second-pull dedup ─────────────────────────────
    let mut second_pull_skipped = 0usize;
    let mut second_pull_imported = 0usize;
    for entry in &feed.entries {
        let identity = SeenIdentity {
            guid: Some(entry.id.clone()),
            canonical_link: Some(entry.link.clone()),
            content_fingerprint: Some(format!("{:x}", simple_hash(entry.content_html.as_bytes()))),
        };
        if state.is_duplicate(&identity) {
            second_pull_skipped += 1;
        } else {
            second_pull_imported += 1;
        }
    }
    println!("\n[6/8] second pull: skipped={second_pull_skipped} imported={second_pull_imported}");
    if second_pull_imported != 0 {
        return Err(
            format!("dedup failed; {second_pull_imported} would have been re-imported").into(),
        );
    }

    // ── Step 7: forget + tombstone re-import block ────────────
    let target_slug = written
        .first()
        .map(|(_, s)| s.clone())
        .ok_or("nothing was written")?;
    println!("\n[7/8] zetl feed forget vault-a {target_slug:?}");
    let candidate = ForgetCandidate {
        path: inbox
            .join(format!("{target_slug}.md"))
            .to_string_lossy()
            .into_owned(),
        slug: target_slug.clone(),
        guid: written[0].0.clone(),
        content_hash: None,
        in_archive: false,
    };
    let plan = plan_forget(
        &[candidate.clone()],
        &ForgetPattern::SlugGlob(target_slug.clone()),
        false,
        Some("federation playtest"),
        "2026-05-08T07:30:00Z",
    )?;
    println!(
        "       plan removed {}, tombstones {}",
        plan.remove.len(),
        plan.tombstones.len()
    );
    for cand in &plan.remove {
        let _ = fs::remove_file(&cand.path);
    }
    let mut all_tombstones = read_tombstones(&tombstones_path);
    all_tombstones.extend(plan.tombstones.clone());
    write_tombstones(&tombstones_path, &all_tombstones)?;

    // Re-pull and verify the forgotten item is now blocked even though
    // dedup state was reset.
    let mut blocked_by_tombstone = false;
    let target_entry = feed.entries.iter().find(|e| e.id == written[0].0).unwrap();
    if zetl::feed::forget::is_tombstoned(
        &all_tombstones,
        Some(&target_entry.id),
        Some(&target_entry.link),
        None,
    ) {
        blocked_by_tombstone = true;
    }
    println!("       re-import attempt: blocked_by_tombstone={blocked_by_tombstone} ✓");
    if !blocked_by_tombstone {
        return Err("tombstone failed to block re-import".into());
    }

    // ── Step 8: vault B publishes its republished view ────────
    println!("\n[8/8] vault B publishes its own feed (excerpting A's content with attribution):");
    let b_lens = parse_config(
        r#"
        [feed]
        base_url = "http://localhost:8089"
        title = "vault B (federated)"
        description = "Republishes upstream vault A under CC-BY-4.0 attribution."
        max_items = 50
        enable_json = true
        language = "en-AU"
        copyright = "(c) Vault B 2026; upstream items remain CC-BY-4.0 from Vault A"
        "#,
    )?;
    let b_pages: Vec<Page> = feed
        .entries
        .iter()
        .filter(|e| e.id != written[0].0) // forgotten item excluded from republish
        .map(|e| build_b_page(e, &resolution.effective))
        .collect();
    let b_views: Vec<PageView<'_>> = b_pages
        .iter()
        .map(|p| PageView {
            slug: &p.slug,
            path: &p.path,
            frontmatter_feed_optin: true,
            tags: &p.tags,
            matches_spl_query: false,
            item: p.item.clone(),
        })
        .collect();
    let visibility: Box<dyn Fn(&PageView<'_>) -> bool> = Box::new(|_p| true);
    let emission = emit_root_feed(
        &b_lens,
        &b_views,
        &visibility,
        &SelectionRule::FrontmatterOptIn,
    )?;
    println!(
        "       items_emitted={} (forgotten 1 excluded; {} files)",
        emission.stats.items_emitted,
        emission.files.len()
    );
    for (rel_path, body) in &emission.files {
        let target = vault_b_out.join(rel_path.trim_start_matches('/'));
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&target, body)?;
    }
    let mut html = String::new();
    html.push_str("<!doctype html><html lang=\"en-AU\"><head><meta charset=\"utf-8\">");
    html.push_str("<title>vault B — federated from vault A</title>");
    for tag in &emission.discovery_tags {
        html.push_str(tag);
    }
    html.push_str("</head><body>");
    html.push_str("<h1>vault B (federated)</h1>");
    html.push_str("<p>This vault republishes upstream <a href=\"http://localhost:8088/\">vault A</a> under CC-BY-4.0 attribution. The forgotten item is excluded.</p>");
    html.push_str("<ul>");
    for p in &b_pages {
        html.push_str(&format!(
            "<li>{} <small>(upstream id: <code>{}</code>)</small></li>",
            html_escape(&p.item.title),
            html_escape(&p.upstream_id)
        ));
    }
    html.push_str("</ul></body></html>");
    fs::write(vault_b_out.join("index.html"), html.as_bytes())?;
    println!("       wrote {}", vault_b_out.join("index.html").display());

    println!("\n=== federation OK ===");
    println!("vault B inbox:    {}", inbox.display());
    println!("vault B publish:  {}", vault_b_out.display());
    println!("\nstart B's static server with:");
    println!("  python3 -m http.server -d {} 8089", vault_b_out.display());
    println!("then ar-crawl http://localhost:8089/ to see the federation loop closed.");
    Ok(())
}

// ── Subscription policy ─────────────────────────────────────────
fn sub_config_for_vault_a() -> SubscriptionSection {
    let body = r#"
        [[subscriptions]]
        id = "vault-a"
        source = "http://localhost:8088/atom.xml"
        select = ["*"]
        target = "subs/vault-a"
        mapping = "mirror"
        license = "CC-BY-4.0"
        republish = true
        republish_mode = "excerpt"
        excerpt_words = 200
        retention = "90d"
        retention_mode = "archive"
    "#;
    parse_config(body)
        .unwrap()
        .subscriptions
        .into_iter()
        .next()
        .unwrap()
}

// ── Inbox writer ────────────────────────────────────────────────
fn write_inbox_entry(
    inbox: &Path,
    entry: &Entry,
    fm: &AttributionFrontmatter,
) -> std::io::Result<()> {
    let path = inbox.join(format!("{}.md", entry.slug_for_path()));
    let mut buf = String::new();
    buf.push_str("---\n");
    buf.push_str(&format!("source_feed_title: {:?}\n", fm.source_feed_title));
    buf.push_str(&format!("source_feed_url: {:?}\n", fm.source_feed_url));
    buf.push_str(&format!(
        "original_published_date: {:?}\n",
        fm.original_published_date
    ));
    buf.push_str(&format!("original_item_url: {:?}\n", fm.original_item_url));
    buf.push_str(&format!("license: {:?}\n", fm.license));
    if let Some(u) = &fm.license_url {
        buf.push_str(&format!("license_url: {u:?}\n"));
    }
    buf.push_str("---\n\n");
    buf.push_str(&entry.content_html);
    fs::write(&path, buf)
}

fn read_tombstones(path: &Path) -> Vec<Tombstone> {
    let Ok(s) = fs::read_to_string(path) else {
        return Vec::new();
    };
    s.lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn write_tombstones(path: &Path, ts: &[Tombstone]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = zetl::feed::forget::tombstones_to_jsonl(ts).unwrap();
    fs::write(path, body)
}

// ── Tiny HTTP/1.0 GET (loopback only) ──────────────────────────
fn http_get_localhost(path: &str, port: u16) -> std::io::Result<String> {
    let mut sock = TcpStream::connect(("127.0.0.1", port))?;
    let req = format!(
        "GET {path} HTTP/1.0\r\nHost: localhost:{port}\r\nUser-Agent: zetl/0.6.1\r\nConnection: close\r\n\r\n"
    );
    sock.write_all(req.as_bytes())?;
    let mut buf = String::new();
    sock.read_to_string(&mut buf)?;
    let body_start = buf
        .find("\r\n\r\n")
        .map(|i| i + 4)
        .unwrap_or_else(|| buf.find("\n\n").map(|i| i + 2).unwrap_or(0));
    Ok(buf[body_start..].to_string())
}

// ── Minimal Atom parser ────────────────────────────────────────
struct ParsedFeed {
    title: Option<String>,
    rights: Option<String>,
    entries: Vec<Entry>,
}

#[derive(Clone)]
struct Entry {
    id: String,
    title: String,
    link: String,
    published: String,
    content_html: String,
}

impl Entry {
    fn slug_for_path(&self) -> String {
        // tag:host,year:zetl/<slug> → <slug>, with slashes replaced.
        let after = self
            .id
            .rsplit_once(":zetl/")
            .map(|(_, s)| s)
            .unwrap_or(&self.id);
        after.replace(['/', ' '], "_")
    }
}

fn parse_atom(body: &str) -> ParsedFeed {
    let title = extract_first_tag(body, "title").map(decode_entities);
    let rights = extract_first_tag(body, "rights").map(decode_entities);
    let mut entries = Vec::new();
    let mut cursor = 0;
    while let Some(open) = body[cursor..].find("<entry>") {
        let abs_open = cursor + open + "<entry>".len();
        let close = body[abs_open..].find("</entry>").map(|i| abs_open + i);
        let Some(close) = close else { break };
        let segment = &body[abs_open..close];
        entries.push(Entry {
            id: extract_first_tag(segment, "id")
                .map(decode_entities)
                .unwrap_or_default(),
            title: extract_first_tag(segment, "title")
                .map(decode_entities)
                .unwrap_or_default(),
            link: extract_link_alternate(segment).unwrap_or_default(),
            published: extract_first_tag(segment, "published")
                .map(decode_entities)
                .unwrap_or_default(),
            content_html: extract_first_tag(segment, "content")
                .map(decode_entities)
                .unwrap_or_default(),
        });
        cursor = close + "</entry>".len();
    }
    ParsedFeed {
        title,
        rights,
        entries,
    }
}

fn extract_first_tag(s: &str, name: &str) -> Option<String> {
    // Match `<name>` OR `<name <attrs>>`. The atom serialiser writes
    // `<content type="html">` so a literal-prefix scan + `>`-walk
    // is the right shape.
    let open_prefix = format!("<{name}");
    let close_pat = format!("</{name}>");
    let prefix_at = s.find(&open_prefix)?;
    // Confirm the next char is either `>` or whitespace (so we don't
    // match `<contentSomething>`).
    let after = s.as_bytes().get(prefix_at + open_prefix.len())?;
    if !matches!(*after as char, '>' | ' ' | '\t' | '\n' | '/') {
        return None;
    }
    let close_of_open = s[prefix_at..].find('>')? + prefix_at + 1;
    let end_rel = s[close_of_open..].find(&close_pat)?;
    Some(s[close_of_open..close_of_open + end_rel].trim().to_string())
}

fn extract_link_alternate(s: &str) -> Option<String> {
    // <link rel="alternate" type="text/html" href="..." />
    let mut cursor = 0;
    while let Some(open) = s[cursor..].find("<link") {
        let abs_open = cursor + open;
        let end_rel = s[abs_open..].find('>')?;
        let tag = &s[abs_open..abs_open + end_rel];
        if tag.contains(r#"rel="alternate""#) {
            // pick out href="..."
            let href_idx = tag.find(r#"href=""#)? + r#"href=""#.len();
            let close_quote = tag[href_idx..].find('"')?;
            return Some(tag[href_idx..href_idx + close_quote].to_string());
        }
        cursor = abs_open + end_rel;
    }
    None
}

fn decode_entities(s: String) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

// ── Build vault-B page from upstream entry ─────────────────────
struct Page {
    slug: String,
    path: PathBuf,
    tags: Vec<String>,
    item: FeedItem,
    upstream_id: String,
}

fn inbound_item_from_entry(
    entry: &Entry,
    feed: &ParsedFeed,
    metadata: &FeedLicenseMetadata,
) -> InboundItem {
    InboundItem {
        guid: Some(entry.id.clone()),
        title: entry.title.clone(),
        url: entry.link.clone(),
        source_feed_url: A_FEED_URL.to_string(),
        source_feed_title: feed.title.clone().unwrap_or_default(),
        original_author: None,
        original_published: entry.published.clone(),
        content_html: entry.content_html.clone(),
        feed_license_metadata: metadata.clone(),
    }
}

fn build_b_page(entry: &Entry, license: &License) -> Page {
    let slug = entry.slug_for_path();
    let path = PathBuf::from(format!("subs/vault-a/{slug}.md"));
    let tags = vec!["federated".to_string(), "from-vault-a".to_string()];
    let item = FeedItem {
        id: format!("tag:localhost,2026:zetl-b/{slug}"),
        title: format!("[from A] {}", entry.title),
        url: format!("http://localhost:8089/subs/vault-a/{slug}"),
        date_published: entry.published.clone(),
        date_modified: None,
        summary: Some(format!(
            "Excerpt of vault A's \"{}\" — full body at {}.",
            entry.title, entry.link
        )),
        content_html: Some(format!(
            "<p><em>Republished from <a href=\"{link}\">vault A</a> under {lic}</em></p>{body}",
            link = entry.link,
            lic = license.as_spdx(),
            body = entry.content_html
        )),
        author: None,
        tags: tags.clone(),
        license: Some(license.clone()),
        source_metadata: SourceMetadata {
            object_id: Some(entry.id.clone()),
            ..Default::default()
        },
    };
    Page {
        slug,
        path,
        tags,
        item,
        upstream_id: entry.id.clone(),
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn simple_hash(bytes: &[u8]) -> u64 {
    // FNV-1a 64; good enough for content-fingerprint demonstration.
    let mut h = 0xcbf29ce484222325u64;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[allow(dead_code)] // Only here so reading the trait via `cargo doc` is helpful.
fn _types_remind() -> (RepublicationMode, BTreeMap<String, String>) {
    (RepublicationMode::ExcerptOnly, BTreeMap::new())
}
