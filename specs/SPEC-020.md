---
title: "SPEC-020: Multi-User Collaborative Editing"
version: 0.1.0
status: draft
date: 2026-03-18
audience: agent, human
parent: SPEC-001
related:
  - SPEC-005  # Defeasible Reasoning (SPL)
  - SPEC-016  # Lifecycle Hooks
  - SPEC-017  # Temporal Graph (History)
  - SPEC-008  # Watch Mode
dependencies:
  - spindle-core (defeasible reasoning for ACL)
  - spindle-parser (SPL parsing for policy documents)
  - webauthn-rs (passkey authentication)
  - bip39 (mnemonic recovery keys)
  - ed25519-dalek (key derivation for agent tokens)
  - git2 (libgit2 bindings for auto-commit)
  - diamond-types (text CRDT backend for collaborative editing; Peritext-style marks layered by ztl)
---

| Field        | Value                                        |
|--------------|----------------------------------------------|
| Document     | SPEC-020                                     |
| Title        | Multi-User Collaborative Editing             |
| Version      | 0.1.0                                        |
| Status       | Draft                                        |
| Author       | Agent (USDD Protocol v1.3.0)                 |
| Date         | 2026-03-18                                   |
| Audience     | agent, human                                 |
| Trace        | USDD §2 (Vision → Specification)             |
| Parent       | SPEC-001                                     |
| Related      | SPEC-005, SPEC-016, SPEC-017, SPEC-008       |
| Feature Gate | `--features collab`                          |

---

## 1. Overview

### 1.1 Problem

ztl serve currently operates as a single-user local wiki. There is no authentication, no access control, and no attribution of edits. To support teams collaborating on a shared vault — or LLM agents acting on behalf of users — the system needs identity, authorization, edit attribution, and agent delegation.

### 1.2 Core Insight

Access control policy belongs *in the wiki itself*, expressed as defeasible logic (SPL). This makes permissions versionable, composable, auditable, and debuggable using the same reasoning engine that already powers `ztl reason`. A page can locally strengthen or weaken its own access without modifying the global policy, and `ztl reason --query` can explain any authorization decision with a full proof chain.

### 1.3 Design Philosophy

- **Identity is cryptographic.** Passkeys for interactive login; BIP39 mnemonics for recovery and agent delegation. No passwords.
- **Authorization is logic.** SPL rules, not config tables. Defeasible so local overrides compose cleanly.
- **Every edit is attributed.** Auto-committed to git with user identity. The vault's git log is the audit trail.
- **Agents are users.** An LLM agent authenticates with a user's derived token and operates under the same ACL. No separate "service account" abstraction.
- **Invitations are capabilities.** A cryptographic token encodes the inviter, the intended role, and an expiry. Accepting an invitation is the only way to create a new account (after the bootstrap user).

### 1.4 Scope

**In scope:**
- Passkey-based authentication with BIP39 recovery
- SPL-based access control evaluated per request
- Git auto-commit on save with user attribution
- Agent authentication via derived API tokens
- Invitation and onboarding flow
- Hook context extensions for user identity
- Peritext-style rich-text CRDT (diamond-types text oplog + sibling marks oplog) for real-time collaborative editing
- Presence awareness (who is viewing/editing, cursors)

**Out of scope (future):**
- End-to-end encryption of vault contents
- Federation across multiple vaults
- OAuth/OIDC integration with external identity providers

---

## 2. User Profiles

### UP-020-001: Vault Owner (Bootstrap User)

**Goals:** Initialize the collaborative vault, invite the first collaborators, define the base access policy in SPL.

**Constraints:** Must have physical access to the machine running `ztl serve`. First user is created via CLI, not the web UI.

**Happy path:**
1. `ztl serve --collab --init-owner "Alice"` → generates passkey challenge + BIP39 mnemonic
2. Alice registers passkey in browser, writes down 12-word mnemonic
3. Alice creates `access.spl` with base SPL policy
4. Alice invites Bob via `ztl invite --as alice --role editor`

### UP-020-002: Invited Collaborator

**Goals:** Join the vault, read and edit pages within their authorized scope.

**Constraints:** Must receive an invitation link from an existing user. Cannot self-register.

**Happy path:**
1. Receives invitation URL from Alice (e.g., `https://vault.local:8080/auth/accept?token=<jwt>`)
2. Opens URL → sees registration page with display name field
3. Registers passkey, receives BIP39 mnemonic
4. Redirected to vault homepage; can read/edit per SPL policy

### UP-020-003: LLM Agent

**Goals:** Programmatically create/edit pages, run index, trigger builds — acting on behalf of a human user.

**Constraints:** Headless (no browser). Authenticates via derived token from user's BIP39 mnemonic. Subject to the same ACL as the delegating user, with optional additional restrictions expressed in SPL.

**Happy path:**
1. User exports agent token: `ztl agent-token --mnemonic "twelve words ..."`
2. Agent sets `ztl_USER_TOKEN=<token>` or uses `Authorization: Bearer <token>` header
3. Agent calls HTTP API or ztl CLI commands — all attributed to the delegating user
4. Agent's scope can be narrowed via SPL `(except ...)` rules targeting `(is-agent ?user)`

### UP-020-004: Hook-Triggered Agent

**Goals:** Respond to vault events (saves, builds, checks) by performing automated edits — e.g., updating an index page, running a linter, posting a summary.

**Constraints:** Invoked by the hook system (SPEC-016). Must not trigger infinite loops. Operates under the identity of the user who triggered the originating event.

**Happy path:**
1. `.ztl/hooks/on-save` script receives hook context with `saved.user`
2. Script invokes LLM agent with vault context
3. Agent edits pages via API using the triggering user's delegated token
4. Hooks on agent's edits are suppressed (depth limit) or filtered by `saved.user.is_agent`

---

## 3. Requirements

### 3.1 Authentication

#### REQ-020-001: Passkey Authentication

The system SHALL support WebAuthn/FIDO2 passkey registration and authentication for interactive browser sessions.

- Registration: server generates challenge → browser creates credential → server stores public key
- Authentication: server generates challenge → browser signs with credential → server verifies
- Credential storage: `.ztl/users/<user-id>/credential.json`
- Multiple passkeys per user SHALL be supported (e.g., phone + laptop)

Trace: TEST-020-001, CON-020-001

#### REQ-020-002: BIP39 Recovery Key

On account creation, the system SHALL generate a 12-word BIP39 mnemonic (128-bit entropy) and display it exactly once to the user.

- The mnemonic derives an ed25519 keypair via SLIP-0010 at path `m/44'/0'/0'`
- The server stores only the public key in `.ztl/users/<user-id>/recovery.json`
- The mnemonic is never stored server-side
- Recovery flow: user presents mnemonic → server verifies signature against stored pubkey → user registers a new passkey

Trace: TEST-020-002, CON-020-002

#### REQ-020-003: Session Management

Upon successful passkey authentication, the server SHALL issue an opaque session token.

| Property       | Value                                     |
|----------------|-------------------------------------------|
| Storage        | `HttpOnly` cookie + server-side session   |
| Cookie flags   | `SameSite=Strict`, `Secure` (when TLS)   |
| Idle timeout   | Configurable, default 7 days              |
| Max lifetime   | Configurable, default 30 days             |
| Server store   | In-memory `HashMap<Token, Session>`       |

Sessions SHALL survive server restarts by persisting to `.ztl/sessions/` (optional, MAY be in-memory only for simplicity).

Trace: TEST-020-003

#### REQ-020-004: Agent Token Authentication

The system SHALL accept API tokens derived from a user's BIP39 mnemonic for headless authentication.

- Token derivation: `base64url(user_id || generation_byte || ed25519_sign(private_key, "ztl-agent-v1-" || user_id || generation))`
- Presented via `Authorization: Bearer <token>` header or `ztl_USER_TOKEN` env var (CLI mode)
- Resolves to the same user identity as the passkey — same ACL, same git attribution
- Agent tokens do not expire (revocation via user account deletion or key rotation)
- Generation counter enables token rotation without changing the recovery key (REQ-020-055)

Trace: TEST-020-004, CON-020-003

#### REQ-020-005: Bootstrap Owner Creation

The first user (vault owner) SHALL be created via CLI:

```
ztl serve --collab --init-owner "<display-name>"
```

- Generates passkey registration challenge served at `/auth/bootstrap`
- Generates and displays BIP39 mnemonic on the terminal (stderr, once)
- Creates `.ztl/users/<user-id>/` directory with profile, credential, and recovery key
- Sets `owner: true` flag in the user profile
- The owner is automatically granted the `admin` role in SPL (injected as runtime fact)
- Bootstrap is a one-time operation; subsequent `--init-owner` invocations SHALL fail if an owner already exists

Trace: TEST-020-005

### 3.2 Invitations

#### REQ-020-006: Invitation Token Generation

Existing users with the `can_invite` permission SHALL generate invitation tokens:

```
ztl invite --as <username> --role <role> [--expires <duration>] [--pages <glob>]
```

The invitation token is a signed JWT containing:

```json
{
  "iss": "<inviter-user-id>",
  "role": "editor",
  "pages": "projects/*",
  "exp": 1742515200,
  "nonce": "<random-128-bit>"
}
```

- Signed with the server's ed25519 signing key (generated on first `--collab` init, stored in `.ztl/collab/server.key`)
- Default expiry: 72 hours
- `--pages` constrains the invitee's initial scope (optional; omitted means vault-wide per role)
- The CLI outputs a full invitation URL: `https://<host>/auth/accept?token=<jwt>`
- Invitation tokens are single-use; the nonce is recorded in `.ztl/collab/used-nonces.json` upon acceptance
- Used nonces SHALL be pruned from `.ztl/collab/used-nonces.json` when their corresponding JWT `exp` timestamp is more than 24 hours in the past

Trace: TEST-020-006, CON-020-004

#### REQ-020-007: Invitation Acceptance

When a user visits the invitation URL:

1. Server validates JWT signature, expiry, and nonce uniqueness
2. Server presents a registration page: display name field + passkey registration prompt
3. User enters display name and registers a passkey
4. Server generates and displays BIP39 mnemonic (in browser, with "write this down" warning)
5. Server creates `.ztl/users/<new-user-id>/` with profile, credential, recovery key
6. Server injects SPL facts for the new user's role and scope (appended to `access.spl`)
7. Server issues session cookie → redirect to vault homepage

The injected SPL facts follow the pattern:

```lisp
; Invited by alice on 2026-03-18
(given (role bob editor))
(given (scope bob "projects/*"))
```

Trace: TEST-020-007

#### REQ-020-008: Invitation SPL Policy

The ability to invite users SHALL itself be governed by SPL:

```lisp
; Admins can invite anyone
(normally r-invite-admin
  (admin ?user)
  (can-invite ?user))

; Editors can invite readers to their own scope
(normally r-invite-editor
  (and (role ?user editor) (scope ?user ?s))
  (can-invite-scoped ?user ?s))

; But nobody can invite admins except the owner
(except d-invite-admin-restrict
  (and (can-invite ?user) (not (owner ?user)))
  (can-invite-as-admin ?user))
```

Trace: TEST-020-008

### 3.3 Access Control

#### REQ-020-009: SPL-Based Authorization

Every HTTP request to a protected route SHALL be authorized by evaluating SPL access control rules.

**Evaluation:** `built-in defaults` + `access.spl` + `requested page SPL` → combine → reason.

1. Load built-in default rules (hardcoded, always present)
2. Load `access.spl` from vault root (the vault-wide policy)
3. Load SPL code fences from the requested page (page-level overrides)
4. Inject runtime facts: `(given (authenticated <user-id>))`, `(given (requesting <user-id> <page> <action>))`
5. If the user is an agent: `(given (is-agent <user-id>))`
6. Combine into a single theory → `spindle_core::reason()`
7. Query for `(can-read <user> <page>)` or `(can-edit <user> <page>)`
8. `+d` or `+D` → allow; `-d` or `-D` → 403 Forbidden

Trace: TEST-020-009, CON-020-005

#### REQ-020-010: Access Policy Sources

The system SHALL load access control SPL from these sources, in order (later sources can defeat earlier ones):

1. **Built-in defaults** — hardcoded minimal policy (authenticated users can read; nobody can edit without explicit grant)
2. **Vault policy** — `access.spl` (standalone SPL file in vault root)
3. **Page-level overrides** — SPL code fences within the requested page's markdown

The built-in default policy:

```lisp
; Default: authenticated users can read all pages
(normally r-default-read
  (authenticated ?user)
  (can-read ?user ?page))

; Default: no edit access without explicit grant
; (absence of can-edit conclusion = denied)

; Owners can do everything — strict rules, cannot be defeated
(always s-owner-read (owner ?user) (can-read ?user ?page))
(always s-owner-edit (owner ?user) (can-edit ?user ?page))
(always s-owner-invite (owner ?user) (can-invite ?user))

; Admins can edit all pages including access.spl
(normally r-admin-edit
  (admin ?user)
  (can-edit ?user ?page))

; Admins can invite
(normally r-admin-invite
  (admin ?user)
  (can-invite ?user))

; Agents cannot edit access.spl
(except d-agent-no-acl
  (is-agent ?user)
  (can-edit ?user "access"))
(prefer d-agent-no-acl r-admin-edit)

; Scoped readers can read pages matching their scope
(normally r-scoped-read
  (and (role ?user reader) (scope ?user ?s) (in-scope ?page ?s))
  (can-read ?user ?page))

; Scoped editors can read and edit pages matching their scope
(normally r-scoped-edit
  (and (role ?user editor) (scope ?user ?s) (in-scope ?page ?s))
  (can-edit ?user ?page))

(normally r-scoped-edit-read
  (and (role ?user editor) (scope ?user ?s) (in-scope ?page ?s))
  (can-read ?user ?page))

; Default visibility: denied pages hidden from sidebar/search, 403 on direct access
(given (visibility-mode mixed))
```

The `in-scope` predicate is a built-in that performs glob matching: `(in-scope "projects/roadmap" "projects/*")` holds. The system grounds `in-scope` facts for all (page, scope) pairs at ACL cache warm-up.

The `in-scope` predicate is a **runtime-grounded** built-in: the system generates `(given (in-scope "<page>" "<scope>"))` facts for all page/scope combinations where the page path matches the scope glob pattern. These facts are regenerated on each ACL cache invalidation.

The owner's access uses `always` (strict rules) — it cannot be defeated by any vault-level or page-level override. This guarantees the owner can never be locked out, even by a malformed policy.

The admin's access to `access.spl` uses `normally` (defeasible) — it CAN be overridden. For example, a vault that wants owner-only policy editing:

```lisp
; Override: only owner can edit access control
(except d-acl-owner-only
  (and (not (owner ?user)) (admin ?user))
  (can-edit ?user "access"))
(prefer d-acl-owner-only r-admin-edit)
```

Or a vault that wants to delegate policy editing to a specific role:

```lisp
; Grant policy-editor role access to access.spl
(normally r-policy-editor-acl
  (role ?user policy-editor)
  (can-edit ?user "access"))
```

The built-in defaults are the sensible starting point; SPL defeasibility is how each vault customizes from there.

Trace: TEST-020-010

#### REQ-020-011: Temporal Access Control

Access rules MAY use SPL temporal reasoning (Allen interval algebra) for time-bounded permissions:

```lisp
; Grant access during the conference (March 20-22, 2026)
(given (during (conference-access) 1742428800000 1742688000000))

(normally r-conf-read
  (and (role ?user attendee)
       (during (conference-access) ?T)
       (within (now) ?T))
  (can-read ?user "Conference Notes"))
```

