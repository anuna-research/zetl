---
title: Feeds (RSS, Atom, JSON Feed)
tags: [reading, build, publishing, rss, atom, jsonfeed, syndication]
---

# Feeds (RSS, Atom, JSON Feed)

`zetl build` and `zetl serve` can emit standards-conformant RSS 2.0, Atom 1.0, and (opt-in) JSON Feed v1.1 alongside your static site. Real RSS readers — NetNewsWire, Feedly, Inoreader — auto-discover the feeds via `<link rel="alternate">` tags injected into every published page. There's no JavaScript runtime, no server process, no extra dependency: the feeds are plain files written into `dist/` next to your HTML, and they pass `xmllint --noout` + `feedparser` (Python's reference parser) with zero warnings.

The implementation also handles **inbound** subscriptions: configure another vault's (or any blog's) feed URL in `[[subscriptions]]`, and `zetl feed pull` will fetch + dedup + apply Creative-Commons-aware republication policy + write the items into your `.zetl/feeds/<sub-id>/inbox/`. From there, the next `zetl build` pulls them into your own published feed under attribution.

The tracking spec is [SPEC-038](https://codeberg.org/anuna/zetl/src/branch/main/specs/SPEC-038-rss-support.md). Everything below is operator-facing how-to.

## Quick start (publishing a feed in 30 seconds)

Add a `[feed]` block to `.zetl/config.toml`:

```toml
[feed]
base_url = "https://yourwiki.example"
title = "Your wiki"
description = "Notes from the field"
```

Mark a couple of pages for inclusion via per-page frontmatter opt-in:

```markdown
---
title: My first feed item
date: "2026-04-15"
feed: true
---

# My first feed item
…
```

Then build:

```bash
zetl build
```

You now have:

- `dist/feed.xml` — RSS 2.0
- `dist/atom.xml` — Atom 1.0
- Discovery `<link rel="alternate">` tags in every page's `<head>`
- A small "Subscribe: RSS Atom" affordance at the bottom of each rendered page

To also emit JSON Feed v1.1, set `enable_json = true`:

```toml
[feed]
base_url = "https://yourwiki.example"
title = "Your wiki"
enable_json = true
```

Now `dist/feed.json` joins the others, and the Subscribe affordance grows a third "JSON Feed" link.

## How pages get into the feed

By default the feed includes pages with `feed: true` in their frontmatter. The intent is conservative: you publish a vault of hundreds of pages but only a few are blog-shaped; only the few opt in.

If you'd rather sweep an entire folder, declare a **scoped feed** (see below). The scope's selector overrides the per-page opt-in for that scope.

### Resolving the publication date

For each item in the feed, zetl walks this fallback chain:

1. `frontmatter.published`
2. `frontmatter.date`
3. `frontmatter.created`
4. (TODO) git-derived first/last-commit date (deferred — see SPEC-038 REQ-3804)
5. File mtime as a last resort

The first present value wins. Both quoted (`date: "2026-04-15"`) and unquoted (`date: 2026-04-15`) YAML forms are supported. Bare `YYYY-MM-DD` is normalised to RFC 3339 midnight UTC.

## Scoped feeds (Hugo's subscriptions catalog)

If you want multiple feeds — one for the blog, one for release notes, one for the worklog — use `[[feed.scopes]]`:

```toml
[feed]
base_url = "https://yourwiki.example"
title = "Your wiki"

[[feed.scopes]]
id = "blog"
title = "Blog"
path = "/blog/feed.xml"
select = "frontmatter"

[[feed.scopes]]
id = "notes"
title = "Notes"
path = "/notes/feed.xml"
select.folder = "notes/"

[[feed.scopes]]
id = "research"
title = "Research"
path = "/research/feed.xml"
select.tag = "research"
```

The selector forms are:

- `select = "frontmatter"` — `frontmatter.feed: true` opt-in (the default rule).
- `select.folder = "blog/"` — every page under that folder.
- `select.tag = "research"` — every page with that tag.
- `select.spl = "..."` — an SPL query (the reasoning engine resolves the page set).

Each scope gets emitted at the configured `path`. Discovery tags surface every scoped feed as its own `<link rel="alternate">`.

zetl also emits `dist/.well-known/zetl-subscriptions.json` — a public catalog advertising every scoped feed for downstream `zetl feed pull` subscribers (CON-3808). Capability cohort feeds are NEVER advertised in this catalog.

## Capability cohort feeds

If you've set up [[Capability URLs]] (SPEC-034), you can emit per-cohort feeds at `/caps/<cohort.token>/feed.xml`:

```toml
[[capability_cohorts]]
id = "research-collaborators"
token = "<≥128-bit entropy random token, generated externally>"
select = ["research/**", "notes/private/2026-Q1.md"]
feed_enabled = true
feed_title = "Research collaborators feed"
```

Cohort tokens **MUST** carry at least 128 bits of entropy (NFR-3809). zetl rejects weak tokens at config-load time:

```
[zetl] feed-config: .zetl/config.toml (REQ-3809 / NFR-3807 / NFR-3809):
  [[capability_cohorts]] 'research-collaborators': token entropy 56 bits
  < required 128 (len=10, alphabet=8)
```

Cohort feeds are NOT advertised in any public discovery surface (no `<link rel="alternate">` in public pages, no entry in `/.well-known/zetl-subscriptions.json`, no sitemap entry). The only authorised emission site is the `/caps/<token>/` URL itself — REQ-3831.

The cohort URL prefix is shared with SPEC-034 capability URLs by design (same auth boundary), but the two token pools are disjoint by construction: SPEC-034 grant tokens live in `grants.toml`, SPEC-038 cohort tokens live in `config.toml`, and operators MUST NOT cross-populate them. See [[Capability URLs]] for the SPEC-034 side.

## Subscribing to other feeds

To pull another vault's (or any blog's) feed into yours, add a `[[subscriptions]]` block:

