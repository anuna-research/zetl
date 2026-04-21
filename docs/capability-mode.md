# Capability Mode — Operator Guide

This is the **task-oriented operator guide** for `zetl build
--capability`. It walks through the workflow an operator runs end to
end: generating secrets, authoring config, issuing and revoking
invites, rotating keys, deploying to a static host, and triaging
reader-side errors.

The normative source is `specs/SPEC-034.md`. The long-form security
reference is `docs/capability-security.md` (threat model, quantitative
bounds, acknowledged residuals). The signing-key lifecycle has its
own reference in `docs/signing.md`. The reader-facing error catalogue
is `docs/reader-troubleshooting.md`. This file links out to those
where it would otherwise duplicate them.

If any statement here conflicts with SPEC-034, the spec governs. File
a PR updating this document; do not depart from the spec.

## Contents

- [1. Threat model (SPEC-034 §11)](#1-threat-model-spec-034-11)
- [2. Per-cohort mode selection](#2-per-cohort-mode-selection)
- [3. Quickstart — from zero to deployed](#3-quickstart--from-zero-to-deployed)
- [4. Grants lifecycle](#4-grants-lifecycle)
- [5. Deploy recipes](#5-deploy-recipes)
- [6. Troubleshooting — "my reader can't decrypt"](#6-troubleshooting--my-reader-cant-decrypt)
- [7. Cross-references](#7-cross-references)

---

## 1. Threat model (SPEC-034 §11)

This section reproduces SPEC-034 §11 so the operator has the
authoritative threat model in one place during incident response. The
spec is normative — if this text drifts from §11, trust the spec.

### 1.1 Adversaries

Seven adversaries are in scope (from §11.1):

| #  | Adversary                   | Concrete example                                                                            |
| -- | --------------------------- | ------------------------------------------------------------------------------------------- |
| A1 | Passive web observer        | Safe Browsing servers, link unfurl bots, browser sync sinks, translation APIs.              |
| A2 | Ciphertext holder           | Anyone who can fetch `/c/<path-cap>/<slug>.html`: search engines, CDN employees.            |
| A3 | CDN-compromised             | Attacker who substitutes bytes at the CDN layer — stolen credentials, rogue edge worker.   |
| A4 | Authenticator thief         | Physical theft of an unlocked device with a paired authenticator.                           |
| A5 | Malicious contributor       | PR author injecting XSS, exfiltrating CSS, or hostile markdown.                             |
| A6 | Compromised CI              | Attacker controlling the build: leaked `ZETL_CAP_SECRET`, leaked `ZETL_CAP_SIGNING_KEY`.    |
| A7 | Signing-key compromiser     | Attacker who extracts `ZETL_CAP_SIGNING_KEY` from CI, ops machine, or backup.               |

### 1.2 Attack/mitigation matrix

Reproduced from SPEC-034 §11.1:

| Attack                                            | Attacker             | Mitigation                                                                                  |
| ------------------------------------------------- | -------------------- | ------------------------------------------------------------------------------------------- |
| URL harvested (delegated-URL)                     | A1                   | **Acknowledged residual** (BUG-002). Opt-in split-key (REQ-3430). Invite expiry (NFR-3412). |
| URL harvested (hardened)                          | A1                   | Immune.                                                                                     |
| Pre-first-click URL interception                  | A1                   | Bounded by expiry + finalisation (scope honestly stated in ADR-3411).                       |
| CDN substitutes attacker-written ciphertext       | A3                   | **Blocked:** Ed25519 signature verification (REQ-3427, ADR-3412).                           |
| CDN serves stale/unsigned content                 | A3                   | Signature verification; no signature → no render.                                           |
| Ciphertext decryption without authenticator       | A1, A2, A3           | 2¹²⁸ security.                                                                              |
| Cross-cohort pubkey linkage (hardened)            | A1, A2               | **Fixed:** per-cohort PRF salt (REQ-3414, ADR-3414).                                        |
| Cross-cohort pubkey linkage (delegated-URL)       | A1, A2               | Naturally mitigated (per-grant keypairs).                                                   |
| Recipient-count inference (outsider)              | A1, A2               | X25519 padding to tier (REQ-3422, ADR-3413). Structurally indistinguishable.                |
| Recipient-count inference (insider)               | cohort member        | Acknowledged: reveals "at most (tier - 1)" other recipients. Not claimed to hide.           |
| Referer leak of fragment                          | A1                   | `rel="noopener noreferrer"` + `Referrer-Policy: no-referrer` + fragment scrubbing.          |
| URL forwarding produces indistinguishable reader  | A1                   | Inherent to bearer-capability invite model; documented (§11.2).                             |
| Pre-registered ServiceWorker intercepts           | A3, prior compromise | **Fixed:** shim purges SWs on load (REQ-3428).                                              |
| Concurrent-tab TOFU race                          | —                    | **Fixed:** `navigator.locks` (REQ-3429).                                                    |
| Revoked reader retains past access                | —                    | Explicit non-goal (NFR-3414); forward secrecy not provided.                                 |
| Signing-key compromise                            | A6, A7               | Rotate via `zetl cap rotate-signing-key`; rebuild; cache-invalidate shim.                   |
| `ZETL_CAP_SECRET` compromise                      | A6                   | Rotate secret; rotate all cohorts; rebuild; re-issue all URLs; re-enrol.                    |
| Malicious PR                                      | A5                   | Sanitiser (ammonia) + CSP + audit-diff corpus (REQ-3424).                                   |
| URL leak via shortener / preview bot              | A1                   | **Fixed:** CLI warning on every `zetl cap invite` (REQ-3410).                               |

### 1.3 Acknowledged residual exposures

From §11.2. These are **not** mitigated; operators for whom any of
them is unacceptable must not use capability mode for that content.

- **Fragment leak during TOFU window** (BUG-002). `history.replaceState`
  is best-effort; browser sync, extensions, history-DB, and OS-level
  URL capture APIs may observe the fragment before scrub. Mitigations:
  short invite expiry, split-key mode (opt-in). For content where this
  is unacceptable, use **hardened mode**.
- **Insider recipient-count inference.** A cohort member can subtract
  their own entry to constrain the real count. Not hidden.
- **Multi-cohort delegated-URL pubkey linkage via operator data.**
  Operators with access to `grants.toml` see which reader is in which
  cohort. Inherent; documented.
- **Forward secrecy not provided.** Revoked readers retain past
  decryption capability. Explicit non-goal.
- **URL forwarding.** Invite URL is a bearer capability; anyone with
  it (during the window) can TOFU-bind.
- **Path-cap probing.** Liveness enumeration is observable. Bounded
  by cohort rotation cadence.
- **CDN access logs.** Operator-configured.
- **Traffic-size analysis.** Plaintext page size leaks via ciphertext
  size.
- **Authenticator-level loss-of-custody.** Standard WebAuthn
  recovery; spec adds no controls.
- **Timing side-channel in decrypt.** Page-size-dependent; not
  defended.
- **Link shorteners / preview bots.** REQ-3410 CLI warning; operator
  obligation.

### 1.4 Incident-response playbooks

From §11.3. `zetl cap emergency-shutdown` prints a live checklist at
invocation time; the table is the reference snapshot:

| Incident                                  | Response                                                                                                                           |
| ----------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| Reader leaves                             | `zetl cap revoke <grant-id>` → rebuild → deploy. Latency ≤ NFR-3409 (rebuild + `max-age`, default ≤ 1 h).                           |
| Authenticator compromised                 | Rotate affected cohort (`zetl cap rotate --cohort <id>`); redistribute entry URLs.                                                 |
| Invite URL leaked pre-TOFU, within expiry | `zetl cap revoke <grant-id>` + re-invite on a fresh URL. Old URL becomes inert on next rebuild.                                    |
| Invite URL leaked post-expiry             | Already inert. Monitor for anomalous access patterns.                                                                              |
| Signing-key compromised                   | `zetl cap rotate-signing-key` → rebuild all pages → deploy → invalidate `/assets/shim.js` cache at CDN. Monitor OBS-3413.          |
| `ZETL_CAP_SECRET` compromised             | `zetl cap genkey` → rotate ALL cohorts → rebuild → re-issue all URLs → readers re-TOFU per device. Effectively a full re-onboard.   |
| Malicious PR landed                       | Revert on main → rebuild → `zetl cap audit-diff` across exposure window → consider sanitiser allowlist tightening.                 |
| Emergency shutdown                        | `zetl cap emergency-shutdown` → follow printed checklist (DNS, CDN purge, secret rotation, reader notification).                   |

See `docs/capability-security.md` for long-form discussion of the
threat model, the per-attack mitigation surface, and the quantitative
bounds (padding, brute-force, path-cap entropy). See `docs/signing.md`
for the signing-key rotation playbook.

---

## 2. Per-cohort mode selection

Capability mode ships **two cohort modes** and one **opt-in variant**.
Picking a mode is a **per-cohort** decision — a vault may mix
delegated-URL and hardened cohorts freely.

### 2.1 Decision aid

Use this table to pick a mode for each cohort.

| You want to…                                                           | Delegated-URL (default) | Hardened  | Delegated-URL + split-key |
| ---------------------------------------------------------------------- | ----------------------- | --------- | ------------------------- |
| Invite readers with a single URL they can click                         | **Yes**                 | No (they must enrol + send you a pubkey) | Yes, plus second factor   |
| Be immune to URL-harvester channels (Safe Browsing, extensions, sync)   | No (residual)           | **Yes**   | Largely (both halves must leak) |
| Support non-technical readers who won't follow an enrolment procedure   | **Yes**                 | Marginal  | Yes, but one extra step   |
| Hide cross-cohort membership from outsiders holding multi-cohort ciphertexts | Yes (per-grant keypair) | **Yes** (per-cohort PRF salt) | Yes                    |
| Avoid per-cohort enrolment friction for readers in many cohorts         | **Yes**                 | No (one enrolment per cohort) | **Yes**               |
| Avoid giving readers bookmarks that contain decryption material         | No                      | **Yes**   | No (bookmark still has half1) |
| Operate on the least infrastructure (no out-of-band pubkey channel)     | **Yes**                 | No        | Yes (but needs second-factor channel) |

### 2.2 Picking in practice

- **Internal engineering wiki, shared on org channels.** Delegated-URL
  (default). The residual is bounded by `NFR-3412` (invite expiry) and
  org trust in direct-message / email channels.
- **Partner / customer documentation, mixed trust.** Hardened mode for
  the sensitive cohort; delegated-URL for the rest. The enrolment
  friction is worth paying for the URL-harvest immunity.
- **Bounty / compensation / course material exposed to preview
  bots.** Split-key mode (REQ-3430) on top of delegated-URL, or
  hardened mode. Assume URLs will leak; design so that alone does not
  grant access.
- **Content you cannot afford to sign with a long-lived key.** Do not
  use capability mode. Use `zetl serve` (SPEC-020) or reverse-proxy
  auth. See `docs/capability-security.md` §2 for the full "don't use
  capability mode for" table.

### 2.3 Configuring mode per cohort

Cohort mode is set in `recipients.toml` (CON-3403), not in
`.zetl/config.toml`:

```toml
# recipients.toml
version = 1

[vault]
signing_pubkey = "ed25519:<base64url-pubkey>"   # written by zetl cap genkey

[[cohort]]
id      = "engineering"
name    = "Engineering Team"
mode    = "delegated-url"                        # default
pubkeys = []                                     # delegated-URL grants populate this implicitly

[[cohort]]
id      = "partners"
name    = "Partner Read-Only"
mode    = "webauthn-prf"                         # hardened
pubkeys = [
  "age-recipient-v1:<base64url-X25519-pubkey>",   # from each partner's enrolment
]
```

Split-key mode is an **invite-time** choice, not a cohort-mode choice:

```toml
# .zetl/config.toml
[access.split_key]
enabled       = true                             # opt-in (REQ-3430)
second_factor = "spoken-phrase"                  # or "qr"
```

With `enabled = true`, `zetl cap invite --split-key` splits the
delegated-URL fragment into two halves; the URL carries `#k1=<half1>`
and the second factor is conveyed separately (spoken phrase or QR
code). See SPEC-034 REQ-3430 / ADR-3415.

---

## 3. Quickstart — from zero to deployed

This walkthrough takes a vault from no-capability-mode to a first
deployed build. Replace `wiki.example` with your hostname throughout.

### 3.1 Generate the two secrets (`zetl cap genkey`)

```bash
$ zetl cap genkey
# ZETL_CAP_SECRET — 48-byte content-encryption secret
export ZETL_CAP_SECRET='<base64url-secret>'

# ZETL_CAP_SIGNING_KEY — Ed25519 vault-signing private key
export ZETL_CAP_SIGNING_KEY='<base64-standard-32-bytes>'

# recipients.toml[vault].signing_pubkey has been updated:
#   ed25519:<base64url-pubkey>
```

Both values print to stdout **exactly once**. Store them in a
password manager immediately; `zetl cap genkey` does not persist
either secret.

`recipients.toml` is modified in place to carry the vault-signing
public key. Commit that change — the pubkey travels in version
control alongside the content it authenticates.

### 3.2 Author `.zetl/config.toml`

Minimum viable capability-mode config:

```toml
# .zetl/config.toml

[access]
mode = "capability"                              # activates capability build

[access.cache]
max_age = 300                                    # /c/* Cache-Control max-age (NFR-3409)
                                                 # bounds [60, 3600]; default 300

[access.signing]
key_env   = "ZETL_CAP_SIGNING_KEY"               # where the private signing key comes from
algorithm = "ed25519"                            # fixed in v0.4.0

[access.split_key]
enabled       = false                            # opt-in (REQ-3430)
second_factor = "spoken-phrase"                  # unused until enabled = true

[access.sw_hygiene]
unregister_all           = true                  # default — purge SWs on load
clear_site_data_on_enrol = true                  # default — emit Clear-Site-Data on /enroll.html

[access.search]
mode = "off"                                     # default; "per-cohort" opt-in

[access.backlinks]
mode = "scoped"                                  # default; "global" is rejected in cap builds
```

Author `recipients.toml` (per §2.3) with at least one cohort. An
empty `pubkeys = []` is fine for a delegated-URL cohort — `zetl cap
invite` populates grants as you issue them.

### 3.3 First invite, then first build

Issue a grant for the first reader:

```bash
$ zetl cap invite alice \
    --cohort engineering \
    --expires 7d \
    --site-url https://wiki.example

⚠  SECURITY WARNING (REQ-3412 / §11.2)
   Invite URLs contain decryption material in the fragment.
   Do NOT pass through URL shorteners, link previewers, or bots
   (Slack unfurl, Microsoft SafeLinks, Google unfurl, etc.).
   Send via channels that do not rewrite URLs:
     direct messages, email without link rewriting, in-person.

grant-id:  g_01JABC...
mode:      delegated-url
cohort:    engineering
expires:   2026-04-28T14:22:00Z
url:       https://wiki.example/c/<path-cap>/welcome.html#k=<43-char-fragment>
```

Now build:

```bash
$ zetl build --capability

[zetl] capability build:
  cohorts:       1 (engineering)
  pages:         34 signed + encrypted
  shim:          dist/assets/shim.js          (SRI sha384-…)
  signing-pub:   ed25519:<base64url-pubkey>
  deploy recipes: dist/_zetl/deploy/          (nginx/Caddy/Netlify/Vercel)
  tombstones:    0
```

`dist/` now contains:

- `dist/c/<path-cap>/<slug>.html` — one **signed + encrypted** envelope
  per page per cohort (CON-3404).
- `dist/assets/shim.js` — content-hashed shim bundle with the
  vault-signing pubkey embedded and an SRI tag the shell points at
  (REQ-3421).
- `dist/enroll.html` — hardened-mode self-enrolment page. Emitted even
  if no hardened cohorts exist; readers that land there by accident
  hit the `err-need-invite` page.
- `dist/_headers`, `dist/_redirects`, `dist/vercel.json` — root-level
  deploy artifacts consumed by Netlify / Cloudflare Pages / Vercel
  verbatim.
- `dist/_zetl/deploy/*` — copy-paste recipes for nginx and Caddy plus
  the Netlify/Vercel recipes as standalone snippets for merging.
- `dist/_zetl/_gone.map` — nginx-consumable tombstone map for retired
  path-caps.

### 3.4 Deploy

Point your static host at `dist/`. The deploy recipe that matches
your platform lives at `dist/_zetl/deploy/` — see §5 for copy-paste
blocks. The **four headers** a capability-mode deployment must emit
are:

| Path                 | Header                    | Value                                                       |
| -------------------- | ------------------------- | ----------------------------------------------------------- |
| `/c/*`               | `Cache-Control`           | `private, max-age=300, must-revalidate` (operator-tunable)  |
| `/c/*`               | `Content-Security-Policy` | (CON-3410 directive; see below)                             |
| `/enroll.html`       | `Clear-Site-Data`         | `"cache", "storage", "executionContexts"`                   |
| `/enroll.html`       | `Content-Security-Policy` | (same as `/c/*`)                                            |
| `/logout`            | `Clear-Site-Data`         | `"cache", "storage", "executionContexts"`                   |
| `/assets/shim.js`    | `Cache-Control`           | `public, max-age=31536000, immutable`                       |

CSP (CON-3410):

```
default-src 'none'; script-src 'self'; style-src 'self'; \
img-src 'self' data:; connect-src 'self'; font-src 'self'; \
frame-ancestors 'none'; base-uri 'none'; form-action 'none'; \
require-trusted-types-for 'script'; trusted-types zetl-cap;
```

The HTML shell carries the same CSP as a `<meta http-equiv>` tag, so
a CDN that drops the HTTP header still enforces CSP in the browser.

### 3.5 Send the invite

Using the URL printed by `zetl cap invite`:

- **Direct message / email without link rewriting.** Best channel for
  delegated-URL mode.
- **In person / voice call.** Trivially free of URL rewriting.
- **Never:** Slack public channels (unfurl rewrites), SMS (carriers
  sometimes rewrite), any link shortener, calendar invites (preview
  bots).

The reader opens the full URL in a PRF-capable browser, taps their
authenticator, and the shim TOFU-binds to that passkey on that
device. See `docs/reader-troubleshooting.md` for the reader-side
error catalogue.

### 3.6 Optional: sanity-check before exposing to real readers

```bash
$ zetl cap list                                # who has access
$ zetl cap check                               # stale-grant + public-safety audit
$ zetl cap audit-diff main HEAD                # malicious-content scan on the diff
```

`zetl cap check` exits non-zero if any grant has expired since the
last build or if a public cohort is missing a required guardrail.
Wire it into CI.

---

## 4. Grants lifecycle

Every operational change in capability mode is one of: **issue**,
**revoke**, **rotate**, **finalise**, or **rotate the signing key**.
This section walks through each.

### 4.1 Issue a grant (`zetl cap invite`)

```bash
# Delegated-URL (default)
zetl cap invite alice --cohort eng --site-url https://wiki.example

# With scope + expiry
zetl cap invite bob --cohort ops \
    --expires 14d \
    --pages 'runbooks/*' \
    --site-url https://wiki.example

# Hardened mode (reader enrols first, sends you their pubkey)
zetl cap invite carol --cohort partners \
    --recipient age-recipient-v1:<base64url-pubkey>

# Print the reader's /enroll.html URL instead (hardened handoff)
zetl cap invite dan --cohort partners \
    --via enrol-page \
    --site-url https://wiki.example

# Split-key mode (requires [access.split_key] enabled = true)
zetl cap invite eve --cohort eng --split-key \
    --site-url https://wiki.example
```

The invite writes to `grants.toml` (CON-3402) and, for delegated-URL,
prints the reader's URL. Re-running the build regenerates signatures
and ciphertexts so the new grant's pubkey lands in the cohort's
recipient list.

### 4.2 Revoke a grant (`zetl cap revoke`)

```bash
$ zetl cap revoke g_01JABC...
revoked: g_01JABC... (engineering, alice)
next: rebuild + deploy to flush the reader's entry from cohort ciphertexts.
```

Revocation takes effect on the **next build + deploy**. The latency
bound is `rebuild_time + cache_max_age` — default ≤ 1 hour
(NFR-3409). Lower `[access.cache] max_age` tightens it at the cost
of more CDN round-trips.

Revocation is **not** forward-secret: the revoked reader retains
every ciphertext they already downloaded. See §1.3.

### 4.3 Rotate a cohort (`zetl cap rotate`)

```bash
$ zetl cap rotate --cohort engineering
rotated content-key salt for cohort: engineering.
URLs remain stable across rotation (REQ-3402).
next: rebuild to re-encrypt every page in this cohort under the new content key.
```

Rotation changes the **content-encryption key** salt for a cohort
without changing the **path-cap** salt — existing URLs remain valid
bookmarks, but every ciphertext under them is re-encrypted under a
new derived key (BUG-023 resolution). Schedule rotations ≥ every 180
days (NFR-3411) or after an incident (stolen authenticator,
suspicious access).

### 4.4 Finalise a grant (`zetl cap finalise`)

```bash
# Operator has confirmed out-of-band that alice has TOFU-bound on her
# laptop and phone; mark the grant as bound.
$ zetl cap finalise g_01JABC...
grant g_01JABC... marked bound=true (REQ-3408).

# Optional: reissue the delegated private key at finalisation time.
$ zetl cap finalise g_01JABC... --rotate-grant
grant g_01JABC... reissued with a fresh priv_A.
next: send the new invite URL through the same channel.
```

Finalisation **does not** cryptographically prove TOFU completion;
it records that the operator has confirmed it out-of-band (ADR-3411).
It is an operational tool, not a security control. If the original
invite channel is still compromised, the finalisation URL is too —
revoke and re-invite via a different channel instead.

### 4.5 Rotate the vault-signing key (`zetl cap rotate-signing-key`)

Signing-key rotation is the high-stakes operation in this lifecycle.
It is the only one where a mis-sequenced step locks readers out.
**See `docs/signing.md` for the full reference;** the summary here is
a cheat sheet:

```bash
# 1. Generate a new Ed25519 keypair. recipients.toml[vault].signing_pubkey
#    is updated in place. Capture stdout for the new private key.
zetl cap rotate-signing-key > new-signing-key.txt
source new-signing-key.txt        # or paste into password manager
shred -u new-signing-key.txt

# 2. Rebuild: every page is re-signed; a new shim bundle is emitted
#    with the new embedded pubkey (new SRI hash).
zetl build --capability

# 3. Deploy. Critical ordering: INVALIDATE /assets/shim.js at the CDN
#    BEFORE the new ciphertexts reach readers. Otherwise a reader with
#    a cached OLD shim rejects new ciphertexts as signature-failed.
cdn-purge /assets/shim.js
deploy.sh dist/
cdn-purge '/c/*'                  # optional; SRI forces shim re-fetch

# 4. Monitor OBS-3413 (signature-failure counter) for a spike.
```

Cache-invalidation order is **load-bearing**. A rotation transition
window in which some readers hold the old shim and others the new is
unavoidable in v1 — the dual-pubkey-slot scheme is SPEC-034 §14 OQ-5,
deferred. See `docs/signing.md` §3.

### 4.6 Other lifecycle verbs

| Verb                            | Purpose                                                                                              |
| ------------------------------- | ---------------------------------------------------------------------------------------------------- |
| `zetl cap list`                 | Enumerate grants; `--cohort <id>` to filter; `--output json` for scripting.                          |
| `zetl cap sweep`                | Mark past-expiry grants revoked in place. Wire into a cron, not into the build.                      |
| `zetl cap check`                | Stale-grant + public-safety audit. Exits non-zero on drift. Wire into CI.                            |
| `zetl cap pair`                 | SPAKE2-authenticated pubkey handoff between two operators (hardened-mode pubkey exchange).           |
| `zetl cap audit-diff OLD NEW`   | Malicious-content scan against the REQ-3424 corpus. Wire into the PR gate.                           |
| `zetl cap emergency-shutdown`   | Print the operator runbook for taking the wiki offline at the host level. Documentation-only.        |

---

## 5. Deploy recipes

`zetl build --capability` writes deploy artifacts under
`dist/_zetl/deploy/` and — for platforms that consume them verbatim
— at `dist/` root (`_headers`, `_redirects`, `vercel.json`). The
recipes below are the copy-paste versions. Values assume
`[access.cache] max_age = 300`; override with your own if you tuned
it.

The **CSP directive** is the CON-3410 string; it appears verbatim in
every recipe. Copy it exactly — reordering, re-casing, or omitting
directives is a silent weakening.

```
default-src 'none'; script-src 'self'; style-src 'self'; \
img-src 'self' data:; connect-src 'self'; font-src 'self'; \
frame-ancestors 'none'; base-uri 'none'; form-action 'none'; \
require-trusted-types-for 'script'; trusted-types zetl-cap;
```

### 5.1 nginx

Paste inside an existing `server { }` block. Include `_gone.map` in
the enclosing `http { }` context so tombstones flow through.

```nginx
# In your http { } context:
map $uri $zetl_gone {
    include /path/to/dist/_zetl/_gone.map;
}

# In your server { } block:
location ^~ /c/ {
    if ($zetl_gone = 1) { return 410; }
    add_header Cache-Control "private, max-age=300, must-revalidate" always;
    add_header Content-Security-Policy "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; font-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'; require-trusted-types-for 'script'; trusted-types zetl-cap;" always;
}

location = /enroll.html {
    add_header Clear-Site-Data '"cache", "storage", "executionContexts"' always;
    add_header Content-Security-Policy "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; font-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'; require-trusted-types-for 'script'; trusted-types zetl-cap;" always;
}

location = /logout {
    add_header Clear-Site-Data '"cache", "storage", "executionContexts"' always;
}

location = /assets/shim.js {
    add_header Cache-Control "public, max-age=31536000, immutable" always;
}
```

The `always` keyword makes nginx emit these headers on non-2xx
responses too — otherwise a 404 on `/c/*.html` would skip the
Cache-Control header and a revoked page could linger longer than
expected.

### 5.2 Caddy

Paste inside your site block.

```caddy
@zetl_cap path /c/*
header @zetl_cap Cache-Control "private, max-age=300, must-revalidate"
header @zetl_cap Content-Security-Policy "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; font-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'; require-trusted-types-for 'script'; trusted-types zetl-cap;"

@zetl_csd_0 path /enroll.html
header @zetl_csd_0 Clear-Site-Data `"cache", "storage", "executionContexts"`
header @zetl_csd_0 Content-Security-Policy "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; font-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'; require-trusted-types-for 'script'; trusted-types zetl-cap;"

@zetl_csd_1 path /logout
header @zetl_csd_1 Clear-Site-Data `"cache", "storage", "executionContexts"`

@zetl_shim path /assets/shim.js
header @zetl_shim Cache-Control "public, max-age=31536000, immutable"
```

The backtick wrappers around the `Clear-Site-Data` value keep Caddy's
tokenizer from consuming the inner double quotes.

### 5.3 Netlify

Netlify reads `_headers` and `_redirects` at the site root. Both are
written by `zetl build --capability`; deploy the `dist/` tree as-is
and Netlify picks them up. The contents are reproduced here for
operators who already maintain these files and want to merge by hand.

`_headers`:

```
/c/*
  Cache-Control: private, max-age=300, must-revalidate
  Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; font-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'; require-trusted-types-for 'script'; trusted-types zetl-cap;

/enroll.html
  Clear-Site-Data: "cache", "storage", "executionContexts"
  Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; font-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'; require-trusted-types-for 'script'; trusted-types zetl-cap;

/logout
  Clear-Site-Data: "cache", "storage", "executionContexts"

/assets/shim.js
  Cache-Control: public, max-age=31536000, immutable
```

`_redirects` (carries tombstones; the one shipped in `dist/` is
authoritative — this is a template):

```
# /c/<retired-path-cap>/<slug>.html  410!
```

### 5.4 Cloudflare Pages

Cloudflare Pages consumes the Netlify `_headers` + `_redirects`
format verbatim; the `dist/` tree deploys directly with no
additional config. Use the Netlify block above.

For **Workers / Page Rules** deployments (non-Pages Cloudflare),
translate the header rules into your response-header config. The CSP
directive, Cache-Control value, and Clear-Site-Data token list must
match byte-for-byte.

### 5.5 Vercel

`vercel.json` is written at `dist/` root and ships verbatim. The
contents for hand-merging:

```json
{
  "headers": [
    {
      "source": "/c/(.*)",
      "headers": [
        { "key": "Cache-Control", "value": "private, max-age=300, must-revalidate" },
        { "key": "Content-Security-Policy", "value": "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; font-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'; require-trusted-types-for 'script'; trusted-types zetl-cap;" }
      ]
    },
    {
      "source": "/enroll.html",
      "headers": [
        { "key": "Clear-Site-Data", "value": "\"cache\", \"storage\", \"executionContexts\"" },
        { "key": "Content-Security-Policy", "value": "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; font-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'; require-trusted-types-for 'script'; trusted-types zetl-cap;" }
      ]
    },
    {
      "source": "/logout",
      "headers": [
        { "key": "Clear-Site-Data", "value": "\"cache\", \"storage\", \"executionContexts\"" }
      ]
    },
    {
      "source": "/assets/shim.js",
      "headers": [
        { "key": "Cache-Control", "value": "public, max-age=31536000, immutable" }
      ]
    }
  ]
}
```

### 5.6 S3 + CloudFront

S3 + CloudFront does not auto-generate from `dist/` — wire the
headers via a CloudFront **Response Headers Policy** and configure
`/c/*` / `/enroll.html` / `/logout` / `/assets/shim.js` as separate
**cache behaviours** that attach the policy.

`aws` CLI sketch (replace distribution IDs + ARNs to suit):

```bash
# 1. Create the response-headers policy. Split CSP, Cache-Control,
#    and Clear-Site-Data into three policies (CloudFront attaches
#    one per cache behaviour).

aws cloudfront create-response-headers-policy \
  --response-headers-policy-config file://zetl-csp.json
# zetl-csp.json:
# {
#   "Name": "zetl-cap-csp",
#   "SecurityHeadersConfig": {
#     "ContentSecurityPolicy": {
#       "Override": true,
#       "ContentSecurityPolicy": "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; font-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'; require-trusted-types-for 'script'; trusted-types zetl-cap;"
#     }
#   },
#   "CustomHeadersConfig": {
#     "Quantity": 0
#   }
# }

aws cloudfront create-response-headers-policy \
  --response-headers-policy-config file://zetl-cap-cache.json
# zetl-cap-cache.json — Cache-Control for /c/*:
# { "Name": "zetl-cap-cache",
#   "CustomHeadersConfig": {
#     "Quantity": 1,
#     "Items": [{
#       "Header": "Cache-Control",
#       "Value": "private, max-age=300, must-revalidate",
#       "Override": true
#     }]
#   }
# }

aws cloudfront create-response-headers-policy \
  --response-headers-policy-config file://zetl-csd.json
# zetl-csd.json — Clear-Site-Data for /enroll.html + /logout:
# { "Name": "zetl-cap-csd",
#   "CustomHeadersConfig": {
#     "Quantity": 1,
#     "Items": [{
#       "Header": "Clear-Site-Data",
#       "Value": "\"cache\", \"storage\", \"executionContexts\"",
#       "Override": true
#     }]
#   }
# }

# 2. Attach one policy per cache behaviour.
#    /c/*                 → zetl-cap-cache + zetl-csp
#    /enroll.html         → zetl-csd + zetl-csp
#    /logout              → zetl-csd
#    /assets/shim.js      → (built-in CachingOptimized with max-age override)
#
# The Management Console path is simpler than the CLI for attaching;
# see the CloudFront "Behaviors" tab on your distribution.

# 3. Sync the dist tree to S3 (not `sync --delete` — deploy atomically
#    by uploading new ciphertexts BEFORE invalidating).
aws s3 sync dist/ s3://<bucket>/ --cache-control 'private, max-age=300, must-revalidate'

# 4. Invalidate the shim first, then the ciphertext tree.
aws cloudfront create-invalidation \
  --distribution-id <id> \
  --paths '/assets/shim.js'
aws cloudfront create-invalidation \
  --distribution-id <id> \
  --paths '/c/*'
```

Notes:

- **Origin shield.** If enabled, invalidation hits the origin shield
  too. If not, inspect edge cache hit rates post-rotation to confirm
  propagation.
- **Signed URLs / OAC.** Capability mode's access control is in the
  ciphertext; **do not** layer CloudFront signed URLs or Origin
  Access Control on top. Both add operational complexity and change
  the reader's mental model without improving security.
- **CloudFront Functions.** If you already run a CF Function, make
  sure it does not drop the headers policy is attaching. CF Functions
  run after cache-behaviour headers but can override them.

---

## 6. Troubleshooting — "my reader can't decrypt"

A reader reports an error. This section is the operator-side
diagnostic tree. The reader-facing catalogue is
`docs/reader-troubleshooting.md` — always ask the reader for the
**exact error kind** (the slug on the error page, e.g.
`err-signature-failed`) before triaging.

### 6.1 Diagnostic flowchart

```
  Reader reports: "I can't read the wiki."
        │
        ▼
  [Ask] "What exactly did you see?"
  (The red banner text or the slug after `err-` on the error page.)
        │
        ├── "err-signature-failed"  ───►  §6.2  Signature verification
        │
        ├── "err-need-invite"       ───►  §6.3  No binding on this device
        │
        ├── "err-identity-unavailable" ──►  §6.4  Binding lost
        │
        ├── "err-tofu-failed"       ───►  §6.5  TOFU binding failed
        │
        ├── "err-decrypt-failed"    ───►  §6.6  Decryption failed
        │                                       (most common: revoked or rotated)
        │
        ├── "err-envelope-malformed" ──►  §6.7  Envelope parse failed
        │
        ├── "err-sw-purge-failed"   ───►  §6.8  ServiceWorker stuck
        │
        ├── "err-lock-unavailable"  ───►  §6.9  Browser too old
        │
        ├── "err-host-missing"      ───►  §6.10 Build / deploy artefact missing
        │
        └── "fallback-prf-unavailable"
             (banner, not error)    ───►  §6.11 Graceful PRF fallback — info only
```

### 6.2 `err-signature-failed` — "This page's signature did not verify"

**The most safety-critical error.** The shim refuses to decrypt. Two
causes dominate in practice:

1. **Recent signing-key rotation without CDN cache flush.** A reader's
   browser holds the **old** shim; the CDN serves **new** signed
   pages; signatures don't verify. This is the operator's most common
   self-inflicted path.
2. **Actual CDN substitution or MITM.** An attacker has inserted
   bytes. Rare, but the error is designed to surface it.

**Operator actions.**

1. Check `OBS-3413` (signature-failure counter). One reader in
   isolation: probably (1). Broad spike: also probably (1), but
   verify.
2. If you rotated the signing key in the last hour:
   - Confirm `/assets/shim.js` has been invalidated at the CDN.
   - Force-re-invalidate: `cdn-purge /assets/shim.js` again.
   - Ask affected readers to hard-refresh (Ctrl/Cmd-Shift-R).
3. If you have **not** rotated:
   - Treat as a potential incident. Follow `docs/signing.md` §5
     (threat-model recap).
   - Consider `zetl cap emergency-shutdown` while you investigate.
4. Confirm recovery by asking the reader to open the page in a
   private/incognito window. Private windows bypass local shim
   caches; if the page renders there, the fix is cache-side on the
   reader's machine.

### 6.3 `err-need-invite` — "Readable only from a fresh invite URL"

The browser has no binding and the URL carries no fragment. Either:

- the reader opened a bookmark to the "clean" URL (post-`replaceState`)
  on a **new device or new browser profile**, or
- a second device tried to re-use an invite URL that a first device
  already consumed (delegated-URL grants are single-device-first-use).

**Operator action.** Issue a fresh invite (`zetl cap invite`) for
the reader and the new device. Do not try to reuse the original
URL.

### 6.4 `err-identity-unavailable` — "Could not recover reading identity"

Passkey is present but the wrapped reading key has been evicted from
IndexedDB. Common causes on the reader's side: cleared site data,
private-window isolation, browser profile reset or re-sync, wrong
browser profile.

**Operator action.** Confirm the reader hasn't simply swapped
profiles (`err-identity-unavailable` in a private window is
expected). Otherwise, issue a fresh invite — the stored binding is
gone and only re-TOFU rebuilds it.

### 6.5 `err-tofu-failed` — "Could not bind this device to the cohort passkey"

Usually one of:

- **Prompt dismissed or biometric failed.** Ask the reader to reload
  and approve the passkey prompt.
- **PRF extension not supported.** Older FIDO2 hardware keys,
  enterprise-locked browsers, very old browser builds.

**Operator action.** Ask which browser + version and which
authenticator (platform vs. hardware key, make/model). If PRF is
unavailable on their setup, switch them to **hardened mode** on a
browser that does support PRF, or offer a different access path
(`zetl serve`, VPN).

### 6.6 `err-decrypt-failed` — "Could not decrypt this page"

Signature verified; the reader's key does not open the ciphertext.
**This is the signal that the reader is no longer in the cohort's
recipient list.** Causes:

1. The reader has been **revoked** (someone ran `zetl cap revoke`).
2. The cohort has been **rotated** (someone ran `zetl cap rotate
   --cohort <id>`) and the reader's grant was not rebuilt into the
   new recipient list.
3. The reader enrolled in cohort A but is trying to read a page that
   belongs to cohort B (multi-cohort vault; reader only has a grant
   for one).

**Operator action.**

1. `zetl cap list --cohort <id>` — is the reader still in the
   cohort's grants?
2. `git log -- grants.toml` — was the cohort rotated or their grant
   revoked?
3. If they should still have access, rebuild + deploy. If the build
   was recent and still excludes them, re-issue the invite.
4. If they shouldn't — fine; confirm with them that access was
   intentionally removed.

### 6.7 `err-envelope-malformed` — "Envelope malformed, cannot be read"

The file received is not in CON-3404 envelope shape. Transient
deploy artefacts cause this most often: the CDN served a
half-written object during a rolling deploy.

**Operator action.**

1. Wait 60 seconds; ask the reader to reload.
2. Check your deploy pipeline for partial-upload modes. S3 `sync`
   can leave temporary files visible; use atomic CDN flip or
   canaried deploys.
3. If it persists across a fresh deploy, re-run `zetl build
   --capability` and inspect `dist/c/<path-cap>/<slug>.html` locally
   — the first seven lines must be the envelope header (CON-3404).

### 6.8 `err-sw-purge-failed` — "Could not clear stale service workers"

A ServiceWorker from **a previous deployment on the same origin** is
intercepting requests and the current shim cannot unregister it from
one tab alone.

**Operator actions.**

1. Do **not** share the origin with non-capability-mode content
   (REQ-3428 consequence). If `/` served a regular SPA with its own
   SW before you enabled capability mode, the old SW persists in
   readers' browsers.
2. Ask affected readers to close **every** tab for the site, then
   reopen. `Clear-Site-Data` on `/logout` will flush everything
   including the SW.
3. If you must share an origin, gate capability content behind a
   separate subdomain (e.g. `wiki.example` for capability mode,
   `app.example` for the SPA). Cross-subdomain SW scopes are
   separate.

### 6.9 `err-lock-unavailable` — "Browser does not support `navigator.locks`"

Very old browser. Nothing to fix server-side.

**Operator action.** Recommend a supported browser (Chrome / Edge /
Brave / Arc / recent Firefox / Safari 16.4+). If the reader is
locked into a corporate browser build that lacks `navigator.locks`,
offer a different access path.

### 6.10 `err-host-missing` — "Capability-mode mount point missing"

`<main data-zetl-capability>` is not in the HTML shell. Build /
deploy artefact drift.

**Operator actions.**

1. Re-run `zetl build --capability`. Confirm the HTML shell at
   `dist/c/<path-cap>/<slug>.html` contains `<main
   data-zetl-capability>`.
2. If a custom theme rendered the shell, confirm the theme template
   preserves the mount point.
3. Deploy.

### 6.11 `fallback-prf-unavailable` — banner, not error

The reader's browser / authenticator does not expose the WebAuthn
PRF extension; the wiki fell back to **fragment-required mode**
(REQ-3412). The page rendered; the banner warns that every future
visit needs the full invite URL.

**Operator action.** None required. Monitor `OBS-3412` — if the
fallback rate is higher than expected, your cohort mix may include
browsers you didn't plan for.

### 6.12 Diagnostic tools

| Tool                              | When to use                                                                          |
| --------------------------------- | ------------------------------------------------------------------------------------ |
| `zetl cap list`                   | Enumerate grants. First question for any "can't read" report.                         |
| `zetl cap list --cohort <id>`     | Is the reporter in this cohort? Has their grant expired or been revoked?              |
| `zetl cap check`                  | Stale-grant + public-safety audit. Run before deploying.                              |
| `zetl cap audit-diff OLD NEW`     | Malicious-content scan. Run on PRs; essential for `err-signature-failed` triage.      |
| `OBS-3413` (performance timeline) | Signature-failure counter in reader RUM. Spikes after rotation are expected and brief. |
| `OBS-3414`                        | ServiceWorker-purge counter. Persistent non-zero → origin shares with a SPA.          |
| `OBS-3415`                        | `navigator.locks` wait > 100 ms counter. Persistent non-zero → concurrency anomaly.   |

---

## 7. Cross-references

- `specs/SPEC-034.md` — normative spec. §11 (threat model), §12
  (documentation plan), §4 REQ-3416 (CLI), §6 CON-3406 (deploy
  emission), §6 CON-3410 (CSP).
- `docs/capability-security.md` — long-form security model, threat
  model expansion, quantitative bounds (padding, brute-force,
  path-cap entropy), acknowledged residuals.
- `docs/signing.md` — vault-signing-key lifecycle, rotation cadence,
  emergency shutdown.
- `docs/reader-troubleshooting.md` — reader-facing error catalogue
  (the reader sees this; the operator reads alongside it during
  triage).
- `tools/sanitiser-config.toml` — normative HTML denylist
  (REQ-3421).
- `.hence/reviews/cap-tier1-crypto-2026-04-20.md` — Tier 1 crypto
  review.
- `.hence/reviews/cap-tier2-2026-04-20.md` — Tier 2 fresh-context
  review.

---

*If any statement in this document conflicts with SPEC-034, the spec
governs. File a PR; do not depart from the spec.*