The system SHALL inject a `(given (now <epoch-ms>))` fact with the current server time before each evaluation.

Trace: TEST-020-011

#### REQ-020-012: Deontic Access Modalities

Access rules MAY use SPL modal operators for obligation and prohibition semantics:

```lisp
; Editors are permitted to edit their scoped pages
(normally r-edit-perm
  (and (role ?user editor) (scope ?user ?s) (matches ?page ?s))
  [P](edit ?user ?page))

; Reviewers are obligated to review flagged pages
(normally r-review-oblig
  (and (role ?user reviewer) (flagged ?page))
  [O](review ?user ?page))

; Nobody may edit the audit log
(always s-no-audit-edit
  (page-is "Audit Log")
  [F](edit ?user "Audit Log"))
```

The system SHALL map modal conclusions to HTTP responses:
- `[P](edit ...)` at `+d` → 200 (allowed)
- `[F](edit ...)` at `+d` → 403 (forbidden, with reason)
- `[O](review ...)` at `+d` → informational (surfaced in UI as task)

Deontic modalities are an **optional overlay** on the base ACL system. The system always checks `(can-read ...)` and `(can-edit ...)` first (REQ-020-009). If deontic rules are present, they are evaluated additionally:
- `[F](edit ?user ?page)` at `+d` overrides a `(can-edit ?user ?page)` at `+d` → denied
- `[P](edit ?user ?page)` at `+d` is redundant with `(can-edit ...)` but makes policy intent explicit
- `[O](review ?user ?page)` is informational only and does not affect access decisions

Vaults that don't use deontic modalities are unaffected — the base `(can-read/can-edit)` predicates are sufficient.

Trace: TEST-020-012

#### REQ-020-013: ACL Cache

The system SHALL cache the computed access matrix to avoid re-running SPL reasoning on every request.

| Property       | Value                                                |
|----------------|------------------------------------------------------|
| Cache key      | `(user_id, page_slug, action)`                       |
| Invalidation   | On vault `merkle_root` change (file save, re-index)  |
| Storage        | In-memory `HashMap` within `WebState`                |
| Warm-up        | Lazy: evaluated on first access per (user, page, action) tuple; bulk invalidation on vault_root_hash change |

Trace: TEST-020-013, NFR-020-001

#### REQ-020-014: Authorization Explainability

The system SHALL expose an API endpoint for explaining authorization decisions:

```
GET /api/acl/explain?user=<id>&page=<slug>&action=<read|edit>
```

Response (JSON):

```json
{
  "user": "bob",
  "page": "Secret Project",
  "action": "edit",
  "decision": "denied",
  "tag": "-d",
  "proof": [
    {"rule": "r-default-read", "type": "defeasible", "conclusion": "+d can-read(bob, Secret Project)"},
    {"rule": "r-restrict-edit", "type": "defeater", "conclusion": "-d can-edit(bob, Secret Project)"},
    {"rule": "r-restrict-edit > r-default-edit", "type": "superiority"}
  ],
  "sources": [
    {"file": "access.spl", "line": 12},
    {"file": "Secret Project.md", "line": 45}
  ]
}
```

This endpoint requires `admin` or `owner` role. It uses `spindle_core::query::why_not()` for denied decisions and proof tracing for allowed decisions.

Trace: TEST-020-014

### 3.4 Edit Tracking and Git Integration

#### REQ-020-015: Auto-Commit on Save

When a page is saved via `PUT /{slug}`, the system SHALL commit the change to the vault's git repository.

**Flow:**
1. Write file to disk (existing behavior)
2. `git add <file_path>` via `git2::Index::add_path()`
3. `git commit` via `git2::Repository::commit()` with:
   - Author: `<display_name> <<user-id>@vault>` (from authenticated session)
   - Message: `edit: <Page Name>` (or custom via `X-Commit-Message` header)
   - Parent: current HEAD
4. `jj git import` — synchronize jj's view of the git repo after the git2 commit (required because jj uses colocated git mode when `.git/` exists, per SPEC-017)
5. Re-index vault + rebuild graph (existing behavior)
6. jj snapshot (existing auto_snapshot, now also captures the git commit)
7. Fire `on-save` hooks with user identity in context (REQ-020-022)

Trace: TEST-020-015, CON-020-006

#### REQ-020-016: Conflict Resolution

For browser-based editing, the Peritext CRDT handles concurrent edits automatically — conflicts are resolved by the CRDT merge semantics (REQ-020-024, REQ-020-025). No manual merge is required.

For API-based edits (agents, CLI) that submit raw markdown via `PUT /{slug}`:

- If no CRDT session is active for the page → write directly (existing behavior)
- If a CRDT session IS active → the submitted markdown is parsed into CRDT operations and merged into the live document, then broadcast to connected editors
- The `If-Match` header with the page's merkle hash MAY be used for optimistic concurrency on API writes; if the on-disk file has changed → 409 Conflict with current content in the response body

Trace: TEST-020-016

#### REQ-020-017: Page Creation and Deletion via API

The system SHALL support creating new pages:

```
PUT /{slug}
Content-Type: text/markdown
X-Create: true

# New Page Title

Content here.
```

- If the page does not exist and `X-Create: true` is present → create file at `<slug>.md`
- If the page does not exist and `X-Create` is absent → 404
- New page creation follows the same git commit flow as edits (REQ-020-015)
- ACL check: `(can-edit ?user ?page)` must hold (page-name derived from slug)
- `DELETE /{slug}` with `can-edit` ACL check → deletes file, git commits `delete: <Page Name>`, evicts active CRDT session, notifies editors via WebSocket `deleted` message. Links to the deleted page become dead links (standard ztl behavior). The deletion is reversible via `git revert`.

Trace: TEST-020-017

### 3.5 Agent Integration

#### REQ-020-018: Agent API Endpoints

The system SHALL expose JSON API endpoints for programmatic vault operations:

| Route                     | Method | Purpose                          | ACL Required         |
|---------------------------|--------|----------------------------------|----------------------|
| `POST /api/index`         | POST   | Trigger vault re-index           | `can-admin`          |
| `POST /api/build`         | POST   | Trigger static site build        | `can-admin`          |
| `GET /api/pages`          | GET    | List all pages (JSON)            | `can-read` (any)     |
| `GET /api/pages/{slug}`   | GET    | Get page content as markdown     | `can-read` (page)    |
| `PUT /api/pages/{slug}`   | PUT    | Create or edit page              | `can-edit` (page)    |
| `GET /api/graph`          | GET    | Export link graph as JSON        | `can-read` (any)     |
| `POST /api/reason`        | POST   | Run SPL query, return conclusions| `can-admin`          |
| `GET /api/acl/explain`    | GET    | Explain ACL decision             | `admin` or `owner`   |
| `GET /api/users`          | GET    | List users                       | `admin`              |

All endpoints require a valid session cookie or `Authorization: Bearer <agent-token>`.

Trace: TEST-020-018, CON-020-007

#### REQ-020-019: Agent Token Derivation CLI

The system SHALL provide a CLI command to derive an agent token:

```
ztl agent-token --mnemonic "<twelve words>"
```

- Derives the ed25519 private key from the mnemonic (SLIP-0010)
- Computes `base64url(user_id || generation_byte || ed25519_sign(private_key, "ztl-agent-v1-" || user_id || generation))`
- Outputs the base64url-encoded token to stdout
- The token is a long-lived bearer credential; the user is responsible for securing it

Trace: TEST-020-019

#### REQ-020-020: Agent Loop Prevention

The system SHALL prevent infinite loops when agents trigger hooks that invoke agents.

**Three mechanisms:**

1. **Hook depth counter:** The environment variable `ztl_HOOK_DEPTH` SHALL be set to `0` on the initial event and incremented on each hook invocation. Hooks SHOULD refuse to invoke agents when depth ≥ 1.

2. **User identity in hook context:** The `saved.user` field (REQ-020-022) includes `is_agent: true` when the save was performed by an agent token. Hooks can filter on this.

3. **Suppression flag:** Agent writes MAY set `X-No-Hooks: true` header to suppress `on-save` hooks for that specific write. The server SHALL respect this header only when the request is authenticated.

Trace: TEST-020-020

#### REQ-020-021: Agent SPL Constraints

Agent permissions SHALL be expressible in SPL using the `(is-agent ?user)` fact:

```lisp
; Agents can only edit pages they're assigned to
(normally r-agent-edit
  (and (is-agent ?user) (agent-assigned ?user ?page))
  (can-edit ?user ?page))

; Agents cannot edit the access control policy
(except d-agent-no-acl
  (is-agent ?user)
  (can-edit ?user "access"))
(prefer d-agent-no-acl r-agent-edit)

; Agents cannot invite users
(except d-agent-no-invite
  (is-agent ?user)
  (can-invite ?user))
(prefer d-agent-no-invite r-invite-admin)

; Specific agent assignments
(given (agent-assigned curator-agent "Index"))
(given (agent-assigned curator-agent "Recent Changes"))
(given (is-agent curator-agent))
```

Trace: TEST-020-021

### 3.6 Hook Context Extensions

#### REQ-020-022: User Identity in Hook Context

All hook lifecycle points SHALL include the acting user's identity in the JSON context:

```json
{
  "user": {
    "id": "alice-abc123",
    "name": "Alice",
    "is_agent": false,
    "roles": ["admin"]
  }
}
```

For `on-save` hooks, the `saved` field SHALL include the user:

```json
{
  "saved": {
    "file": "meeting-notes.md",
    "page": "Meeting Notes",
    "content_length": 482,
    "user": {
      "id": "alice-abc123",
      "name": "Alice",
      "is_agent": false
    }
  }
}
```

Hooks invoked during unauthenticated operations (e.g., `pre-build` from CLI) SHALL have `"user": null`.

Trace: TEST-020-022

#### REQ-020-023: Agent Hook Lifecycle Point

The system SHALL support an `on-agent` hook lifecycle point:

| Property    | Value                                              |
|-------------|----------------------------------------------------|
| Trigger     | Manual (`ztl agent run <name>`), scheduled, chained|
| Context     | `{ task, target_pages, user, budget_tokens }`      |
| Exit 0      | Agent action accepted; output logged               |
| Exit non-0  | Agent action rejected; stderr logged as warning    |

This enables orchestration patterns:
- "After `post-build`, run the link-checker agent"
- "When a page is tagged `#needs-review`, notify the reviewer agent"
- Scheduled via external cron invoking `ztl agent run <name>`

Trace: TEST-020-023

#### REQ-020-023a: ACL Violation Hook Lifecycle Point

The system SHALL support an `on-acl-violation` hook lifecycle point:

| Property    | Value                                              |
|-------------|----------------------------------------------------|
| Trigger     | External edit violates ACL policy (REQ-020-043)    |
| Context     | `{ violation: { page, expected_acl, actual_editor, commit_sha } }` |
| Exit 0      | Violation acknowledged; logged                     |
| Exit non-0  | Hook failure; logged as warning                    |

This hook enables automated responses to ACL violations (e.g., notify admins via Slack, create an audit entry, trigger a revert workflow).

### 3.7 Collaborative Editing (Peritext CRDT)

#### REQ-020-024: Peritext CRDT Editing Layer

The system SHALL use the Peritext algorithm (Ink & Switch, 2021) as the live editing layer for concurrent multi-user editing of the same page.

**Document lifecycle:**

| Phase       | Representation             | Owner               |
|-------------|----------------------------|----------------------|
| At rest     | Flat markdown (`.md` file) | Filesystem + git     |
| Loading     | Markdown → Peritext CRDT   | Server               |
| Live edit   | Peritext CRDT state        | Server (authoritative), clients (local replicas) |
| Saving      | Peritext CRDT → markdown   | Server               |
| Committed   | Markdown diff in git       | git2/libgit2         |

**Load flow:**
1. Read `.md` file from disk
2. Parse markdown into Peritext CRDT document: each character gets an `opId`, formatting spans become `addMark` operations
3. Hold CRDT state in memory, keyed by page slug, in `WebState`

**Edit flow:**
1. Client connects via WebSocket to `/ws/edit/{slug}`
2. Server sends current CRDT state as initial sync
3. Client sends local operations (insert, delete, addMark, removeMark)
4. Server merges operations into authoritative CRDT state
5. Server broadcasts merged operations to all other connected clients
6. All clients converge to the same document state (CRDT guarantee)

**Save flow (quiescence):**
1. After N seconds of no edits (configurable, default 5s), or on explicit save
2. Server serializes CRDT state to canonical markdown
3. If markdown differs from on-disk file → write, git commit (REQ-020-015), fire hooks
4. If markdown is unchanged → no-op

Trace: TEST-020-024, CON-020-008

#### Marks Layer Architecture

Peritext was first published with a TypeScript implementation riding on automerge's `RichText`. ztl implements the same algorithm natively in Rust on top of `diamond-types = "1.0"`. The backend is split into two co-operating CRDT oplogs inside a single document (`DiamondCrdtDocument`):

- **Text oplog** (`diamond_types::list::OpLog`): owns the character sequence and merges concurrent splices. Every character carries a DT op-id (agent + seq) — the equivalent of Peritext's `opId` anchor, stable under concurrent insert and delete.
- **Marks oplog** (project-owned `MarksDoc`, wrapping a *sibling* `diamond_types::list::OpLog`): carries span-level edits as newline-delimited JSON entries appended at the oplog tail. Three op kinds:
  - `Mark { name, value, start, end, expand }` — open a span of the given `MarkType` (REQ-020-025) with its per-mark `ExpandMark` (inclusive for `bold`/`italic`/`strikethrough`/`highlight`; non-growing for `code`/`wikilink`/`link`/`comment`).
  - `Unmark { name, value?, start, end, expand }` — carve any overlapping same-named span out of the range.
  - `Shift { pos, delta }` — emitted automatically by every text splice so open spans track the text-oplog's char positions under concurrent edits.
- **Atomic writes.** `DiamondCrdtDocument` brackets each splice so the text and marks oplogs advance together. The two oplogs share agent ids and are serialised side-by-side in the WAL blob.
- **Materialisation.** To read the current mark set, replay the marks oplog in DT's canonical merge order and fold the ops into a `Vec<Mark>`. `Shift` ops are applied with per-span growth awareness at each boundary — this is where Peritext's inclusive vs non-growing distinction is enforced (REQ-020-025). Exclusive marks (`wikilink`, `link`) are last-write-wins per overlapping range; DT's canonical order is Lamport-total, matching the "Lamport timestamp ordering of the opId" contract in REQ-020-025.
- **No extra CRDT dependency.** The marks layer rides the same `diamond-types` crate as the text layer. ztl does not depend on `diamond-types-extended`, `automerge-rs` `RichText`, or a hand-rolled Lamport store. The `MarkType` surface (mark name, `ExpandMark`, nesting order, conflict mode) is unchanged from the earlier automerge-based design, so `CrdtBackend::{mark, unmark, marks}` callers are byte-identical on the wire.

Rationale for the split-oplog shape: storing mark ops inline in the text oplog would corrupt char offsets for text splices and complicate markdown serialisation. Two parallel DT documents lets DT do all RLE packing, agent-ordering, and merge bookkeeping for free on both sides.

#### REQ-020-025: Peritext Mark Types

The Peritext CRDT schema SHALL support the following mark types, mapped to/from markdown syntax:

| Mark type    | Markdown syntax        | Growth behavior | Conflict resolution     |
|--------------|------------------------|-----------------|-------------------------|
| `bold`       | `**text**`             | Inclusive       | Coexist (overlay)       |
| `italic`     | `*text*`               | Inclusive       | Coexist (overlay)       |
| `code`       | `` `text` ``           | Non-growing     | Coexist (overlay)       |
| `strikethrough` | `~~text~~`          | Inclusive       | Coexist (overlay)       |
| `wikilink`   | `[[target\|alias]]`    | Non-growing     | Last-write-wins per span|
| `link`       | `[text](url)`          | Non-growing     | Last-write-wins per span|
| `highlight`  | `==text==`             | Inclusive       | Coexist (overlay)       |
| `comment`    | `%%text%%`             | Non-growing     | Coexist (overlay)       |

**Growth behavior:**
- **Inclusive:** text inserted at the boundary of a formatted span inherits the formatting (e.g., typing at the end of a bold word stays bold)
- **Non-growing:** text inserted at the boundary does NOT inherit the formatting (e.g., typing after a wikilink closing `]]` is plain text)

**Conflict resolution:**
- **Coexist:** overlapping marks of different types all apply (bold + italic = bold italic)
- **Last-write-wins:** overlapping marks of the same exclusive type resolve by Lamport timestamp ordering of the `opId`

Trace: TEST-020-025

#### REQ-020-026: Block-Level Structure

Peritext handles inline formatting. Block-level structure (headings, lists, code fences, frontmatter, SPL blocks) SHALL be handled as **indivisible block tokens** in the CRDT sequence.