```toml
[[subscriptions]]
id = "upstream-vault"
source = "https://upstream.example/atom.xml"
select = ["*"]
target = "subs/upstream"
mapping = "mirror"
license = "CC-BY-SA-4.0"          # operator override (CON-3811)
republish = true                  # opt-in to republishing
republish_mode = "excerpt"        # "full" or "excerpt"
excerpt_words = 200               # bounded [50, 500]
retention = "90d"                 # "forever" / "<N>d|w|mo|y" / "last-<N>"
retention_mode = "archive"        # default; or "delete"

[wiki]
self_license = "CC-BY-SA-4.0"     # gates CC-BY-SA full-republish
is_commercial = false             # gates CC-BY-NC full-republish
```

Then run `zetl feed pull` to fetch the upstream feed once. Pulled items are persisted to `.zetl/feeds/upstream-vault/inbox/<slug>.md` with REQ-3822 attribution frontmatter:

```markdown
---
source_feed_title: "Upstream vault"
source_feed_url: "https://upstream.example/atom.xml"
original_published_date: "2026-04-15T00:00:00Z"
original_item_url: "https://upstream.example/some-post"
license: "CC-BY-SA-4.0"
license_url: "https://creativecommons.org/licenses/by-sa/4.0/"
---

<original body>
```

Subsequent `zetl feed pull` calls dedup against three signals (GUID + canonical link + content fingerprint, REQ-3812) so the same item is never written twice. After pull, the next `zetl build` includes the inbox items in your republished output (gated by the eligibility table below).

> **Build-determinism note:** `zetl build` itself is offline + deterministic. The `zetl feed pull` step is the only network ceremony — schedule it as a separate CI step ahead of build, or pin the inbox snapshot in version control if you want fully reproducible builds.

### Creative-Commons republication eligibility

Republication is **private-by-default** (REQ-3818). When the upstream feed declares a CC license, the eligibility table from SPEC-038 REQ-3820 gates what you may publish:

| Source license | Receiving vault | Outcome |
|---|---|---|
| CC0 / public domain | any | full republication allowed |
| CC-BY (any version) | any | full or excerpt-mode (operator's `republish_mode` choice) + attribution mandatory |
| CC-BY-SA-4.0 | `[wiki].self_license = "CC-BY-SA-4.0"` | full allowed |
| CC-BY-SA-4.0 | any other / no self_license | downgraded to excerpt |
| CC-BY-NC-4.0 | `[wiki].is_commercial = false` | full allowed |
| CC-BY-NC-4.0 | `[wiki].is_commercial = true` | downgraded to excerpt |
| CC-BY-ND-4.0 | any | excerpt-only (wikilink rewriting modifies the body) |
| Unknown / no license | `republish = false` (default) | denied |
| Unknown / no license | `republish = true` AND `i_have_permission = true` | full or excerpt per operator choice — accepted operator risk per ADR-3809 |

The decision carries a structured `rationale` field naming which clause fired, surfaced in the OBS-3806 counter for audit.

> **Legal posture:** the eligibility table is defence in depth, not legal advice. Operators carry full legal responsibility for compliance with each upstream feed's license. Mark `i_have_permission = true` only with explicit out-of-band permission from the rights-holder; mark `is_commercial = false` only when the receiving vault is genuinely non-commercial. Verify with counsel before republishing third-party content under any of the CC-BY-NC / CC-BY-SA / Unknown branches.

### Retraction and forgetting

When an item disappears from the source feed on a subsequent pull, zetl marks the local file's frontmatter with `retracted_by_source: <ISO 8601 timestamp>` and removes it from the next republished build (REQ-3823).

To explicitly erase items you've already imported (e.g. a takedown request), use `zetl feed forget`:

```bash
# Forget by slug glob:
zetl feed forget upstream-vault "private/*"

# Forget by GUID prefix:
zetl feed forget upstream-vault "tag:upstream.example,2024:zetl/old-post"

# Forget by content-hash prefix (for content-addressed dedup keys):
zetl feed forget upstream-vault deadbeef --include-archive --reason "takedown 2026-05-08"

# Dry run first:
zetl feed forget upstream-vault "**/*.md" --dry-run
```

Forget is destructive: it removes the inbox file AND mints a tombstone in `.zetl/feeds/upstream-vault/tombstones.jsonl`. The tombstone blocks re-import on every subsequent `zetl feed pull` — even if the source republishes the same item with a fresh GUID, the content-hash signal still matches (REQ-3834 / T22 attack mitigation).

## CLI surface

`zetl feed` collects the inbound-side commands:

```text
zetl feed pull [SUB_ID...]                    # fetch one or all subscriptions
zetl feed list                                 # tabular subscription status
zetl feed status <SUB_ID>                      # detailed: dedup state, retention, errors
zetl feed validate <PATH>                      # strict-parser conformance smoke test
zetl feed forget <SUB_ID> <PATTERN>            # erase + tombstone

# Common flags:
--json                                         # JSON output regardless of TTY
--no-input                                     # disable interactive prompts (CI mode)
--include-archive                              # forget also archived/ entries
--reason TEXT                                  # captured in tombstone records
--dry-run                                      # forget without writing
--feed-format rss|atom|jsonfeed                # validate force-format (auto-detected by default)
```

Run `zetl feed --help` (or any subcommand `--help`) for the canonical surface.

> **Naming note:** the word "feed" appears on three axes in zetl. `[feed]` config = OUTBOUND publishing, configured at the vault level. `[[subscriptions]]` config + `zetl feed *` CLI = INBOUND subscription, ingested into `.zetl/feeds/<sub-id>/inbox/`. `frontmatter.feed: true` = per-page opt-in for the OUTBOUND root feed. They never overlap operationally — outbound is a `zetl build` byproduct, inbound is a `zetl feed pull` ceremony.

### v1 wire status

In v1.0:

- `zetl feed validate <path>` is fully wired (offline; XXE check + JSON Feed v1.1 schema check + size report).
- `zetl feed pull|list|status|forget` exit non-zero with a structured "not yet wired" stub error pointing at the deferred shell-side work. The pure-core pipeline (`feed::fetch::HttpTransport` trait + `feed::inbound::process_inbound_item` + `feed::forget::plan_forget`) is implemented and unit-tested; the final wire that connects them to disk + a real HTTP transport is the [next-cycle follow-up](https://codeberg.org/anuna/zetl/src/branch/main/plans/IMPL-038-wires.spl).

The outbound side (RSS / Atom / JSON Feed emission, scoped feeds, capability cohorts, discovery tags, theme Subscribe affordance) is fully wired and shipping in v1.0.

## Credentials

Per-feed authentication for inbound subscriptions lives in `.zetl/credentials.toml`, **not** `.zetl/config.toml`. Three schemes are supported (REQ-3824):

```toml
[upstream-basic]
auth_type = "basic"
username = "alice"
password = "hunter2"

[upstream-bearer]
auth_type = "bearer"
token = "abc123"

[upstream-query]
auth_type = "query_param"
token_param = "api_key"
token_value = "secret"
# OR pre-built URL form:
# url_with_token = "https://upstream.example/feed?api_key=secret"
```

zetl enforces several invariants on this file (REQ-3825):

- **Mode 0600** on read: looser permissions are rejected with a structured error naming the file + the actual mode. Run `chmod 600 .zetl/credentials.toml` to fix.
- **Mode 0600** on write: `zetl` writes credentials atomically (write-rename) with `O_CREAT` mode-0600 from open, so the secret is never visible at a looser mode even transiently.
- **No credential keys in `config.toml`**: putting `password`, `token`, `token_value`, `secret`, `api_key`, `auth`, or `access_token` in `.zetl/config.toml` produces a hard error citing REQ-3825 + ADR-3810. Move the leaked keys to `.zetl/credentials.toml`.
- **Cross-origin redirect drops credentials**: per REQ-3826, the inbound fetcher strips Authorization headers AND query-param tokens on every cross-origin redirect hop. The redirected request runs unauthenticated.
- **Persistent 401/403 → suspended**: per REQ-3828, after one re-read + retry, persistent auth failures pause the subscription. Operators see a `zetl_feed_inbound_auth_failure_total` counter increment + a structured warn-line.

When you run `zetl init`, `.zetl/credentials.toml` and `.zetl/credentials/` are appended to `.gitignore` automatically. Vault-export commands exclude these paths by default.

## On-disk layout

```
<vault-root>/
├── .zetl/
│   ├── config.toml                        # public; in version control
│   ├── credentials.toml                   # mode 0600; gitignored
│   ├── feeds/
│   │   └── <sub-id>/
│   │       ├── inbox/                     # ingested items as Markdown
│   │       ├── archived/                  # retention-archived items (REQ-3833)
│   │       └── tombstones.jsonl           # forget records (REQ-3834)
│   ├── feed-state/
│   │   └── <feed-id>/
│   │       ├── snapshot.merkle            # AST snapshot for changelog feed
│   │       └── events.jsonl               # append-only event log
│   └── ... (other zetl state)
└── dist/                                  # zetl build output
    ├── feed.xml                           # root RSS 2.0
    ├── atom.xml                           # root Atom 1.0
    ├── feed.json                          # root JSON Feed v1.1 (when enable_json)
    ├── blog/feed.xml                      # per-scope (REQ-3813)
    ├── changelog.xml                      # AST-backed changelog feed (REQ-3816)
    ├── caps/<token>/feed.xml              # cohort feeds (REQ-3829)
    └── .well-known/zetl-subscriptions.json
```

## Conformance + interoperability

The implementation has been audited against three external specifications:

- **RSS 2.0** ([rssboard.org](https://www.rssboard.org/rss-specification)) — channel-level title / link / description, item-level guid with `isPermaLink="false"`, RFC 822 pubDate, IANA-registered link schemes, atom:link rel=self extension. `feedparser` reports `bozo=False, version='rss20'`.
- **Atom 1.0** ([RFC 4287](https://datatracker.ietf.org/doc/html/rfc4287)) — feed-level id / title / updated / atom:author (mandatory per §4.1.1), entry-level id / title / updated / published, rel=alternate + rel=self links, xml:lang attribute on root, `_zetl:` namespaced extension for source metadata. `feedparser` reports `bozo=False, version='atom10'`.
- **JSON Feed v1.1** ([jsonfeed.org/version/1.1](https://www.jsonfeed.org/version/1.1/)) — required `version` URL, top-level `title` / `items`, per-item `id` + `content_html`, `authors` array (not legacy singular `author`), RFC 3339 `date_published`, application/feed+json MIME.

`zetl feed validate <path>` runs an offline conformance smoke test on any feed body — your own published feeds before deploy or upstream feeds before subscribing.

## Determinism

Per NFR-3804 + NFR-3805, the published feeds are byte-deterministic for byte-identical input: the same vault produces byte-identical `feed.xml`, `atom.xml`, and `feed.json` across rebuilds, machines, and zetl versions in the same major-version line. Item ids are stable RFC 4151 tag URIs of the form `tag:<host>,<creation-year>:zetl/<slug>`. Frontmatter mtime, build timestamp, and host clock never leak into the feed bodies.

## Security defences (inbound)

Pulling someone else's feed has a threat model. SPEC-038 §Threat Model documents 22 attacks (T1..T22); the implementation addresses each:

- **SSRF (T1, T5)** — RFC 1918 / link-local / loopback / RFC 6598 / multicast / reserved IPs rejected at every redirect hop. `file://` and `data:` URL schemes refused.
- **XXE (T3)** — DOCTYPE declarations and external entity references rejected before parse.
- **Decompression bombs (T6)** — body capped at 1 MiB regardless of `Content-Length`.
- **Privacy leaks (T7)** — minimal `User-Agent: zetl/<version>`; no `Referer`.
- **GUID mutation dedup (T8)** — first-seen identity record over GUID + canonical link + content fingerprint; flipping any one signal still matches.
- **Credentials in config (T17)** — leak-scan refuses any of 10 secret-shaped keys in `.zetl/config.toml`.
- **Cross-origin credential leakage (T18)** — Authorization header AND query-param tokens dropped on cross-origin redirect.
- **Token in URL log lines (T19)** — `redact_url_for_log` replaces secret-shaped query params with `<redacted>` before any log write.
- **401 lockout (T20)** — exactly one re-read + retry; persistent failure suspends the feed.
- **Capability token leakage (T21)** — cohort tokens never appear in feed bodies, build logs, observability labels (`cohort_id` is used instead), or vault-export archives.
- **Forget bypass via re-publish (T22)** — tombstone records block re-import even when the source republishes the same item with a fresh GUID.

For the gory details, see [SPEC-038 §10 Security](https://codeberg.org/anuna/zetl/src/branch/main/specs/SPEC-038-rss-support.md).

## See also

- [[Static Site Export]] — `zetl build` overview
- [[Web Server]] — `zetl serve` for local + collab modes
- [[Capability URLs]] — SPEC-034 cap-protected pages (which inform cohort-feed exclusion)
- [SPEC-038](https://codeberg.org/anuna/zetl/src/branch/main/specs/SPEC-038-rss-support.md) — the canonical implementation spec
- [plans/IMPL-038-wires.spl](https://codeberg.org/anuna/zetl/src/branch/main/plans/IMPL-038-wires.spl) — open follow-up work (inbound HTTP transport, full pull/list/status wiring)