- Each block boundary (heading prefix `## `, list marker `- `, fence `` ``` ``) is a single atomic token
- Users can insert, delete, and reorder blocks, but not concurrently split a block boundary
- Code fences (including SPL blocks) are opaque ranges — their content is plain text within the CRDT, not subject to formatting marks
- Frontmatter (`---` delimited YAML) is a single opaque block at position 0

This avoids the problem Peritext explicitly scopes out (block-level merging) by treating blocks as structural atoms rather than attempting to merge their internal structure.

Trace: TEST-020-026

#### REQ-020-027: Canonical Markdown Serialization

The system SHALL define a canonical serialization from Peritext CRDT state to markdown, such that:

1. `parse(serialize(crdt_state))` produces an equivalent CRDT state (round-trip fidelity)
2. The serialization is deterministic — same CRDT state always produces byte-identical markdown
3. Formatting marks serialize to their standard markdown syntax in a fixed order: strikethrough > bold > italic > code > highlight (outermost to innermost)
4. Wikilinks serialize as `[[target]]` or `[[target|alias]]` depending on whether alias differs from target
5. Whitespace normalization: single blank line between blocks, no trailing whitespace, trailing newline at EOF

This ensures git diffs are clean and predictable regardless of which user's operations produced the final state.

Trace: TEST-020-027

#### REQ-020-028: WebSocket Editing Protocol

The system SHALL expose a WebSocket endpoint for live editing:

```
WS /ws/edit/{slug} (requires valid session cookie or one-time ticket, see REQ-020-061)
```

Browser clients authenticate via session cookie. Agent clients obtain a one-time ticket via `POST /auth/ws-ticket` and connect with `/ws/edit/{slug}?ticket=<one-time-ticket>` (see REQ-020-061). This avoids exposing long-lived bearer tokens in WebSocket URLs.

**Messages (server → client):**

```json
{"type": "sync", "state": "<encoded-crdt-state>"}
{"type": "op", "ops": [{"insert": "a", "opId": "3@alice", "after": "2@bob"}]}
{"type": "op", "ops": [{"addMark": "bold", "start": "1@alice", "end": "5@alice", "opId": "6@alice"}]}
{"type": "presence", "user": "alice", "cursor": {"opId": "3@alice"}, "selection": null}
{"type": "presence", "user": "bob", "page": "Architecture", "action": "left"}
{"type": "saved", "page": "Architecture", "user": "bob", "commit": "abc1234"}
```

**Messages (client → server):**

```json
{"type": "op", "ops": [{"insert": "x", "after": "3@alice"}]}
{"type": "op", "ops": [{"delete": "3@alice"}]}
{"type": "op", "ops": [{"addMark": "wikilink", "target": "Other Page", "start": "1@a", "end": "5@a"}]}
{"type": "cursor", "opId": "3@alice", "selection": {"start": "1@a", "end": "5@a"}}
{"type": "save"}
```

**Presence:**
- Each client periodically sends cursor position (as `opId` reference, not index — stable under concurrent edits)
- Server broadcasts cursor positions to all other clients on the same page
- Cursors are rendered in the editor with user name labels

**Connection lifecycle:**
1. Client opens WebSocket → server sends `sync` with full CRDT state
2. Client applies local edits → sends `op` messages
3. Server merges, broadcasts to peers
4. On disconnect → server broadcasts `presence.left`
5. If all clients disconnect → CRDT state remains in memory for a configurable TTL (default 10 minutes), then flushes to disk and is evicted

Trace: TEST-020-028

#### REQ-020-029: CRDT State Management

The server SHALL manage CRDT document states in memory:

| Property          | Value                                                  |
|-------------------|--------------------------------------------------------|
| Storage           | `HashMap<PageSlug, CrdtDocument>` in `WebState`       |
| Load trigger      | First WebSocket connection to a page                   |
| Eviction          | TTL after last client disconnects (default 10 min)     |
| Flush trigger     | Quiescence (5s no edits), explicit save, eviction      |
| Flush action      | Serialize → write → re-index → git commit → jj snapshot |
| Memory bound      | Configurable max concurrent CRDT documents (default 50)  |

When a page is requested via `GET /{slug}` (read-only view) and no CRDT state is loaded, the server reads directly from the `.md` file as it does today. The CRDT is only loaded when someone opens the editor.

Trace: TEST-020-029

#### REQ-020-044: CRDT Crash Recovery

The server SHALL persist CRDT operations to a write-ahead log (WAL) for crash recovery:

| Property     | Value                                        |
|--------------|----------------------------------------------|
| Location     | `.ztl/crdt/<slug>.wal`                      |
| Write        | Append each CRDT operation as it arrives     |
| Truncate     | After successful flush to markdown           |
| Recovery     | On startup, replay WAL for any non-empty files|
| Max WAL size | 10MB per document (force-flush if exceeded)  |

On server restart, if `.ztl/crdt/` contains non-empty WAL files:
1. Load the on-disk markdown for each page
2. Parse into CRDT state
3. Replay WAL operations
4. Immediately flush (serialize → write → commit)
5. Delete the WAL file

This ensures at most one CRDT operation is lost on crash (the one in-flight at crash time), rather than up to 5 seconds of work.

Trace: TEST-020-044

#### REQ-020-034: CRDT Flush Pipeline (Merkle and History Integration)

When a CRDT document flushes to disk, the system SHALL execute the full vault pipeline to keep the merkle tree, jj history, and derived caches consistent:

**Flush pipeline (sequential):**

1. **Serialize** — CRDT state → canonical markdown (REQ-020-027)
2. **Write** — write `.md` file to disk
3. **Re-scan** — re-parse the written file through the scanner to produce a fresh `ParsedFile` with updated merkle leaves, wikilinks, and SPL blocks (SPEC-006)
4. **Recompute merkle** — update the file's merkle root and recompute `vault_root_hash`
5. **Invalidate ACL cache** — if `vault_root_hash` changed, clear the ACL decision cache (REQ-020-013) and recompute the access matrix
6. **Rebuild search index** — update Tantivy index for the changed file
7. **Git commit** — `git add` + `git commit` with user attribution (REQ-020-015)
7a. **jj git import** — synchronize jj's view of the git repo after the git2 commit (required because jj uses colocated git mode when `.git/` exists, per SPEC-017)
8. **jj snapshot** — trigger `auto_snapshot` with the new `vault_root_hash`; dedup prevents a snapshot if the hash is unchanged (SPEC-017 §REQ-076)
9. **Rebuild graph** — update `LinkGraph` in `WebState` if wikilinks changed
10. **Fire hooks** — run `on-save` hooks with updated context including user identity (REQ-020-022)

This is the same pipeline as the existing `save_handler` (PUT), unified into a single code path that both the PUT handler and CRDT flush invoke.

Trace: TEST-020-034

#### REQ-020-035: Access Policy Immediate Flush

Edits to `access.spl` or to any page containing SPL code fences SHALL trigger an **immediate flush** rather than waiting for the quiescence timer.

- When the file being edited is `access.spl`, every CRDT operation triggers a flush with zero quiescence delay
- When a CRDT operation modifies content within an SPL code fence in any markdown page (detected by the block token boundaries, REQ-020-026), the server SHALL flush within 500ms
- This minimizes the window where the in-memory ACL diverges from the editor's visible state

If the SPL content has a parse error after flush, the system SHALL fall back to the previous valid policy (not the built-in defaults) and surface the error to the editing user via a WebSocket message:

```json
{"type": "spl-error", "message": "Parse error at line 12: unexpected token", "fallback": "previous-valid-policy"}
```

Trace: TEST-020-035

#### REQ-020-036: CRDT State and Historical Queries

Historical queries (`--at`, `page.history`, `vault.history`) SHALL reflect the **last flushed state**, not in-flight CRDT edits.

- The CRDT is a live draft; flushed markdown committed to git is the canonical record
- `ztl links Foo --at now` returns the state as of the last flush, not the current CRDT state
- If a user requests the current state of a page with an active CRDT session via the API (`GET /api/pages/{slug}`), the response SHALL include a `X-CRDT-Dirty: true` header if the CRDT has unflushed edits, and a `X-CRDT-Last-Flush` header with the ISO 8601 timestamp of the last flush
- Template variables `page.history.last_changed` reflect the last git commit (flush), not the last CRDT keystroke

Trace: TEST-020-036

#### REQ-020-037: Merkle Hash for API Conflict Detection with Active CRDT

When an API client submits `PUT /{slug}` with `If-Match` and a CRDT session is active for that page:

1. The server SHALL compare `If-Match` against the **CRDT document's last-flush merkle hash**, not the on-disk file hash (which may be identical since the CRDT hasn't flushed yet, or stale if it has)
2. If the CRDT has unflushed edits, the effective merkle hash is the hash of the serialized CRDT state (computed on demand, not cached)
3. If the hashes don't match → 409 Conflict with the current CRDT-serialized content in the response body

This ensures API clients (agents) are aware of in-flight CRDT edits, not just the last flushed state.

Trace: TEST-020-037

#### REQ-020-038: SPL Block Re-parsing on Flush

When a CRDT flush writes a file containing SPL blocks, the re-scan step (REQ-020-034 step 3) SHALL:

1. Extract SPL blocks from the flushed markdown
2. Compute dual hashes per SPEC-006: `content_hash` (raw text) and `ast_hash` (parsed AST)
3. If `ast_hash` changed compared to the previous scan → invalidate the reasoning cache for affected theories
4. If `content_hash` changed but `ast_hash` did not → no theory rebuild (whitespace-only edits)
5. If the SPL block is in `access.spl` or is tagged as access policy → trigger ACL recomputation (REQ-020-035)
6. Run drift detection (SPEC-006 §grounding) if grounding metadata exists on affected rules

SPL is NOT parsed incrementally during CRDT editing. The CRDT treats code fences as opaque text. Parsing and validation happen only on flush, which is the point where the SPL enters the reasoning pipeline.

Trace: TEST-020-038

### 3.9 External Edit Reconciliation

#### REQ-020-039: Filesystem Watch for External Edits

When running in `--collab` mode, the server SHALL watch the vault directory for filesystem changes not originating from its own CRDT flush or save pipeline (SPEC-008 watch mode extended).

**Detection:**

The server maintains a `pending_writes: HashSet<PathBuf>` of files it is currently writing. When a filesystem event fires for a file NOT in this set, it is an **external edit** — someone or something modified the file outside of ztl serve.

Sources of external edits:
- An agent editing `.md` files directly on disk
- A `git pull` or `git merge` bringing in remote changes
- A user editing files with a text editor while ztl serve is running
- A CI job or cron script modifying vault files

**Reconciliation pipeline (on external edit detected):**

1. **Debounce** — batch filesystem events over a 500ms window (existing SPEC-008 behavior)
2. **Re-scan changed files** — re-parse through scanner, recompute merkle leaves
3. **Recompute vault_root_hash** — if unchanged, stop (SPEC-006 two-tier cache)
4. **Invalidate ACL cache** — if `access.spl` or any page with SPL blocks changed
5. **Rebuild search index** — update Tantivy for changed files
6. **Update link graph** — rebuild `LinkGraph` in `WebState`
7. **jj snapshot** — trigger `auto_snapshot` with new `vault_root_hash`
8. **Notify connected clients** — broadcast a WebSocket event to all connected editors

Trace: TEST-020-039

#### REQ-020-040: CRDT Reconciliation on External Edit

When an external edit modifies a file that has an **active CRDT session**, the server SHALL reconcile the external change with the live CRDT state:

**Case 1 — CRDT is clean (no unflushed edits):**
- Discard the in-memory CRDT state
- Reload from the new on-disk markdown
- Send a `sync` message to all connected editors with the new CRDT state
- Editors see the page content replaced with the externally-modified version

**Case 2 — CRDT is dirty (has unflushed edits):**
- Parse the external file content into a Peritext CRDT document
- Merge the external CRDT state with the live CRDT state using CRDT merge semantics
- The merge is automatic and conflict-free (CRDT guarantee) — both the external edits and the in-flight CRDT edits are preserved
- Broadcast merged operations to all connected editors
- Mark the CRDT as dirty — it will flush on quiescence, producing a merged markdown file and a git commit

**Case 3 — External edit deletes the file:**
- Notify connected editors via WebSocket: `{"type": "deleted", "page": "..."}`
- Evict the CRDT session
- If the CRDT had unflushed edits, write them to a recovery file at `.ztl/recovery/<slug>.md` and log a warning

Trace: TEST-020-040

#### REQ-020-041: Git-Based External Change Detection

In addition to filesystem watching, the server SHALL periodically check the git ref for the current branch (default: every 30 seconds) to detect changes from `git pull`, `git merge`, or `git rebase`:

1. Read current HEAD commit hash (via `git2`)
2. If HEAD has advanced since last check → external commits were added
3. Diff the old HEAD against new HEAD to identify changed files
4. Feed changed files into the reconciliation pipeline (REQ-020-039 steps 2-8)

This catches cases where filesystem events are missed (e.g., NFS mounts, Docker volume sync) and provides a reliable fallback.

The poll interval is configurable: `ztl serve --collab --git-poll-interval 30s`

Trace: TEST-020-041

#### REQ-020-042: External Edit Attribution

When an external edit is detected via filesystem watch or git poll:

- If the edit arrived as a git commit, the commit author is used for audit/hook context: `{"user": {"name": "Agent Bot", "id": "external:agent-bot@example.com", "is_external": true}}`
- If the edit is an uncommitted file change (no git commit), the user is `{"user": {"name": "(external)", "id": "external:filesystem", "is_external": true}}`
- External edits are NOT re-committed by ztl (they are already committed, or they are uncommitted working directory changes that the external actor is responsible for)
- `on-save` hooks fire with the `is_external: true` flag so hooks can distinguish ztl-originated saves from external edits

Trace: TEST-020-042

#### REQ-020-043: ACL Enforcement on External Edits

External edits bypass ztl's ACL (they happen at the filesystem/git level, not through the HTTP API). The server SHALL:

1. After reconciling an external edit, evaluate the resulting vault state against the ACL policy
2. If an external edit modified `access.spl` → immediately recompute ACL; log a WARN: `"access policy modified externally — ACL recomputed"`
3. If an external edit created or modified pages in a way that would violate the ACL (e.g., an agent without `can-edit` wrote to a restricted page) → the edit is already on disk (cannot be undone), but:
   - Log a WARN with the violation details
   - Fire an `on-acl-violation` hook (if configured) with the violation context
   - The violation is surfaced to admin users in the UI as a banner

The system does NOT reject or revert external edits — that would be dangerous (data loss). It detects and reports violations after the fact. The vault owner can then revert the commit manually if needed.

Trace: TEST-020-043

### 3.10 User Experience

#### REQ-020-045: Recovery Phrase UX

The BIP39 mnemonic display (during bootstrap and invitation acceptance) SHALL include:

- A plain-language explanation: "This is your backup key. If you lose access to your device, these 12 words are the only way to recover your account. We cannot recover them for you."
- The 12 words displayed in a numbered list with large, clear typography
- A "Copy to clipboard" button (with a warning: "Store this somewhere safe, not in a screenshot")
- A mandatory confirmation checkbox: "I have saved my recovery phrase" before the user can proceed
- No automatic timeout or dismissal — the user controls when to move on

Trace: TEST-020-045

#### REQ-020-046: Passkey Registration Guidance

The passkey registration screen (bootstrap and invitation acceptance) SHALL include:

- A brief explanation: "Your browser will ask you to set up a login method — this might be your fingerprint, face, or a security key. This replaces passwords."
- A visual indicator of what to expect (e.g., "You'll see a prompt from your browser or device")
- Graceful handling of passkey registration failure: a clear error message with a "Try again" button
- A fallback note: "If your device doesn't support passkeys, you can use your recovery phrase to log in from a device that does."

Trace: TEST-020-046

#### REQ-020-047: Access Request Flow

When a user encounters a page they cannot access (403 response, REQ-020-033), the system SHALL provide a built-in access request mechanism:

- The 403 page includes a "Request access" button (visible in `mixed` and `transparent` modes)
- Clicking the button creates an access request record in `.ztl/collab/access-requests.json`:
  ```json
  {"user": "bob-d4e5f6", "page": "secret-project", "requested_at": "2026-03-18T14:00:00Z", "status": "pending"}
  ```
- All users with `admin` or `owner` role receive a WebSocket notification: `{"type": "access-request", "user": "Bob", "page": "Secret Project"}`
- Admins see pending access requests in their dashboard (REQ-020-049)
- An `on-access-request` hook fires (if configured) for custom workflows (e.g., Slack notification)
- Admins can approve (by editing `access.spl` to grant access) or dismiss the request

This is the built-in default. Vaults can disable the button by setting `(given (access-requests disabled))` in `access.spl`.

Trace: TEST-020-047

#### REQ-020-048: Permission Management UI

The system SHALL provide a web-based interface for common permission operations, accessible to users with `admin` or `owner` role:

**Route:** `GET /_admin/permissions`

**Features:**
- List all users with their current roles and scopes (derived from SPL facts in `access.spl`)
- "Change role" dropdown per user (reader / editor / admin) — generates and appends the appropriate SPL fact to `access.spl`
- "Change scope" field per user — generates `(given (scope <user> "<glob>"))` fact
- "Revoke access" button — removes the user's role/scope facts from `access.spl`
- Preview pane showing the SPL that will be generated before committing
- All changes go through the normal save pipeline: write `access.spl` → git commit → ACL recompute

The UI generates SPL; it does not replace it. Advanced users can still edit `access.spl` directly. The UI only manages facts it recognizes (role, scope, admin grants) — custom rules are left untouched.

Trace: TEST-020-048

#### REQ-020-049: User Dashboard

The system SHALL provide a personalized dashboard for each authenticated user:

**Route:** `GET /_me`

**Contents:**
- **Recent edits:** pages this user has edited (from git log, last 20 commits by this user)
- **Accessible pages:** pages the user has `can-edit` permission for, grouped by folder
- **Pending reviews:** pages where deontic `[O](review ...)` obligations apply to this user (if any)
- **Access requests:** (admin/owner only) pending access requests from other users
- **Role summary:** plain-language description of the user's permissions — e.g., "You are an **editor** with access to **projects/***"
- **Active sessions:** list of connected devices (from session store)
- **Recovery:** link to re-display passkey management (add/remove passkeys) and initiate recovery flow

Trace: TEST-020-049

#### REQ-020-050: Web-Based Invitation

The system SHALL provide a web-based invitation flow for users with `can_invite` permission:

**Route:** `GET /_admin/invite`

**Features:**
- Form with fields: display name (optional hint), role (dropdown: reader/editor/admin), scope (optional glob pattern), expiry (dropdown: 24h/72h/7d/30d)
- "Generate invitation link" button → creates JWT (same as `ztl invite` CLI) and displays the URL
- "Copy link" button for easy sharing
- List of pending (unexpired, unused) invitations with ability to revoke (marks nonce as used)

The CLI `ztl invite` command remains for automation. The web UI is the primary flow for non-technical inviters.

Trace: TEST-020-050

#### REQ-020-051: Page Comments

The system SHALL support page-level comments for lightweight coordination:

**Storage:** Comments are stored in a sidecar file `.ztl/comments/<slug>.json`:
```json
[
  {"user": "alice-abc123", "text": "I'm rewriting the intro section", "at": "2026-03-18T14:00:00Z"},
  {"user": "bob-d4e5f6", "text": "Sounds good, I'll hold off", "at": "2026-03-18T14:01:00Z"}
]
```

**Features:**
- Comment sidebar visible when viewing or editing a page
- Text input at the bottom for adding a comment
- Comments are NOT part of the page content — they don't affect the markdown, merkle tree, or git history of the page itself
- Comments are broadcast to all users viewing/editing the page via WebSocket
- ACL: users who can read a page can view its comments; users who can edit a page can add comments
- Comments are ephemeral by default — auto-pruned after 30 days (configurable)

Trace: TEST-020-051

#### REQ-020-052: Agent Merge Notification

When an external edit is merged into an active CRDT session (REQ-020-040), the system SHALL notify connected editors with a clear in-editor notification:

**WebSocket message:**
```json
{
  "type": "external-merge",
  "source": {"name": "Bot", "id": "external:bot@ci", "is_agent": true},
  "summary": "3 paragraphs added, 1 paragraph modified",
  "changed_ranges": [{"start": "42@server", "end": "67@server"}]
}
```

**Editor behavior:**
- Display a non-modal banner: "[Bot] made changes to this page — 3 paragraphs added"
- Briefly highlight the changed ranges in the editor (e.g., yellow background that fades after 3 seconds)
- If the merge is large (> 500 characters changed), show a "Review changes" button that opens a diff view

Trace: TEST-020-052

#### REQ-020-053: Page History UI

The system SHALL provide a page history view accessible from the page chrome:

**Route:** `GET /{slug}/_history`

**Features:**
- Chronological list of edits to this page, derived from git log for the file
- Each entry shows: author name, timestamp, commit message, and a "View this version" link
- Diff view between any two versions (rendered markdown diff, not raw)
- "Restore this version" button (creates a new commit reverting to the selected version — requires `can-edit`)
- Attributed to the correct user (from git author, which includes user display name per REQ-020-015)
- If the page has an active CRDT session with unflushed edits, show a "Draft (unsaved)" entry at the top

Trace: TEST-020-053

#### REQ-020-054: Visibility Explanations

When a user encounters a locked or restricted page indicator (lock icon on wikilink, 403 page), the system SHALL provide contextual explanations:

- **Lock icon tooltip (mixed mode):** "This page is restricted. Click to request access." (not just "restricted page")
- **403 page:** Show the name of at least one admin or the page owner who can grant access: "Contact [Alice] to request access" (admin names derived from SPL `(admin ?user)` conclusions)
- **Hidden mode dead links:** No explanation (by design — the page's existence is hidden). But if a user creates a page with the same name as a hidden page, the system SHALL warn: "A restricted page with this name already exists. Contact an admin."
- **Visibility mode switch notification:** If an admin changes the vault's visibility mode, all connected users receive a WebSocket notification explaining the change

Trace: TEST-020-054

### 3.11 Security Hardening

#### REQ-020-055: Agent Token Revocation

Agent tokens SHALL be revocable independently of the recovery key.

- Each agent token includes a `generation` counter in the signed payload: `base64url(user_id || generation_byte || ed25519_sign(private_key, "ztl-agent-v1-" || user_id || generation))`
- The user profile stores `agent_token_generation: u8` (default 0)
- Server verification checks that the token's generation matches the stored generation
- `ztl agent-token --rotate --mnemonic "<words>"` bumps the generation counter in the profile and outputs a new token
- Old tokens immediately become invalid
- The recovery key (BIP39 mnemonic) remains unchanged across rotations
- Admin UI (REQ-020-048) includes a "Revoke agent token" button per user that bumps their generation counter

Trace: TEST-020-055

#### REQ-020-056: Mnemonic Display Security

The BIP39 mnemonic display page SHALL apply defense-in-depth measures:

- Served with Content-Security-Policy: `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'`
- The mnemonic is rendered in a `user-select: none` container with a deliberate "Copy" button (discourages screenshots of selected text)
- The page includes a `<meta name="robots" content="noindex">` tag
- The mnemonic is transmitted exactly once over the TLS connection; the server does not store or cache it
- The "Copy to clipboard" button clears the clipboard after 60 seconds via a client-side timer
- A warning is displayed: "Do not screenshot this. Do not paste it into a chat. Write it on paper."
- The page auto-clears the mnemonic from the DOM after 5 minutes, showing "Recovery phrase hidden for security — reload to see it again" (reload will fail since the server only serves it once)

Trace: TEST-020-056

#### REQ-020-057: Server Signing Key Protection

The server's ed25519 signing key (`.ztl/collab/server.key`) SHALL be protected:

- File permissions: `0600` (owner read/write only), enforced on creation
- `.ztl/collab/server.key` SHALL be added to `.gitignore` automatically on `--collab` init
- `.ztl/users/` SHALL be added to `.gitignore` automatically on `--collab` init
- `.ztl/sessions/` SHALL be added to `.gitignore` automatically on `--collab` init
- `.ztl/collab/` SHALL be added to `.gitignore` automatically on `--collab` init
- On startup, the server SHALL verify file permissions on `server.key` and WARN if they are more permissive than `0600`
- The key is never exposed via any API endpoint or template variable

Trace: TEST-020-057

#### REQ-020-058: Owner Fact Injection Hardening

The `(owner ?user)` fact SHALL be injected exclusively as a runtime fact from the user profile (`owner: true` flag in `.ztl/users/<id>/profile.json`), NOT from `access.spl` or page-level SPL.

- During ACL evaluation, the system SHALL strip any `(given (owner ...))` facts from user-editable SPL sources (`access.spl` and page code fences) before combining the theory
- Only the built-in defaults layer may contain owner-related strict rules
- If `access.spl` contains `(given (owner ...))`, log a WARN: "owner assertion in access.spl ignored — owner is determined by user profile"
- Similarly, `(given (admin ...))` facts SHALL only be loaded from `access.spl`, not from page-level SPL. Pages can assert `(can-read ...)` and `(can-edit ...)` for their own page name only.

Trace: TEST-020-058

#### REQ-020-059: Page-Level SPL Sandboxing

SPL code fences in markdown pages SHALL be sandboxed to prevent privilege escalation:

**Allowed in page-level SPL:**
- Facts and rules referencing the page's own name: `(can-read ?user "<this-page>")`, `(can-edit ?user "<this-page>")`
- Defeaters that restrict access to this page: `(except ... (can-read ?user "<this-page>"))`
- Superiority relations between rules defined within the same page

**Rejected in page-level SPL (stripped with WARN log):**
- Global identity facts: `(given (owner ...))`, `(given (admin ...))`, `(given (role ...))`, `(given (scope ...))`
- Access conclusions for other pages: `(can-read ?user "<other-page>")`
- `(given (visibility-mode ...))` or any vault-wide configuration facts
- `(given (is-agent ...))` or agent assignment facts

The system SHALL validate page-level SPL blocks during the re-scan step (REQ-020-034 step 3). Violations are logged at WARN level and the offending facts/rules are excluded from the combined theory.

Trace: TEST-020-059

#### REQ-020-060: Authentication Rate Limiting

The system SHALL rate-limit authentication endpoints to mitigate brute-force attacks:

| Endpoint | Limit | Window | Response |
|---|---|---|---|
| `POST /auth/login` (passkey) | 10 failures per user | 1 minute | 429 Too Many Requests |
| `POST /auth/recover` | 5 failures per user | 1 minute | 429 Too Many Requests |
| `GET /auth/accept` (invitation) | 10 attempts per IP | 1 minute | 429 Too Many Requests |
| Agent token (Bearer) | 10 failures per IP | 1 minute | 429 Too Many Requests |

Rate limiting is tracked in-memory (no external dependency). Counters reset on server restart. Failed attempt = invalid credential/signature/token.

Trace: TEST-020-060

#### REQ-020-061: WebSocket Ticket Authentication

Agent clients SHALL NOT pass long-lived bearer tokens as WebSocket query parameters. Instead:

1. Agent calls `POST /auth/ws-ticket` with `Authorization: Bearer <agent-token>`
2. Server validates the bearer token and issues a one-time ticket (opaque, 128-bit random, expires in 30 seconds)
3. Agent connects to `WS /ws/edit/{slug}?ticket=<one-time-ticket>`
4. Server validates the ticket, marks it as used, resolves to user identity
5. Tickets are single-use and expire after 30 seconds — they cannot be replayed

Browser clients continue to authenticate via session cookie (no ticket needed).

This prevents long-lived agent tokens from appearing in server logs, reverse proxy logs, or monitoring systems.

Trace: TEST-020-061

#### REQ-020-062: Recovery Challenge Expiry

Recovery challenges (CON-020-002) SHALL expire:

- Challenges expire 5 minutes after issuance
- Server stores `{ challenge, user_id, issued_at }` in memory
- Submissions after expiry return 410 Gone
- Maximum 3 active challenges per user (prevents challenge flooding)
- Expired challenges are pruned lazily on next request

Trace: TEST-020-062

#### REQ-020-063: Content Security Policy

All HTML responses SHALL include a Content-Security-Policy header:

```
Content-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self' wss:; frame-ancestors 'none'
```

- `frame-ancestors 'none'` prevents clickjacking
- No inline scripts allowed — all JS must be in static files
- `connect-src 'self' wss:` allows WebSocket connections to the same origin
- User-generated markdown content is rendered as HTML but cannot execute scripts due to the script-src restriction

Trace: TEST-020-063

#### REQ-020-064: CSRF Protection

All state-changing HTTP endpoints (PUT, POST, DELETE) SHALL require CSRF protection:

- The server issues a CSRF token with each session, stored in a `X-CSRF-Token` response header on page loads
- State-changing requests MUST include the CSRF token as a `X-CSRF-Token` request header
- Requests without a valid CSRF token return 403 Forbidden
- API endpoints authenticated via `Authorization: Bearer` header are exempt (bearer tokens are not automatically attached by browsers)
- WebSocket connections are exempt (established via explicit client-side code, not automatic browser requests)

Combined with `SameSite=Strict` cookies, this provides defense-in-depth against CSRF.

Trace: TEST-020-064

#### REQ-020-065: Admin Route Hardcoded Check

Routes under `/_admin/*` SHALL enforce a hardcoded role check in addition to SPL evaluation:

- The Axum middleware for `/_admin/*` routes checks `user.profile.owner == true || user.profile.roles.contains("admin")` from the user profile JSON
- This check happens BEFORE SPL evaluation — it is not defeatable by policy changes
- If the hardcoded check fails → 403 Forbidden (even if SPL would allow access)
- SPL is evaluated as an additional layer: the hardcoded check is necessary but not sufficient

This ensures that a malformed or compromised `access.spl` cannot expose admin panels to non-admin users.

Trace: TEST-020-065

#### REQ-020-066: Comment Authentication and Integrity

Page comments (REQ-020-051) SHALL be written only through authenticated API endpoints:

- `POST /{slug}/_comments` — add a comment (requires `can-edit` for the page)
- `GET /{slug}/_comments` — list comments (requires `can-read` for the page)
- Comments are stored in `.ztl/comments/<slug>.json` with integrity metadata:
  ```json
  {
    "user_id": "alice-abc123",
    "text": "Rewriting intro",
    "at": "2026-03-18T14:00:00Z",
    "hmac": "<HMAC-SHA256(server_key, user_id || text || at)>"
  }
  ```
- The HMAC prevents tampering with comment attribution if an attacker gains filesystem access
- External processes writing directly to `.ztl/comments/` will produce comments with invalid HMACs — these are displayed with a "(unverified)" badge

Trace: TEST-020-066

#### REQ-020-067: TLS Enforcement Warning

When `--collab` mode is active and the server binds to a non-loopback address:

- Emit a WARN at startup: "Collab mode without TLS: passkeys require HTTPS. Configure a reverse proxy with TLS, or bind to localhost."
- If the `ztl_INSECURE_COLLAB=1` environment variable is not set, refuse to start and exit with an error message explaining the requirement
- When bound to `127.0.0.1` or `::1`, no warning is emitted (localhost is a secure context for WebAuthn)

Trace: TEST-020-067

### 3.8 Visibility and Access Denied Behavior

#### REQ-020-030: Denied Page Visibility Mode

The system SHALL support three visibility modes for pages a user cannot access:

| Mode          | Sidebar/Search | Direct URL         | Wikilinks to denied page       |
|---------------|----------------|--------------------|--------------------------------|
| `transparent` | Visible (grayed)| 403 "Access denied"| Rendered as link (grayed out)  |
| `mixed`       | Hidden         | 403 "Access denied"| Rendered with lock indicator   |
| `hidden`      | Hidden         | 404 (indistinguishable from nonexistent) | Rendered as dead link |

**Default:** `mixed`

The default mode is set via SPL fact in the built-in policy:

```lisp
(given (visibility-mode mixed))
```

Using SPL facts for global configuration allows visibility mode to participate in the reasoning pipeline — for example, a rule could make visibility mode depend on the user's role. The `visibility-mode` fact is evaluated once per ACL cache warm-up, not per request.

Vaults MAY override this in `access.spl`:

```lisp
; Switch to fully hidden mode (sensitive page names)
(given (visibility-mode hidden))
```

Per-page visibility overrides SHALL also be supported:

```lisp
; Force-hide this page from unauthorized users (even in mixed/transparent mode)
(normally r-force-hide
  (not (can-read ?user "Secret Project"))
  (hidden-from ?user "Secret Project"))

; Force-show this page title (even in hidden mode) — content still inaccessible
(normally r-force-visible
  (not (can-read ?user "Public Roadmap"))
  (visible-title ?user "Public Roadmap"))
```

Trace: TEST-020-030

#### REQ-020-031: Sidebar and Search Filtering

When rendering the sidebar, page grid, and search results, the system SHALL filter pages based on the current user's access and the visibility mode:

1. Evaluate `(can-read ?user ?page)` for each page (from ACL cache, REQ-020-013)
2. For pages where read is denied:
   - Check for `(hidden-from ?user ?page)` → always hide
   - Check for `(visible-title ?user ?page)` → always show title (with lock icon)
   - Otherwise apply the vault's `visibility-mode`:
     - `transparent`: show in sidebar/search with grayed-out styling, no content preview in search results
     - `mixed`: hide from sidebar and search results
     - `hidden`: hide from sidebar and search results
3. The search API (`/api/search`) SHALL never return content snippets for pages the user cannot read, regardless of visibility mode

Trace: TEST-020-031

#### REQ-020-032: Wikilink Rendering for Denied Pages

When rendering a page that contains wikilinks to pages the current user cannot access:

| Visibility mode | Wikilink rendering |
|-----------------|-------------------|
| `transparent`   | Rendered as a grayed-out link with page title visible; click → 403 page |
| `mixed`         | Rendered with lock icon and generic "restricted page" tooltip; page title visible; click → 403 page |
| `hidden`        | Rendered as a dead link (same as linking to a nonexistent page); no indication the page exists |

The link graph exposed via `GET /api/graph` SHALL also be filtered: denied nodes are included as `{ "name": "(restricted)", "locked": true }` in `mixed`/`transparent` mode, or omitted entirely in `hidden` mode.

Trace: TEST-020-032

#### REQ-020-033: Access Denied Response

When a user requests a page they cannot read:

**`mixed` and `transparent` mode (403):**
```
HTTP/1.1 403 Forbidden
Content-Type: text/html

[Rendered page with vault chrome, showing:]
- Page title (visible)
- Lock icon
- "You don't have access to this page."
- If explainability is enabled for the user's role: link to /api/acl/explain
- "Request access" link (fires on-access-request hook if configured)
```

**`hidden` mode (404):**
```
HTTP/1.1 404 Not Found
Content-Type: text/html

[Standard 404 page — identical to requesting a genuinely nonexistent page]
```

Trace: TEST-020-033

---

## 4. Architecture Decisions

### ADR-020-001: Passkeys Over Passwords

**Context:** The system needs user authentication for a self-hosted wiki.

**Decision:** Use WebAuthn passkeys as the primary authentication mechanism with BIP39 mnemonic as recovery/delegation key. No passwords.

**Rationale:**
- Passkeys are phishing-resistant and eliminate credential stuffing
- BIP39 is a well-understood standard for human-readable key backup
- The mnemonic doubles as the agent delegation mechanism — one concept, two uses
- No password hashing, no password reset flow, no password policy complexity

**Trade-offs:**
- Requires a browser with WebAuthn support (all modern browsers)
- BIP39 mnemonic must be written down physically — digital storage defeats the purpose
- Recovery flow requires manual passkey re-registration

**Alternatives rejected:**
- Password + TOTP: more complex, less secure, more code to maintain
- OAuth/OIDC: requires external IdP, adds network dependency, violates local-first philosophy

### ADR-020-002: SPL as ACL Language

**Context:** The system needs access control that is auditable, composable, and fits the existing ztl philosophy.

**Decision:** Express all access control policy as SPL (Spindle defeasible logic), evaluated by the existing `spindle-core` reasoning engine.

**Rationale:**
- ztl already integrates SPL for reasoning (SPEC-005) — zero new language to learn
- Defeasible rules naturally model permission overrides (page-level rules defeat vault-level defaults)
- Superiority relations make conflict resolution explicit and traceable
- `why_not()` query operator provides built-in explainability for denied access
- Access policy is versioned in git alongside the content it protects
- Temporal reasoning (Allen algebra) enables time-bounded access without custom expiry logic
- Modal operators (`[P]`, `[F]`, `[O]`) map cleanly to permission/prohibition/obligation semantics
- Trust decay enables automatic permission degradation for stale delegations

**Trade-offs:**
- SPL reasoning has non-trivial cost per evaluation → mitigated by ACL cache (REQ-020-013)
- Policy errors (e.g., circular superiority) could lock users out → mitigated by built-in defaults that are strict rules (cannot be defeated)
- Requires users to understand SPL to write custom policies → mitigated by templates and explainability endpoint

**Alternatives rejected:**
- RBAC config file: simpler but not composable; cannot do page-level overrides without a separate system
- OPA/Rego: powerful but adds a foreign policy language and runtime dependency
- Casbin: good library but string-based matchers lack formal semantics and explainability

### ADR-020-003: Git Auto-Commit via libgit2

**Context:** Edits need to be tracked with attribution in the vault's version history.

**Decision:** Use `git2` (libgit2 Rust bindings) to commit directly after each save. One save = one commit.

**Rationale:**
- No dependency on `git` binary being installed
- Direct library calls are faster than shell-out
- `git2` supports author/committer distinction — agent commits attributed to the delegating user
- The vault's git log becomes a complete, user-attributed audit trail
- Compatible with existing jj snapshot system (SPEC-017) — jj can track the git repo

**Trade-offs:**
- One commit per save can produce noisy history → mitigated by squash workflows outside ztl
- libgit2 does not support all git features (e.g., partial clone) → not needed for local vaults
- Concurrent saves must serialize git commits → use a write lock on the repo
- jj uses colocated git mode when `.git/` exists (SPEC-017). After each git2 commit, the system MUST call `jj git import` (via jj-lib) to synchronize jj's understanding of the git state. Without this step, jj's internal view of branches and commits diverges from the git repository, causing snapshot and history operations to fail or produce incorrect results.

**Alternatives rejected:**
- Shell out to `git` CLI: works but slower, binary dependency, error handling harder
- Custom append-only log: not standard, not interoperable, duplicates VCS functionality
- jj-only (no git): jj is already used for snapshots but git is ubiquitous for collaboration

### ADR-020-004: Invitation Tokens Over Self-Registration

**Context:** New users need to be onboarded to a collaborative vault.

**Decision:** New accounts can only be created by accepting a signed invitation token generated by an existing user with `can_invite` permission.

**Rationale:**
- Prevents unauthorized access to private vaults
- Invitation flow naturally maps to SPL: `(can-invite ?user)` is itself a defeasible conclusion
- Inviter can constrain invitee's initial role and page scope
- Single-use nonces prevent token replay
- JWT format is compact, URL-safe, and self-describing

**Trade-offs:**
- No self-service registration — requires an existing user to initiate
- Lost invitation tokens must be regenerated (no "resend" — just create a new one)
- Bootstrap requires CLI access to create the first owner

**Alternatives rejected:**
- Open registration with approval queue: more complex, exposes registration endpoint to attackers
- Shared secret/passphrase: weaker security, no per-invitee scoping
- Manual account creation by admin only: poor UX, doesn't scale

### ADR-020-005: Peritext CRDT for Collaborative Editing

**Context:** Multiple users need to edit the same wiki page concurrently. Hot documents (project pages, incident runbooks, meeting notes) are the norm in a team knowledge graph — contention on the most valuable pages is expected, not exceptional.

**Decision:** Use the Peritext CRDT algorithm as the live editing layer. Markdown files on disk remain the at-rest format; the CRDT is the live editing representation, serialized back to markdown on save.

**Rationale:**
- Peritext is specifically designed for rich text CRDT merging — it solves the interleaving problems that make plain-text CRDTs break markdown formatting across concurrent edits
- Async-first: designed for independent copies that merge, not real-time OT. Aligns with ztl's local-first philosophy
- `addMark`/`removeMark` operations map directly to markdown inline formatting (`**`, `*`, `` ` ``, `[[...]]`)
- `opId`-based anchoring means cursor positions and formatting spans are stable under concurrent insertions and deletions
- Growth behavior (inclusive vs non-growing) correctly models the difference between "typing at the end of bold text stays bold" and "typing after a wikilink is plain text"
- Tombstone-based deletion preserves merge correctness even when characters are deleted and re-inserted concurrently

**Trade-offs:**
- Peritext handles inline formatting only; block-level structure (headings, lists, code fences) requires separate handling (REQ-020-026 treats blocks as atomic tokens)
- CRDT state is larger than plain text — memory overhead per loaded document. Mitigated by eviction TTL and max concurrent document limit (REQ-020-029)
- Canonical markdown serialization must be carefully defined to ensure clean git diffs (REQ-020-027)
- The reference implementation is TypeScript (Automerge-based). ztl implements Peritext's semantics natively in Rust over `diamond-types = "1.0"`: diamond-types provides the text oplog, and a project-owned sibling marks oplog (`MarksDoc`) carries `Mark`/`Unmark`/`Shift` span ops with per-span `ExpandMark` — see "Marks Layer Architecture" under §3.7. This replaces the earlier plan to lean on `automerge-rs` `RichText`, whose rich-text API churned across 0.5/0.6 and did not give us stable Peritext boundary-growth semantics

**Alternatives rejected:**
- Soft page locks (one writer at a time): creates contention on the most valuable pages — exactly the wrong trade-off for a team wiki
- Plain-text CRDT on markdown source (Yjs `Text`, automerge `Text`, or diamond-types with no marks layer): concurrent formatting edits produce broken markdown — the exact problem Peritext was designed to solve, and the reason ztl layers an explicit marks oplog over diamond-types rather than treating markdown source as flat text
- OT (Operational Transform): requires a central server for total ordering; more complex to implement correctly; does not support offline/async editing

---

## 5. Contracts

### CON-020-001: User Profile Schema

Stored at `.ztl/users/<user-id>/profile.json`:

```json
{
  "id": "alice-a1b2c3d4",
  "name": "Alice",
  "created_at": "2026-03-18T10:00:00Z",
  "invited_by": null,
  "owner": true,
  "credentials": [
    {
      "credential_id": "<base64url>",
      "public_key": "<base64url>",
      "sign_count": 42,
      "created_at": "2026-03-18T10:00:00Z",
      "label": "MacBook Pro"
    }
  ],
  "recovery_pubkey": "<base64url-ed25519-public-key>"
}
```

- `id`: 8 random hex chars prefixed by slugified display name
- `invited_by`: user ID of the inviter, or `null` for the bootstrap owner
- `credentials`: array of WebAuthn credentials (supports multiple passkeys)
- `recovery_pubkey`: ed25519 public key derived from BIP39 mnemonic

Implements: REQ-020-001, REQ-020-002, REQ-020-005

### CON-020-002: BIP39 Recovery Flow

```
POST /auth/recover
Content-Type: application/json

{
  "user_id": "alice-a1b2c3d4",
  "challenge_response": "<signed-challenge-base64url>"
}
```

Flow:
1. `GET /auth/recover?user=<id>` → returns a random challenge (256-bit nonce)
2. Client signs the challenge with the ed25519 private key derived from the mnemonic
3. `POST /auth/recover` with the signed challenge
4. Server verifies signature against stored `recovery_pubkey`
5. On success → issue session → redirect to passkey registration page

Implements: REQ-020-002

### CON-020-003: Agent Token Format

```
Token = base64url(user_id_bytes || generation_byte || ed25519_sign(private_key, "ztl-agent-v1-" || user_id || generation))
```

- Total: 16 bytes (user_id) + 1 byte (generation) + 64 bytes (signature) = 81 bytes → 108 base64url characters
- Presented via `Authorization: Bearer <token>` header or `ztl_USER_TOKEN` env var

Server verification:
1. Decode token → extract user_id (first 16 bytes), generation (byte 17), and signature (remaining 64 bytes)
2. Load `recovery_pubkey` from `.ztl/users/<user_id>/profile.json`
3. Verify `ed25519_verify(pubkey, "ztl-agent-v1-" || user_id || generation, signature)`
3a. Verify that generation matches `agent_token_generation` in the user profile
4. On success → resolve to user identity

Implements: REQ-020-004, REQ-020-019

### CON-020-004: Invitation Token (JWT)

```json
{
  "header": {"alg": "EdDSA", "typ": "JWT"},
  "payload": {
    "iss": "alice-a1b2c3d4",
    "sub": "ztl-invite",
    "role": "editor",
    "pages": "projects/*",
    "exp": 1742515200,
    "nonce": "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6"
  }
}
```

- Signed with server's ed25519 key (`.ztl/collab/server.key`)
- `role`: one of `reader`, `editor`, `admin`
- `pages`: optional glob pattern constraining initial scope
- `nonce`: 128-bit random, tracked in `.ztl/collab/used-nonces.json` to enforce single-use

Implements: REQ-020-006, REQ-020-007

### CON-020-005: ACL Evaluation Contract

**Input:**
```rust
struct AclQuery {
    user_id: String,
    page_slug: String,
    action: Action,       // Read | Edit
    is_agent: bool,
    now_epoch_ms: i64,
}
```

**Process:**
1. Load SPL from: built-in defaults + `access.spl` + target page's SPL blocks
2. Inject runtime facts:
   ```lisp
   (given (authenticated "<user_id>"))
   (given (requesting "<user_id>" "<page_slug>" "<action>"))
   (given (now <epoch_ms>))
   ```
3. If `is_agent`: `(given (is-agent "<user_id>"))`
4. Load user's roles from profile → inject as facts: `(given (role "<user_id>" <role>))`
5. Combine into theory, ground, reason
6. Check conclusion for `(can-<action> "<user_id>" "<page_slug>")`

**Output:**
```rust
enum AclDecision {
    Allowed { tag: ConclusionTag, rule_trace: Vec<RuleRef> },
    Denied  { tag: ConclusionTag, rule_trace: Vec<RuleRef> },
}
```

Implements: REQ-020-009, REQ-020-010

### CON-020-006: Git Commit Contract

On each successful save:

```rust
fn auto_commit(
    repo: &git2::Repository,
    file_path: &Path,       // relative from vault root
    user_name: &str,
    user_id: &str,
    message: Option<&str>,  // from X-Commit-Message header
) -> Result<git2::Oid, CollabError>
```

- Author signature: `"{user_name} <{user_id}@vault>"`
- Default message: `"edit: {page_name}"`
- Commits to current HEAD (no branching)
- Uses a `Mutex<git2::Repository>` in `WebState` to serialize concurrent commits

Implements: REQ-020-015

### CON-020-007: Agent API Response Format

All `/api/*` endpoints return JSON with consistent envelope:

```json
{
  "ok": true,
  "data": { ... },
  "error": null
}
```

Error responses:

```json
{
  "ok": false,
  "data": null,
  "error": {
    "code": "ACL_DENIED",
    "message": "can-edit(bob, Secret Project) was denied",
    "proof": [ ... ]
  }
}
```

Error codes: `ACL_DENIED`, `NOT_FOUND`, `CONFLICT`, `INVALID_TOKEN`, `SESSION_EXPIRED`, `INVALID_REQUEST`, `INTERNAL_ERROR`.

Implements: REQ-020-018

### CON-020-008: Peritext CRDT Document Model

```rust
/// A live CRDT document held in memory while being edited
struct CrdtDocument {
    /// Page slug this document represents
    slug: String,
    /// CRDT state: a diamond-types text oplog paired with a sibling
    /// marks oplog (see §3.7 "Marks Layer Architecture"). Characters
    /// carry DT op-ids; mark spans are Peritext-style Mark/Unmark/Shift
    /// ops replayed in DT's canonical merge order.
    state: DiamondCrdtDocument,
    /// Connected editing sessions
    sessions: HashMap<SessionId, CrdtSession>,
    /// Last edit timestamp (for quiescence flush)
    last_edit: Instant,
    /// Last flush timestamp (for dirty detection)
    last_flush: Instant,
    /// Whether state has diverged from on-disk markdown
    dirty: bool,
}

struct CrdtSession {
    user_id: String,
    cursor: Option<OpId>,
    selection: Option<(OpId, OpId)>,
    connected_at: Instant,
}
```

**Markdown → CRDT parsing rules:**
1. Split document into blocks (frontmatter, headings, paragraphs, code fences, lists, etc.)
2. Each block boundary becomes an atomic block token in the CRDT sequence
3. Within paragraph/inline blocks, parse markdown inline syntax into Peritext marks:
   - `**text**` → `addMark("bold", start, end)` with inclusive growth
   - `[[target|alias]]` → `addMark("wikilink", start, end, {target, alias})` with non-growing
   - (full mapping per REQ-020-025)
4. Each character and block token gets a unique `opId` (Lamport timestamp @ node ID)

**CRDT → Markdown serialization rules:**
1. Iterate CRDT sequence, collecting active marks at each position
2. At block token boundaries, emit the corresponding markdown block syntax
3. Within inline content, emit opening/closing markdown syntax for mark transitions
4. Mark nesting order (outermost to innermost): strikethrough, bold, italic, code, highlight
5. Wikilinks emit as `[[target]]` or `[[target|alias]]`
6. Normalize whitespace: single blank line between blocks, no trailing whitespace, newline at EOF

Implements: REQ-020-024, REQ-020-025, REQ-020-026, REQ-020-027

---

## 6. Test Specifications

### TEST-020-001: Passkey Registration and Login

**Scenario:** User registers a passkey and authenticates.
**Precondition:** Bootstrap owner created via `--init-owner`.
**Steps:**
1. GET `/auth/bootstrap` → receive WebAuthn registration challenge
2. Simulate passkey creation (mock authenticator)
3. POST credential to server → 200, session cookie set
4. GET `/` with session cookie → 200 (authenticated homepage)
5. Clear cookie → GET `/` → 401

### TEST-020-002: BIP39 Recovery

**Scenario:** User recovers access using their mnemonic.
**Precondition:** User exists with registered passkey and stored recovery pubkey.
**Steps:**
1. GET `/auth/recover?user=<id>` → challenge
2. Sign challenge with ed25519 key derived from mnemonic
3. POST signed challenge → 200, session issued
4. Register new passkey → stored alongside (or replacing) original

### TEST-020-003: Session Expiry

**Scenario:** Idle session expires after timeout.
**Steps:**
1. Authenticate → receive session
2. Advance server clock beyond idle timeout
3. GET `/` with session cookie → 401

### TEST-020-004: Agent Token Authentication

**Scenario:** Agent authenticates with derived token.
**Steps:**
1. Derive token from user's mnemonic via `ztl agent-token`
2. GET `/api/pages` with `Authorization: Bearer <token>` → 200
3. Verify response attributes actions to the correct user
4. Use an invalid token → 401

### TEST-020-005: Bootstrap Owner

**Scenario:** First user creation via CLI.
**Steps:**
1. `ztl serve --collab --init-owner "Alice"` → BIP39 mnemonic printed to stderr
2. Verify `.ztl/users/` contains exactly one user with `owner: true`
3. Attempt `--init-owner` again → error ("owner already exists")

### TEST-020-006: Invitation Generation

**Scenario:** Admin generates an invitation.
**Steps:**
1. `ztl invite --as alice --role editor --pages "projects/*"` → invitation URL
2. Decode JWT → verify `iss`, `role`, `pages`, `exp`, `nonce`
3. Verify JWT is signed with server key

### TEST-020-007: Invitation Acceptance

**Scenario:** New user accepts invitation.
**Steps:**
1. GET invitation URL → registration page (200)
2. Register passkey + receive mnemonic
3. Verify new user profile created with correct `invited_by`, role
4. Verify SPL facts injected into `access.spl`
5. Re-use same invitation URL → 410 Gone (nonce consumed)

### TEST-020-008: Invitation ACL

**Scenario:** Non-admin cannot invite admin-level users.
**Steps:**
1. Editor user attempts `ztl invite --as bob --role admin` → denied by SPL
2. Owner user attempts same → succeeds

### TEST-020-009: SPL Authorization — Basic

**Scenario:** Read/edit access governed by SPL policy.
**Steps:**
1. Set up vault with access policy granting Bob `editor` on `projects/*`
2. Bob GET `/projects/roadmap` → 200
3. Bob PUT `/projects/roadmap` → 200
4. Bob GET `/secret/internal` → 403
5. Bob PUT `/secret/internal` → 403

### TEST-020-010: SPL Authorization — Page Override

**Scenario:** Page-level SPL defeats vault-level policy.
**Steps:**
1. Vault policy grants all editors read access
2. Page `Secret.md` contains SPL defeater blocking reads for non-members
3. Editor (non-member) GET `/secret` → 403
4. Editor (member) GET `/secret` → 200

### TEST-020-011: Temporal Access Control

**Scenario:** Time-bounded access expires.
**Steps:**
1. Policy grants access during interval `[T1, T2]`
2. At time T1+1 → access allowed
3. At time T2+1 → access denied

### TEST-020-012: Deontic Modalities

**Scenario:** `[F]` prohibition blocks access regardless of other permissions.
**Steps:**
1. Policy has `[P](edit user page)` and `[F](edit user page)` with `[F]` preferred
2. User attempts edit → 403

### TEST-020-013: ACL Cache Invalidation

**Scenario:** Cache updates when vault changes.
**Steps:**
1. Bob can read page X (cached)
2. Admin edits `access.spl` to revoke Bob's access
3. Bob's next request for page X → 403 (cache invalidated by merkle change)

### TEST-020-014: Authorization Explainability

**Scenario:** Admin queries why Bob cannot edit a page.
**Steps:**
1. GET `/api/acl/explain?user=bob&page=secret&action=edit` → JSON with proof trace
2. Verify proof includes the defeating rule and superiority relation

### TEST-020-015: Auto-Commit on Save

**Scenario:** Save creates attributed git commit.
**Steps:**
1. Alice saves page via PUT
2. Inspect `git log -1` → author is "Alice <alice-xxx@vault>", message is "edit: Page Name"
3. Verify file content matches saved content

### TEST-020-016: CRDT Conflict Resolution

**Scenario:** Concurrent edits merge via Peritext CRDT.
**Steps:**
1. Alice and Bob both connect to WS `/ws/edit/roadmap`
2. Alice inserts "Phase 1" at position X; Bob inserts "Phase 2" at position X concurrently
3. Both operations are merged by the server — both insertions appear, ordered deterministically by opId
4. Both clients converge to identical document state
5. Alice bolds "Phase 1"; Bob bolds "Phase 2" concurrently → both bold spans preserved after merge

**Scenario:** API write merges into active CRDT session.
**Steps:**
1. Alice is editing "Roadmap" via WebSocket
2. Agent submits `PUT /roadmap` with updated markdown
3. Server parses agent's markdown into CRDT ops, merges into live document
4. Alice's editor receives the merged operations — no data loss

### TEST-020-017: Page Creation via API

**Scenario:** Agent creates a new page.
**Steps:**
1. PUT `/new-page` with `X-Create: true` and markdown body → 201
2. Verify file exists at `new-page.md`
3. Verify git commit with agent's delegating user as author

### TEST-020-018: Agent API Endpoints

**Scenario:** Agent lists pages and reads content.
**Steps:**
1. GET `/api/pages` → JSON array of page objects
2. GET `/api/pages/architecture` → raw markdown content
3. POST `/api/index` → 200, vault re-indexed

### TEST-020-019: Agent Token Derivation

**Scenario:** CLI generates valid agent token.
**Steps:**
1. `ztl agent-token --mnemonic "<words>"` → token on stdout
2. Use token to authenticate → resolves to correct user

### TEST-020-020: Agent Loop Prevention

**Scenario:** Agent save does not trigger infinite hook chain.
**Steps:**
1. Configure `on-save` hook that invokes an agent
2. Agent saves a page → hook fires with `ztl_HOOK_DEPTH=1`
3. Hook checks depth, does not invoke agent again
4. Alternatively: agent saves with `X-No-Hooks: true` → no hook fires

### TEST-020-021: Agent SPL Constraints

**Scenario:** Agent cannot edit access control.
**Steps:**
1. SPL policy includes `(except d-agent-no-acl ...)` defeating agent edit on "access"
2. Agent attempts PUT `/access-control` → 403
3. Agent attempts PUT `/index` (assigned page) → 200

### TEST-020-022: User Identity in Hook Context

**Scenario:** Hook receives user identity.
**Steps:**
1. Alice saves a page
2. `on-save` hook reads stdin JSON → `saved.user.name == "Alice"`, `saved.user.is_agent == false`

### TEST-020-023: Agent Hook Lifecycle

**Scenario:** `on-agent` hook is triggered.
**Steps:**
1. `ztl agent run curator` → `on-agent` hook fires
2. Hook receives context with `task: "curator"`, `user` identity
3. Hook exits 0 → success logged

### TEST-020-024: Peritext CRDT Editing

**Scenario:** Two users edit the same page concurrently with formatting.
**Steps:**
1. Alice connects to WS `/ws/edit/architecture`; receives `sync` with CRDT state
2. Bob connects to same page; receives identical `sync`
3. Alice types "important" and bolds it; Bob types "note" and italicizes it — at different positions
4. Both receive each other's operations via `op` messages
5. Both clients show "**important**" and "*note*" in the correct positions
6. After 5s quiescence → server flushes to markdown → git commit
7. On-disk `.md` contains canonical markdown with both edits

**Scenario:** Wikilink mark has non-growing behavior.
**Steps:**
1. Alice creates `[[Project X]]` wikilink
2. Bob types " is great" immediately after the closing `]]`
3. " is great" is plain text, not part of the wikilink

**Scenario:** Presence and cursors.
**Steps:**
1. Alice connects to WS `/ws/edit/roadmap` → receives current presence list
2. Bob connects → Alice receives `{"type": "presence", "user": "bob", "cursor": {...}}`
3. Bob types → Alice sees Bob's cursor position update in real time
4. Bob disconnects → Alice receives `{"type": "presence", "user": "bob", "action": "left"}`

### TEST-020-025: Mark Type Mapping

**Scenario:** Markdown formatting round-trips through CRDT.
**Steps:**
1. Load `**bold** and *italic* and [[Link]]` into CRDT
2. Verify three marks: bold (inclusive), italic (inclusive), wikilink (non-growing)
3. Serialize back to markdown → byte-identical output

### TEST-020-026: Block-Level Atomicity

**Scenario:** Block boundaries are atomic in CRDT.
**Steps:**
1. Document contains `## Heading` followed by paragraph
2. Alice edits paragraph; Bob edits heading text concurrently
3. Both edits merge cleanly — heading prefix `## ` remains intact

**Scenario:** SPL code fence is opaque.
**Steps:**
1. Document contains `` ```spl `` block with access control rules
2. User edits inside the code fence — no formatting marks applied
3. Serialized markdown preserves exact SPL syntax

### TEST-020-027: Canonical Serialization

**Scenario:** Deterministic markdown output from CRDT state.
**Steps:**
1. Alice and Bob both make edits to the same document
2. Server serializes CRDT state to markdown
3. A second serialization of the same state produces byte-identical output
4. Git diff shows clean, readable changes

### TEST-020-028: WebSocket Editing Protocol

**Scenario:** WebSocket lifecycle for editing.
**Steps:**
1. Alice connects to WS `/ws/edit/roadmap` with session cookie → receives `sync` message with CRDT state
2. Alice sends `op` (insert character) → server broadcasts to other clients
3. Bob connects → receives `sync` with current state including Alice's edit
4. Bob sends `op` → Alice receives it
5. Alice sends `cursor` update → Bob receives presence update
6. Alice sends `save` → server flushes, broadcasts `saved` with commit hash
7. Alice disconnects → Bob receives `presence.left`

**Scenario:** Agent connects via WebSocket with one-time ticket.
**Steps:**
1. Agent POSTs to `/auth/ws-ticket` with bearer token → receives one-time ticket
2. Agent connects to WS `/ws/edit/roadmap?ticket=<one-time-ticket>` → receives `sync`
3. Agent sends operations → merged and broadcast normally

### TEST-020-029: CRDT State Management

**Scenario:** CRDT eviction after all clients disconnect.
**Steps:**
1. Alice opens editor for "Notes" → CRDT loaded
2. Alice disconnects → CRDT remains in memory (TTL 10 min)
3. Bob connects within 10 min → receives existing CRDT state (no reload from disk)
4. Nobody connects for 10 min → CRDT evicted (flushed to disk if dirty)
5. Carol connects → CRDT loaded fresh from disk

**Scenario:** Memory bound enforced.
**Steps:**
1. Open editors for 50 different pages (max concurrent)
2. Open editor for 51st page → least-recently-used CRDT evicted (flushed if dirty)
3. Evicted page's editors receive `sync` with reloaded state

### TEST-020-030: Visibility Modes

**Scenario:** Mixed mode (default) hides denied pages from sidebar but shows 403 on direct access.
**Steps:**
1. Bob has `editor` role but cannot read "Secret Project"
2. Bob loads homepage → sidebar does NOT list "Secret Project"
3. Bob searches for "Secret" → no results
4. Bob navigates directly to `/secret-project` → 403 with lock icon and page title
5. Bob views a page that links to "Secret Project" → wikilink rendered with lock icon and "restricted page" tooltip

**Scenario:** Hidden mode returns 404 for denied pages.
**Steps:**
1. Vault sets `(given (visibility-mode hidden))` in `access.spl`
2. Bob navigates to `/secret-project` → 404 (identical to nonexistent page)
3. Bob views a page linking to "Secret Project" → rendered as dead link (no lock, no hint)

**Scenario:** Transparent mode shows denied pages grayed out.
**Steps:**
1. Vault sets `(given (visibility-mode transparent))`
2. Bob loads homepage → sidebar lists "Secret Project" grayed out
3. Bob clicks it → 403 with access denied message

**Scenario:** Per-page visibility override.
**Steps:**
1. Vault uses `mixed` mode (default)
2. "Secret Project" has SPL: `(normally r-force-hide ... (hidden-from ?user "Secret Project"))`
3. Bob navigates to `/secret-project` → 404 (not 403, because force-hidden)
4. "Public Roadmap" has SPL: `(normally r-force-visible ... (visible-title ?user "Public Roadmap"))`
5. Bob sees "Public Roadmap" in sidebar with lock icon despite `mixed` mode hiding it by default

### TEST-020-031: Sidebar and Search Filtering

**Scenario:** Search never leaks content snippets.
**Steps:**
1. "Secret Project" contains the text "launch date is March 30"
2. Bob (cannot read "Secret Project") searches for "launch date"
3. Results do NOT include "Secret Project" or any snippet from it, regardless of visibility mode

### TEST-020-032: Wikilink Rendering

**Scenario:** Wikilinks to denied pages render per visibility mode.
**Steps:**
1. Page "Overview" contains `[[Secret Project]]` and `[[Public Page]]`
2. Bob can read "Public Page" but not "Secret Project"
3. In `mixed` mode: "Public Page" is a normal link; "Secret Project" shows lock icon
4. In `hidden` mode: "Secret Project" rendered as dead link (red, same as nonexistent)
5. In `transparent` mode: "Secret Project" rendered as grayed-out link

### TEST-020-033: Link Graph Filtering

**Scenario:** API graph respects visibility mode.
**Steps:**
1. GET `/api/graph` as Bob (cannot read "Secret Project")
2. In `mixed` mode: "Secret Project" node present as `{"name": "(restricted)", "locked": true}`
3. In `hidden` mode: "Secret Project" node omitted entirely
4. Edges to/from the denied node handled accordingly

### TEST-020-034: CRDT Flush Pipeline

**Scenario:** CRDT flush triggers full vault pipeline.
**Steps:**
1. Alice opens editor for "Roadmap" → CRDT loaded
2. Alice types several edits → no disk write yet (quiescence timer running)
3. 5 seconds pass with no edits → CRDT flushes
4. Verify: `.md` file updated on disk
5. Verify: `ParsedFile` re-scanned with fresh merkle leaves
6. Verify: `vault_root_hash` recomputed
7. Verify: git commit created with Alice as author
8. Verify: jj snapshot created (or deduped if hash unchanged)
9. Verify: search index updated
10. Verify: `on-save` hook fired with correct context

### TEST-020-035: Access Policy Immediate Flush

**Scenario:** Editing access.spl flushes immediately.
**Steps:**
1. Admin opens `access.spl` in editor → CRDT loaded
2. Admin adds `(given (role dave reader))`
3. Within 500ms → CRDT flushes (no quiescence delay)
4. Dave can now read pages immediately (ACL cache refreshed)

**Scenario:** SPL parse error during access policy edit.
**Steps:**
1. Admin edits `access.spl`, introduces syntax error
2. Flush triggers → SPL parse fails
3. ACL falls back to previous valid policy (not built-in defaults)
4. Admin receives WebSocket message: `{"type": "spl-error", "message": "..."}`
5. Other users' access is unaffected

### TEST-020-036: Historical Queries and CRDT State

**Scenario:** History reflects flushed state, not in-flight edits.
**Steps:**
1. "Roadmap" last flushed at T1 with content "v1"
2. Alice is actively editing in CRDT — current CRDT content is "v2" (unflushed)
3. Bob runs `ztl links Roadmap --at now` → sees "v1" state
4. GET `/api/pages/roadmap` → returns "v1" markdown with header `X-CRDT-Dirty: true`
5. Alice stops typing → quiescence flush at T2 writes "v2"
6. Bob runs query again → sees "v2" state

### TEST-020-037: API Conflict Detection with Active CRDT

**Scenario:** Agent detects CRDT divergence.
**Steps:**
1. "Roadmap" on disk has merkle hash H1
2. Alice opens editor → CRDT loaded from H1 state
3. Alice makes edits in CRDT (unflushed) — effective hash is now H2
4. Agent sends PUT `/roadmap` with `If-Match: H1`
5. Server computes merkle hash of current CRDT serialization → H2
6. H1 ≠ H2 → 409 Conflict with CRDT-serialized content in response body

### TEST-020-038: SPL Dual Hashing on Flush

**Scenario:** SPL whitespace edit doesn't trigger theory rebuild.
**Steps:**
1. Page contains SPL block with access rules
2. User edits whitespace inside the SPL block (adds blank line)
3. CRDT flushes → re-scan computes dual hashes
4. `content_hash` changed, but `ast_hash` unchanged
5. Reasoning cache NOT invalidated — no theory rebuild

**Scenario:** SPL semantic edit triggers rebuild.
**Steps:**
1. User changes `(normally r1 bird flies)` to `(normally r1 bird swims)`
2. CRDT flushes → re-scan computes dual hashes
3. Both `content_hash` and `ast_hash` changed
4. Reasoning cache invalidated → theory rebuilt on next query

### TEST-020-039: Filesystem Watch for External Edits

**Scenario:** Agent edits a file directly on disk while ztl serve is running.
**Steps:**
1. ztl serve is running with "Roadmap" page indexed
2. External process writes new content to `Roadmap.md` on disk
3. Within 1 second → ztl detects filesystem change
4. Verify: vault re-scanned, merkle tree recomputed
5. Verify: search index updated with new content
6. Verify: link graph updated if wikilinks changed
7. Verify: jj snapshot created
8. GET `/roadmap` returns the externally-modified content

### TEST-020-040: CRDT Reconciliation on External Edit

**Scenario:** External edit while CRDT session is clean.
**Steps:**
1. Alice has "Roadmap" open in editor (CRDT loaded, no pending edits)
2. Agent writes new content to `Roadmap.md` on disk
3. Alice's editor receives `sync` with updated content
4. Alice sees the new content without losing her cursor position

**Scenario:** External edit while CRDT session is dirty.
**Steps:**
1. Alice is actively editing "Roadmap" in CRDT (unflushed edits)
2. Agent writes new content to `Roadmap.md` on disk
3. Server parses external content into CRDT, merges with Alice's edits
4. Both Alice's edits and the agent's edits are preserved (CRDT merge)
5. Alice's editor receives merged operations
6. On quiescence → flush produces markdown containing both contributions

**Scenario:** External edit deletes a file with active CRDT.
**Steps:**
1. Alice is editing "Temp Notes" in CRDT with unflushed edits
2. External process deletes `Temp Notes.md`
3. Alice receives `{"type": "deleted", "page": "Temp Notes"}`
4. Verify: `.ztl/recovery/temp-notes.md` contains Alice's unflushed edits
5. Verify: CRDT session evicted

### TEST-020-041: Git-Based External Change Detection

**Scenario:** Agent pushes commits to the repo.
**Steps:**
1. ztl serve is running, HEAD is at commit A
2. Agent runs `git pull` → HEAD advances to commit B (adds new page, modifies existing)
3. Within 30 seconds (git poll interval) → ztl detects HEAD change
4. Verify: new page appears in sidebar and search
5. Verify: modified page content updated

**Scenario:** Filesystem events missed, git poll catches up.
**Steps:**
1. Simulate missed filesystem event (e.g., NFS sync delay)
2. Git poll detects HEAD change → reconciliation pipeline runs
3. Vault state converges to current git state

### TEST-020-042: External Edit Attribution

**Scenario:** External git commit attributed to correct author.
**Steps:**
1. Agent commits as "Bot <bot@ci>" and pushes
2. ztl detects external commit
3. `on-save` hook fires with `{"user": {"name": "Bot", "id": "external:bot@ci", "is_external": true}}`

**Scenario:** Uncommitted external file edit.
**Steps:**
1. External process writes file without committing
2. `on-save` hook fires with `{"user": {"name": "(external)", "id": "external:filesystem", "is_external": true}}`

### TEST-020-043: ACL Violation Detection on External Edit

**Scenario:** External edit modifies access policy.
**Steps:**
1. Agent modifies `access.spl` directly on disk
2. ztl detects change → ACL recomputed immediately
3. Log contains WARN: "access policy modified externally"
4. New policy takes effect

**Scenario:** External edit violates ACL.
**Steps:**
1. "Secret Project" page has ACL restricting edits to admins
2. External agent (non-admin) writes to `Secret Project.md` on disk
3. ztl detects change → evaluates ACL → violation detected
4. Log contains WARN with violation details
5. `on-acl-violation` hook fires (if configured)
6. Admin sees violation banner in UI
7. File content is NOT reverted (data preservation)

### TEST-020-045: Recovery Phrase UX
**Scenario:** New user sees recovery phrase during invitation acceptance.
**Steps:**
1. User accepts invitation → recovery phrase screen displayed
2. Verify: 12 words shown in numbered list
3. Verify: "I have saved my recovery phrase" checkbox is unchecked, proceed button is disabled
4. Check the checkbox → proceed button enabled
5. User proceeds → redirected to vault homepage

### TEST-020-046: Passkey Registration Guidance
**Scenario:** User sees guidance during passkey registration.
**Steps:**
1. User reaches passkey registration step → guidance text displayed
2. Passkey registration fails → clear error with "Try again" button
3. User retries → registration succeeds

### TEST-020-047: Access Request Flow
**Scenario:** User requests access to a restricted page.
**Steps:**
1. Bob visits `/secret-project` → 403 with "Request access" button
2. Bob clicks "Request access" → request recorded
3. Alice (admin) receives WebSocket notification
4. Alice sees request in dashboard → edits access.spl to grant Bob access
5. Bob can now read the page

### TEST-020-048: Permission Management UI
**Scenario:** Admin changes a user's role via web UI.
**Steps:**
1. Alice visits `/_admin/permissions` → sees user list with roles
2. Alice changes Bob's role from reader to editor → preview shows SPL diff
3. Alice confirms → `access.spl` updated, git committed
4. Bob can now edit pages in his scope

### TEST-020-049: User Dashboard
**Scenario:** User views their dashboard.
**Steps:**
1. Bob visits `/_me` → sees recent edits, accessible pages, role summary
2. Role summary shows "editor with access to projects/*"
3. Recent edits list shows Bob's last 5 page edits with timestamps

### TEST-020-050: Web-Based Invitation
**Scenario:** Admin invites a user via web UI.
**Steps:**
1. Alice visits `/_admin/invite` → sees invitation form
2. Alice fills in role=editor, scope=projects/*, expiry=72h → clicks "Generate"
3. Invitation URL displayed with "Copy link" button
4. Alice can see the pending invitation in the list

### TEST-020-051: Page Comments
**Scenario:** Users coordinate via comments while editing.
**Steps:**
1. Alice opens "Roadmap" → comment sidebar visible
2. Alice adds comment: "Rewriting intro section"
3. Bob (viewing same page) sees Alice's comment appear in real time
4. After 30 days → comment auto-pruned

### TEST-020-052: Agent Merge Notification
**Scenario:** Agent edit merged into active CRDT session.
**Steps:**
1. Alice is editing "Index" in browser
2. Agent writes to "Index" via git → external edit merged into CRDT
3. Alice sees banner: "[Bot] made changes — 2 paragraphs added"
4. Changed ranges briefly highlighted in editor

### TEST-020-053: Page History UI
**Scenario:** User views and restores page history.
**Steps:**
1. Alice visits `/roadmap/_history` → sees chronological list of edits
2. Each entry shows author, timestamp, message
3. Alice clicks "View this version" on an older entry → sees rendered content
4. Alice clicks "Restore this version" → new commit created reverting to that version

### TEST-020-054: Visibility Explanations
**Scenario:** User encounters locked page with helpful context.
**Steps:**
1. Bob sees lock icon on wikilink → tooltip says "This page is restricted. Click to request access."
2. Bob clicks → 403 page shows "Contact Alice to request access"
3. Bob tries to create a page with the same name as a hidden page → warning displayed

### TEST-020-055: Agent Token Revocation
**Scenario:** Admin revokes an agent's token.
**Steps:**
1. Agent authenticates with token (generation=0) → succeeds
2. Admin bumps agent_token_generation to 1
3. Agent retries with old token (generation=0) → 401
4. Agent derives new token with generation=1 → succeeds

### TEST-020-056: Mnemonic Display Security
**Scenario:** Mnemonic page has security headers.
**Steps:**
1. Accept invitation → mnemonic page served
2. Verify: CSP header present, no-robots meta tag
3. Wait 5 minutes → mnemonic cleared from DOM
4. Reload → server returns error (one-time display)

### TEST-020-057: Server Key Protection
**Scenario:** Server key has correct permissions.
**Steps:**
1. `ztl serve --collab --init-owner "Alice"` → server.key created
2. Verify: file permissions are 0600
3. Verify: `.gitignore` includes `.ztl/collab/`, `.ztl/users/`, `.ztl/sessions/`

### TEST-020-058: Owner Fact Injection Hardening
**Scenario:** access.spl cannot assert owner.
**Steps:**
1. Admin adds `(given (owner mallory))` to access.spl
2. ACL evaluation → fact stripped, WARN logged
3. Mallory does NOT have owner privileges

### TEST-020-059: Page-Level SPL Sandboxing
**Scenario:** Page cannot escalate privileges.
**Steps:**
1. Editor adds `(given (admin mallory))` in page's SPL code fence
2. Re-scan → fact stripped, WARN logged
3. Mallory does NOT have admin privileges
4. Editor adds `(except d1 (not (role ?user editor)) (can-read ?user "this-page"))` → accepted (restricts own page only)

### TEST-020-060: Authentication Rate Limiting
**Scenario:** Brute force recovery is rate-limited.
**Steps:**
1. Submit 5 invalid recovery attempts for user alice → all return 401
2. Submit 6th attempt → 429 Too Many Requests
3. Wait 1 minute → attempts allowed again

### TEST-020-061: WebSocket Ticket Authentication
**Scenario:** Agent uses one-time ticket for WebSocket.
**Steps:**
1. Agent POSTs to /auth/ws-ticket with bearer token → receives ticket
2. Agent connects to WS with ticket → authenticated
3. Agent retries same ticket → rejected (single-use)
4. Ticket expires after 30 seconds → rejected

### TEST-020-062: Recovery Challenge Expiry
**Scenario:** Expired challenge rejected.
**Steps:**
1. GET /auth/recover?user=alice → challenge issued
2. Wait 6 minutes
3. POST signed challenge → 410 Gone

### TEST-020-063: Content Security Policy
**Scenario:** CSP headers on all pages.
**Steps:**
1. GET any page → verify CSP header present
2. Verify: script-src 'self' (no inline scripts)
3. Verify: frame-ancestors 'none'

### TEST-020-064: CSRF Protection
**Scenario:** PUT without CSRF token rejected.
**Steps:**
1. Authenticate via passkey → receive session + CSRF token
2. PUT /roadmap without X-CSRF-Token header → 403
3. PUT /roadmap with valid X-CSRF-Token → 200
4. PUT /api/pages/roadmap with Authorization: Bearer → 200 (exempt from CSRF)

### TEST-020-065: Admin Route Hardcoded Check
**Scenario:** Non-admin cannot access admin routes even with SPL misconfiguration.
**Steps:**
1. Malformed access.spl grants editor access to /_admin/*
2. Editor visits /_admin/permissions → 403 (hardcoded check)
3. Admin visits same → 200

### TEST-020-066: Comment Integrity
**Scenario:** Tampered comments detected.
**Steps:**
1. Alice adds comment via API → stored with HMAC
2. External process modifies comment text in JSON file → HMAC mismatch
3. Comment displayed with "(unverified)" badge

### TEST-020-067: TLS Enforcement
**Scenario:** Collab mode warns without TLS.
**Steps:**
1. `ztl serve --collab --port 3000` (not localhost) → exits with TLS warning
2. `ztl_INSECURE_COLLAB=1 ztl serve --collab` → starts with WARN
3. `ztl serve --collab` binding to 127.0.0.1 → no warning

---

## 7. Non-Functional Requirements

### NFR-020-001: ACL Evaluation Latency

ACL cache lookup SHALL complete in < 1ms. Full SPL evaluation (cache miss) SHALL complete in < 50ms for policies with ≤ 500 rules.

Trace: REQ-020-013

### NFR-020-002: Concurrent Users

The system SHALL support at least 20 concurrent authenticated users with acceptable latency (p99 < 200ms for page renders).

### NFR-020-003: Git Commit Throughput

Auto-commit SHALL complete in < 100ms per save (excluding hooks). Concurrent saves are serialized via mutex; queue depth > 10 SHALL return 503 Service Unavailable.

### NFR-020-004: User Data Isolation

User credentials and session tokens SHALL be stored in `.ztl/users/` with filesystem permissions `0700`. The server process SHALL be the only reader.

### NFR-020-005: No External Network Dependencies

Authentication, authorization, and git commits SHALL operate entirely locally. No external services required. The system MUST function on an air-gapped network.

### NFR-020-006: CRDT Memory

Each loaded CRDT document SHALL consume ≤ 10MB of memory for documents up to 100KB of markdown. The 50-document concurrent limit (REQ-020-029) implies a maximum CRDT memory footprint of ~500MB.

### NFR-020-007: WebSocket Latency

CRDT operation broadcast latency (server receives op → other clients receive broadcast) SHALL be < 50ms at p99 for up to 10 concurrent editors on the same page.

### NFR-020-008: CRDT Sync Payload

Initial `sync` message payload SHALL be < 1MB for documents up to 100KB of markdown. Incremental `op` messages SHALL be < 1KB each.

---

## 8. Observability

### OBS-020-001: Authentication Events

The system SHALL log authentication events:

```
[ztl] auth: login user=alice method=passkey duration_ms=45
[ztl] auth: login-failed user=alice reason=invalid_credential
[ztl] auth: recovery user=alice duration_ms=120
[ztl] auth: agent-auth user=alice duration_ms=2
```

### OBS-020-002: Authorization Decisions

The system SHALL log authorization decisions when verbose:

```
[ztl] acl: allowed user=alice page=roadmap action=edit source=cache duration_ms=0
[ztl] acl: denied user=bob page=secret action=read rule=r-restrict-read duration_ms=32
```

### OBS-020-003: Git Commit Events

```
[ztl] commit: user=alice page="Meeting Notes" sha=abc1234 duration_ms=18
```

### OBS-020-004: Invitation Events

```
[ztl] invite: created by=alice role=editor pages=projects/* expires=2026-03-21T10:00:00Z
[ztl] invite: accepted by=bob-d4e5f6 invited_by=alice role=editor
[ztl] invite: rejected reason=expired nonce=a1b2c3d4
```

---

## 9. Phased Implementation

### Phase 1: Identity Foundation

**Goal:** Passkey auth, session management, bootstrap owner, basic ACL (hardcoded roles, no SPL yet).

**Changes:**
- Add `webauthn-rs`, `bip39`, `ed25519-dalek` dependencies (feature-gated)
- Implement user profile storage (`.ztl/users/`)
- Implement passkey registration/authentication routes
- Implement BIP39 generation and recovery flow
- Implement session middleware for Axum
- Implement `--collab --init-owner` CLI flow
- Gate all existing routes behind session check when `--collab` is active
- Add basic role-based ACL (admin/editor/reader) as placeholder for SPL

**Verification:** TEST-020-001, TEST-020-002, TEST-020-003, TEST-020-005

### Phase 2: Invitations and Git Integration

**Goal:** Invitation flow, auto-commit, conflict detection.

**Changes:**
- Implement invitation token generation (`ztl invite`)
- Implement invitation acceptance flow (registration page)
- Implement `git2`-based auto-commit in save handler
- Implement `If-Match` conflict detection
- Extend hook context with user identity (REQ-020-022)

**Verification:** TEST-020-006, TEST-020-007, TEST-020-015, TEST-020-016, TEST-020-022

### Phase 3: SPL Access Control

**Goal:** Replace hardcoded roles with full SPL-based authorization.

**Changes:**
- Implement ACL evaluation pipeline (CON-020-005)
- Implement built-in default policy
- Implement `access.spl` loading
- Implement page-level SPL override collection
- Implement ACL cache with merkle-based invalidation
- Implement `/api/acl/explain` endpoint
- Implement temporal access (`(now ...)` fact injection)
- Implement deontic modality mapping to HTTP responses

**Verification:** TEST-020-009 through TEST-020-014, TEST-020-030 through TEST-020-033

### Phase 4: Agent Integration

**Goal:** Agent token auth, API endpoints, loop prevention, agent SPL constraints.

**Changes:**
- Implement agent token derivation CLI
- Implement `Authorization: Bearer` middleware
- Implement `/api/*` agent endpoints
- Implement `ztl_HOOK_DEPTH` and `X-No-Hooks` mechanisms
- Implement `on-agent` hook lifecycle point
- Implement `(is-agent ...)` fact injection

**Verification:** TEST-020-004, TEST-020-017 through TEST-020-021, TEST-020-023

### Phase 5: Peritext CRDT Collaborative Editing

**Goal:** Real-time multi-user editing via Peritext CRDT with markdown round-trip.

**Changes:**
- Implement Peritext-style rich-text CRDT in Rust on top of `diamond-types = "1.0"` (text oplog) + a project-owned sibling marks oplog carrying `Mark`/`Unmark`/`Shift` span ops (see §3.7 "Marks Layer Architecture")
- Define mark types for markdown formatting + wikilinks (REQ-020-025)
- Implement block-level atomic tokens (REQ-020-026)
- Implement markdown → CRDT parser
- Implement CRDT → canonical markdown serializer (REQ-020-027)
- Implement WebSocket `/ws/edit/{slug}` endpoint with CRDT sync protocol (REQ-020-028)
- Implement CRDT document state management with eviction/flush (REQ-020-029)
- Implement cursor presence broadcast
- Integrate CRDT flush with git auto-commit pipeline (REQ-020-015)
- Implement API write → CRDT merge path (REQ-020-016)
- Build browser editor component with CRDT client (ProseMirror or CodeMirror 6)

**Verification:** TEST-020-024 through TEST-020-029, TEST-020-034 through TEST-020-038

### Phase 6: External Edit Reconciliation

**Goal:** Detect and reconcile edits from git, filesystem, or agents operating outside the web UI.

**Changes:**
- Extend filesystem watch to detect external edits (REQ-020-039)
- Implement CRDT reconciliation on external edit (REQ-020-040)
- Implement git HEAD polling for external commit detection (REQ-020-041)
- Implement external edit attribution (REQ-020-042)
- Implement ACL violation detection and `on-acl-violation` hook (REQ-020-043)

**Verification:** TEST-020-039 through TEST-020-043

### Phase 7: User Experience Layer

**Goal:** Web-based permission management, invitation UI, dashboard, comments, history UI, and onboarding polish.

**Changes:**
- Implement recovery phrase UX with confirmation step (REQ-020-045)
- Implement passkey registration guidance (REQ-020-046)
- Implement access request flow with admin notifications (REQ-020-047)
- Implement permission management UI at `/_admin/permissions` (REQ-020-048)
- Implement user dashboard at `/_me` (REQ-020-049)
- Implement web-based invitation at `/_admin/invite` (REQ-020-050)
- Implement page comments with sidecar storage (REQ-020-051)
- Implement agent merge notifications in editor (REQ-020-052)
- Implement page history UI at `/{slug}/_history` (REQ-020-053)
- Implement visibility explanations and admin contact (REQ-020-054)

**Verification:** TEST-020-045 through TEST-020-054

### Phase 8: Security Hardening

**Goal:** Harden authentication, authorization, and data protection against adversarial use.

**Changes:**
- Implement agent token generation counter and rotation (REQ-020-055)
- Implement mnemonic display security measures (REQ-020-056)
- Implement server key protection and .gitignore automation (REQ-020-057)
- Implement owner fact stripping from user-editable SPL (REQ-020-058)
- Implement page-level SPL sandboxing (REQ-020-059)
- Implement authentication rate limiting (REQ-020-060)
- Implement WebSocket ticket authentication for agents (REQ-020-061)
- Implement recovery challenge expiry (REQ-020-062)
- Implement Content-Security-Policy headers (REQ-020-063)
- Implement CSRF protection (REQ-020-064)
- Implement hardcoded admin route checks (REQ-020-065)
- Implement comment HMAC integrity (REQ-020-066)
- Implement TLS enforcement warning (REQ-020-067)

**Verification:** TEST-020-055 through TEST-020-067

---

## 10. Future Considerations

| Area                  | Description                                                        |
|-----------------------|--------------------------------------------------------------------|
| E2E encryption        | Encrypt vault pages at rest; decrypt in browser with user key      |
| OAuth/OIDC bridge     | Accept external IdP tokens mapped to local SPL identities          |
| Federation            | Cross-vault links with ACL negotiation between vault servers       |
| Audit log page        | Auto-generated wiki page showing all auth/edit events              |
| Trust decay for users | SPL `(decays ...)` applied to user roles for automatic expiry      |
| Agent budgets         | Token/request budgets per agent enforced via SPL arithmetic        |
| Offline CRDT sync     | Client-side CRDT state persisted in browser; sync on reconnect     |
| Block-level CRDT      | Extend Peritext with block-level merge (headings, lists, tables)   |
| CRDT history replay   | Replay CRDT operations for per-character attribution ("git blame for paragraphs") |

---

## 11. Open Questions

1. **Q: Should `access.spl` be editable via the web UI?**
   A: Yes, but only by users with `owner` or `admin` role. Edits to the access policy are themselves governed by the current policy — a strict bootstrap rule (`always s-owner-edit`) ensures the owner can never be locked out.

2. **Q: What happens if SPL policy has errors (parse failure, circular superiority)?**
   A: During live CRDT editing of access policy, parse errors trigger fallback to the **previous valid policy** (not built-in defaults), and the editor receives an `spl-error` WebSocket message (REQ-020-035). For non-CRDT contexts (e.g., malformed `access.spl` on disk at startup), the system falls back to built-in defaults. In both cases, the error is logged at WARN level and surfaced in the UI as a banner for admin users.

3. **Q: Should agent tokens be revocable without deleting the user?**
   A: Yes. Add a `revoked_agent_tokens` list to the user profile. The server checks this list on each agent auth. Alternatively, rotate the recovery key (new mnemonic), which invalidates all derived tokens.

4. **Q: How does this interact with the jj history system?**
   A: CRDT flush triggers the full pipeline: write markdown → re-scan (merkle) → git commit → jj `auto_snapshot` (REQ-020-034). Historical queries reflect the last flushed state, not in-flight CRDT edits (REQ-020-036). The jj snapshot captures the git commit with user attribution, enabling temporal queries like "show me the vault as Bob saw it last Tuesday."

5. **Q: Should invitation acceptance inject SPL facts automatically, or require admin approval?**
   A: Automatically (REQ-020-007). The inviter pre-authorized the role and scope when they generated the token. If approval is desired, the inviter simply doesn't generate the token until they're ready.

6. **Q: What is the source of truth — CRDT or markdown on disk?**
   A: Markdown on disk is always the canonical, committed state. The CRDT is a transient editing layer that exists only while a page has active editors. On flush, the CRDT serializes to markdown — at that point the file, merkle tree, git history, and jj snapshots all update atomically (REQ-020-034). Between flushes, the CRDT is a draft. If the server crashes with unflushed CRDT state, the last flushed markdown is the recovery point. This is by design — the CRDT improves the editing experience but does not replace the file-based architecture.

7. **Q: Can the quiescence flush interval be dangerous for the merkle tree and ACL?**
   A: The default 5-second quiescence delay means the merkle tree and ACL cache are up to 5 seconds stale during active editing. For access policy files, this is mitigated by immediate flush (REQ-020-035). For regular pages, the staleness affects only the jj snapshot granularity and search index freshness — both acceptable trade-offs for the performance benefit of batching flushes.

8. **Q: What happens if an external git push conflicts with an active CRDT session?**
   A: The CRDT merge is conflict-free by definition — external content is parsed into CRDT operations and merged. The result contains both the external edits and the in-flight edits. On flush, the merged state is committed as a new git commit. No manual merge needed. However, the semantic result may be surprising (e.g., an agent rewrites a paragraph while a human is editing it — both versions appear). Users should be aware that git-based agents operate outside the presence system and their edits arrive as surprise merges.

9. **Q: Should external edits bypass ACL or be subject to it?**
   A: External edits bypass ACL because they happen at the filesystem/git level — ztl cannot prevent them. The system detects and reports violations after the fact (REQ-020-043). This is a conscious design choice: ztl is not a security boundary for the filesystem. It's an application-level access control layer for the web interface and API. Filesystem permissions and git access control (SSH keys, deploy keys) are the appropriate mechanisms for restricting who can push to the repo.

---

## 12. Dependencies

| Crate            | Version | Purpose                                | Feature Gate |
|------------------|---------|----------------------------------------|--------------|
| `webauthn-rs`    | 0.5     | WebAuthn/FIDO2 passkey server          | `collab`     |
| `bip39`          | 2       | BIP39 mnemonic generation/validation   | `collab`     |
| `ed25519-dalek`  | 2       | Ed25519 key derivation and signing     | `collab`     |
| `git2`           | 0.19    | libgit2 bindings for auto-commit       | `collab`     |
| `jsonwebtoken`   | 9       | JWT creation/validation for invitations| `collab`     |
| `diamond-types`  | 1.0     | Text CRDT oplog (ztl adds a Peritext-style marks layer on top) | `collab`     |
| `spindle-core`   | (git)   | Defeasible reasoning for ACL           | `reason`     |
| `spindle-parser` | (git)   | SPL parsing for policy documents       | `reason`     |
| `rand`           | 0.8     | Nonce and token generation             | `collab`     |
