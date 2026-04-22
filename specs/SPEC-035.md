---
title: "SPEC-035: Collaborative Static Asset Uploads"
version: 0.1.0
status: draft
date: 2026-04-22
audience: agent, human
parent: SPEC-020
related:
  - SPEC-020  # Multi-user collaborative editing (ACL foundation)
  - SPEC-004  # Web UI and static export
  - SPEC-005  # Defeasible reasoning (SPL)
  - SPEC-034  # Capability-URL distribution (separate, complementary)
dependencies:
  - axum-multipart (or axum body streaming)
  - sha2 (SHA-256 integrity)
  - mime_guess (MIME type detection by extension)
  - spindle-core (SPL reasoning for can-upload ACL)
  - git2 (auto-commit of uploaded assets)
---

# SPEC-035: Collaborative Static Asset Uploads

## Information Table

| Field        | Value                                                                            |
| ------------ | -------------------------------------------------------------------------------- |
| Document ID  | SPEC-035                                                                         |
| Title        | Collaborative Static Asset Uploads                                               |
| Version      | 0.1.0                                                                            |
| Status       | Draft                                                                            |
| Author       | Agent (USDD Protocol v1.3.0)                                                     |
| Date         | 2026-04-22                                                                       |
| Audience     | Agent, Human                                                                     |
| Trace        | USDD Agent Protocol v1.3.0                                                       |
| Parent       | SPEC-020 (Multi-User Collaborative Editing)                                      |
| Related      | SPEC-004 (Web UI), SPEC-005 (SPL), SPEC-034 (capability distribution)           |
| Dependencies | axum-multipart, sha2, mime_guess, spindle-core, git2                             |

---

## 1. Overview

### 1.1 Problem

`zetl serve --collab` lets authorised collaborators create and edit Markdown pages in a shared vault, but provides no mechanism for uploading static files — images, diagrams, PDFs, exported HTML reports, or any binary asset. The consequences are:

- **Embedded images must be externally hosted.** Collaborators who want `![diagram](/some/path)` in a vault page either commit files via git (requires git access, bypasses the web UI) or use a third-party image host (creates external dependencies, breaks self-hosting).
- **Static HTML pages cannot be published via zetl.** A team member who exports a Jupyter notebook, a BI dashboard, or a custom one-pager cannot serve it at a stable vault URL without direct filesystem access.
- **Vault pages cannot reliably reference binary content.** Wikilinks, internal search, and backlink graphs assume the vault's content is text. Binary assets have no first-class representation.

### 1.2 Core Insight

Assets are vault content. They should live in the vault's git history, be subject to the same SPL-based ACL that governs pages, and be reachable at stable, human-readable URLs. The simplest design is: assets are files stored under `.zetl/assets/` within the vault, served under the `/assets/{*path}` URL namespace, uploaded through the same authenticated HTTP surface as page saves.

The namespace separation (`/assets/` prefix) is the key invariant: an uploaded file never collides with a vault page slug, so the two content types compose cleanly.

### 1.3 Design Principles

1. **Vault is the source of truth.** Assets live under `.zetl/assets/` and are committed to git with uploader attribution.
2. **ACL is first-class.** A new SPL predicate `can-upload` controls who may upload. The built-in default inherits from the editor role hierarchy. Vault administrators can restrict or expand it independently.
3. **Namespace isolation.** Assets are always served under `/assets/{*path}`. They never occupy the page slug namespace.
4. **Allowlist, not denylist.** Accepted MIME types are explicitly enumerated. Anything not on the list is rejected with a descriptive error.
5. **Atomic writes.** Files are written to a temporary path and renamed atomically; a partial upload never appears at the served path.
6. **No content transformation.** Assets are served byte-identical to what was uploaded. Zetl does not process, transcode, or optimise uploaded content.
7. **Safe-by-default serving headers.** Uploaded assets, especially HTML, are served with headers that isolate them from the zetl session context.

### 1.4 Scope

**In scope (v1):**
- File upload via HTTP multipart POST and raw body stream to `/api/assets/{*slug}`
- Asset storage under `.zetl/assets/` with JSON metadata sidecar per asset
- Asset serving under `/assets/{*path}` (raw, no template wrapping)
- CRUD surface: upload (create/replace), list, delete
- SPL-based `can-upload` permission predicate with built-in default rules
- Per-asset and vault-total storage limits (configurable)
- MIME type allowlist enforcement
- SHA-256 integrity verification on write and serve
- Git auto-commit of asset uploads and deletions
- Observability signals (counters, gauges, log lines)
- Collab-mode only — upload endpoint returns 404 in non-collab mode

**Out of scope (v1):**
- Image resizing, transcoding, or optimisation
- Streaming of large video files (the file-size cap applies)
- Zip/archive extraction
- Directory listing UI for the asset tree
- Asset-to-page backlinks (assets are not indexed in the vault graph)
- CDN or object-storage backends
- WebDAV or S3-compatible upload protocols
- Asset search (assets are not indexed by Tantivy)
- Fine-grained per-asset ACL (all assets share the vault-scoped `can-upload` / `can-read-assets` predicates)

---

## 2. User Profiles

### UP-035-001: Content Editor

**Role:** A collaborator with the `editor` role who authors vault pages.

**Goals:**
- Embed images and diagrams in Markdown pages without leaving the zetl UI.
- Keep assets versioned alongside pages in the same git repository.
- Get a stable URL immediately after upload.

**Constraints:**
- Not comfortable with the git CLI.
- Uses only the zetl web UI or a simple HTTP client (curl, VS Code extension).
- Technical proficiency: intermediate — knows what a URL and MIME type are, but not how the server works.

**Daily workflow:**
1. Writes a vault page about a project.
2. Has a PNG diagram on their laptop.
3. Opens the zetl UI, navigates to the asset upload panel.
4. Drags the file, chooses a slug like `assets/diagrams/architecture-2026.png`.
5. Pastes the resulting URL into their Markdown as `![Architecture](/assets/diagrams/architecture-2026.png)`.

### UP-035-002: Report Publisher

**Role:** A collaborator or admin who delivers outputs to stakeholders outside the vault.

**Goals:**
- Publish a static HTML file (e.g., a Jupyter notebook export, a custom dashboard, a quarterly report) at a stable URL.
- The URL must be accessible to stakeholders who do not have zetl accounts (if the vault's ACL allows public asset reads).
- The HTML file must render exactly as built — zetl must not wrap it in the vault template.

**Constraints:**
- The report is a pre-built standalone HTML file (all assets inlined or referenced from the same file).
- URL must be stable: linking in emails, in slide decks, cannot change quarter-over-quarter.
- May replace the file in-place next quarter under the same URL (overwrite existing slug).

**Daily workflow:**
1. Exports quarterly report as `q1-2026.html` from a BI tool.
2. Uploads via `PUT /api/assets/reports/q1-2026.html` with `X-Create: true`.
3. Shares `https://vault.example.com/assets/reports/q1-2026.html` with stakeholders.
4. Next quarter: uploads `q2-2026.html` under a new slug.

### UP-035-003: Vault Administrator

**Role:** Vault owner or admin, responsible for access policy and resource usage.

**Goals:**
- Control which users can upload assets (may want to restrict file upload to admins only).
- Enforce and monitor storage limits.
- Delete assets uploaded in error or no longer needed.
- Audit who uploaded what and when.

**Constraints:**
- May not be familiar with SPL syntax (should be able to express policy in plain terms with guidance).
- Needs clear error messages when limits are exceeded so they can respond before vault is full.

**Daily workflow:**
1. New collaborator invited with editor role — admin checks whether `can-upload` default is appropriate.
2. Receives a notification that storage quota is at 90%.
3. Reviews asset list via `GET /api/assets`, identifies stale files.
4. Deletes them via `DELETE /api/assets/{slug}`.
5. Git log provides full audit of who uploaded what.

---

## 3. Happy Paths

### HP-035-001: Content Editor Uploads an Image

**Preconditions:**
- Vault is running in collab mode (`zetl serve --collab`).
- User is authenticated with the `editor` role and global scope (`**`).
- No asset with slug `diagrams/architecture.png` exists.

**Steps:**

| Step | User action | Expected system response |
|------|-------------|--------------------------|
| 1 | `POST /api/assets/diagrams/architecture.png` with `X-Create: true`, `X-CSRF-Token: …`, multipart body containing the PNG | Server validates ACL (`can-upload` ✓), MIME type (`image/png` ✓), size (< 10 MB ✓), quota (not exceeded ✓), slug (safe ✓) |
| 2 | — | Server writes file atomically to `.zetl/assets/diagrams/architecture.png` |
| 3 | — | Server writes metadata sidecar to `.zetl/assets/diagrams/architecture.png.meta.json` |
| 4 | — | Server auto-commits both files to git |
| 5 | — | Server responds `201 Created` with JSON body: `{"slug": "diagrams/architecture.png", "url": "/assets/diagrams/architecture.png", "size_bytes": 48320, "mime_type": "image/png", "uploaded_at": "2026-04-22T11:00:00Z", "sha256": "abc…"}` |
| 6 | Editor pastes `![Architecture](/assets/diagrams/architecture.png)` into a vault page | Page renders with the image embedded |
| 7 | User visits `GET /assets/diagrams/architecture.png` | Server returns the PNG with `Content-Type: image/png`, `X-Content-Type-Options: nosniff`, `Cache-Control: public, max-age=31536000, immutable` |

**Postconditions:**
- Asset is at `.zetl/assets/diagrams/architecture.png` and `.zetl/assets/diagrams/architecture.png.meta.json`.
- Git history contains a commit attributed to the uploader.
- `GET /assets/diagrams/architecture.png` returns the file.

**Failure modes (see REQ-3507, REQ-3508, REQ-3509, REQ-3510, REQ-3511):**

| Condition | Response |
|-----------|----------|
| File > 10 MB | 413 with `{"error": "file_too_large", "max_bytes": 10485760, "received_bytes": N}` |
| MIME type not in allowlist | 415 with `{"error": "mime_type_not_allowed", "received": "application/x-msdownload", "allowed": ["image/*", …]}` |
| Vault storage quota exceeded | 507 with `{"error": "storage_quota_exceeded", "quota_bytes": 104857600, "used_bytes": N}` |
| Slug contains `..` or absolute path | 400 with `{"error": "invalid_slug"}` |
| User lacks `can-upload` permission | 403 with `{"error": "forbidden"}` |
| Slug already exists, no `X-Overwrite: true` | 409 with `{"error": "slug_exists", "slug": "…"}` |

### HP-035-002: Report Publisher Uploads Static HTML

**Preconditions:**
- Vault is in collab mode; user is authenticated with editor role + global scope.
- `reports/q1-2026.html` does not yet exist.

**Steps:**

| Step | User action | Expected system response |
|------|-------------|--------------------------|
| 1 | `POST /api/assets/reports/q1-2026.html` with `X-Create: true`, body is the HTML file | Server validates (text/html ✓), stores, commits |
| 2 | — | 201 with URL `/assets/reports/q1-2026.html` |
| 3 | User shares URL | — |
| 4 | Stakeholder opens `GET /assets/reports/q1-2026.html` in browser | Server returns HTML with `Content-Type: text/html; charset=utf-8`, `X-Content-Type-Options: nosniff`, `X-Frame-Options: SAMEORIGIN`, **no zetl template wrapper** |

**Failure modes:**
- Stakeholder not authenticated AND vault is in non-public ACL mode → 403 redirect to login.

### HP-035-003: Replace an Existing Asset

**Preconditions:** Asset `reports/q1-2026.html` exists.

**Steps:**

| Step | Action | Response |
|------|--------|----------|
| 1 | `POST /api/assets/reports/q1-2026.html` with `X-Overwrite: true`, new file content | Validates ACL, overwrites file atomically, writes updated metadata, commits |
| 2 | — | 200 with updated metadata JSON |
| 3 | Stakeholders who already have the URL cached receive the new file | (Server does not set immutable cache header on the replaced asset — see REQ-3519) |

### HP-035-004: Administrator Deletes an Asset

**Preconditions:** Asset exists; user is admin or owner.

**Steps:**

| Step | Action | Response |
|------|--------|----------|
| 1 | `DELETE /api/assets/diagrams/architecture.png` with `X-CSRF-Token` | Server validates ACL (`can-upload` ✓), deletes file + metadata sidecar, commits deletion |
| 2 | — | 204 No Content |
| 3 | `GET /assets/diagrams/architecture.png` | 404 Not Found |

### HP-035-005: Listing Assets

**Preconditions:** Several assets are uploaded.

**Steps:**

| Step | Action | Response |
|------|--------|----------|
| 1 | `GET /api/assets` | 200 with JSON array of asset metadata, sorted by slug |
| 2 | `GET /api/assets?prefix=reports/` | 200 with only assets under `reports/` subtree |

---

## 4. Synthetic User Simulation

### Simulation SIM-035-001 — Content Editor flow

**Model:** claude-sonnet-4-6  
**User profile:** UP-035-001 Content Editor  
**Happy path:** HP-035-001  
**Date:** 2026-04-22

**Walkthrough:**

**Step 1 — Finding the upload surface.** The editor wants to upload a PNG. They look for an "Upload" button in the vault UI. *Gap identified*: the spec defines an API endpoint (`/api/assets/{slug}`) but does not describe any web UI panel for browsing and uploading assets. The editor is not comfortable with curl. → **Finding SIM-035-001-F1** (below).

**Step 2 — Choosing a slug.** The editor must provide the slug `diagrams/architecture.png` as part of the URL. *Ambiguity*: it's not obvious that the slug must include the file extension. If they omit `.png`, the server will not know the MIME type from the slug alone. → **Finding SIM-035-001-F2** (below).

**Step 3 — Getting the embed URL.** After upload, the server returns `{"url": "/assets/diagrams/architecture.png", …}`. The editor copies this and writes `![Architecture](/assets/diagrams/architecture.png)` in their Markdown page. This works correctly.

**Step 4 — Verifying the image renders.** The editor opens the page. The image appears. ✓

**Findings:**

#### Finding: No Web UI for Asset Upload

- **Step:** 1 (finding the upload surface)
- **Category:** Gap
- **Description:** The spec defines only a JSON API endpoint. The Content Editor (UP-035-001) uses the zetl web UI exclusively and has no way to upload a file without using a command-line tool. A minimal upload form at `/_admin/assets` is needed for v1.
- **User impact:** Editors who cannot use curl are completely unable to use this feature.
- **Proposed resolution:** Add `REQ-3518`: a minimal asset management UI at `/_admin/assets` showing asset list and a file-upload form. The form POSTs to `/api/assets/{slug}` (client-side slug derived from filename, editable before submit).
- **Trace:** Creates REQ-3518

#### Finding: Slug Must Include File Extension — Not Communicated

- **Step:** 2 (choosing a slug)
- **Category:** Ambiguity
- **Description:** The server validates MIME type against the uploaded file's content (via magic bytes or the `Content-Type` header), but it also stores the extension as part of the slug. If a user uploads a PNG under slug `diagrams/architecture` (no `.png`), the stored file has no extension, and the server will serve it with `Content-Type: application/octet-stream` (or a MIME detected from content).
- **User impact:** Files served without the correct extension may not render in browsers. The user gets the wrong URL to embed.
- **Proposed resolution:** Clarify in REQ-3503 that MIME type is determined from the `Content-Type` header on upload (required, validated), NOT from the slug's extension. Extension in the slug is user-controlled and advisory. Amend CON-3501 to require `Content-Type` header on upload requests.
- **Trace:** Amends REQ-3503 and CON-3501

### Simulation SIM-035-002 — Report Publisher flow

**Model:** claude-sonnet-4-6  
**User profile:** UP-035-002 Report Publisher  
**Happy path:** HP-035-002  
**Date:** 2026-04-22

**Step 1 — Uploading the HTML file.** The publisher runs `curl -X POST -H "X-Create: true" -H "X-CSRF-Token: …" -F "file=@q1-2026.html;type=text/html" https://vault.example.com/api/assets/reports/q1-2026.html`. The server accepts. ✓

**Step 2 — Sharing the URL.** URL is `https://vault.example.com/assets/reports/q1-2026.html`. Pasted into email. ✓

**Step 3 — Stakeholder opens URL (vault ACL is public read).** Browser fetches the HTML. Server returns with `X-Content-Type-Options: nosniff`. *Ambiguity*: the spec says assets can be viewed subject to `can-read-assets` ACL, but what is the default for unauthenticated stakeholders when the vault's `visibility-mode` is `transparent` or `mixed`? → **Finding SIM-035-002-F1**.

**Step 4 — The HTML file references relative paths.** The report HTML has `<script src="./charts.js"></script>`. This JS file is not uploaded. Browser requests `GET /assets/reports/charts.js` → 404. *Error path gap*: the spec does not say how to handle HTML pages with relative asset dependencies. → **Finding SIM-035-002-F2** (out-of-scope for v1, document as known limitation).

**Findings:**

#### Finding: Unauthenticated Asset Access Policy Undefined

- **Step:** 3 (stakeholder opens URL)
- **Category:** Gap
- **Description:** The spec introduces `can-read-assets` ACL but does not define what the built-in default is for unauthenticated users (no zetl session) when `visibility-mode` is `transparent` or `mixed`.
- **User impact:** If the default is "deny unauthenticated", external stakeholders cannot view the report — defeating the primary use case. If the default is "allow unauthenticated", sensitive assets may be exposed by mistake.
- **Proposed resolution:** Add REQ-3516 default: if `visibility-mode` is `transparent` (vault is effectively public), unauthenticated users can read assets. If `mixed` or `hidden`, assets require authentication. Administrators can override via SPL.
- **Trace:** Creates REQ-3516

#### Finding: Relative-Reference HTML Dependencies Are Not Uploadable Together

- **Step:** 4 (HTML references relative JS)
- **Category:** Error path gap
- **Description:** The report publisher's HTML file references sibling files (`charts.js`, `styles.css`). Each must be uploaded separately under the correct slug. No grouping or bundle upload exists in v1.
- **User impact:** Multi-file HTML assets do not work out of the box unless the publisher uploads each file under the correct relative slug.
- **Proposed resolution:** Document as a known v1 limitation in §9 (Out of Scope). Recommend publishers inline all assets using tools like `html-inline` or Jupyter's `--to html` mode with `--embed-images`.
- **Trace:** Documented in §9; no new REQ (v1 scope decision)

---

## 5. Requirements

### 5.1 Functional Requirements

---

**REQ-3501: Collab-Only Upload Endpoint**

The system SHALL expose the asset upload API exclusively when `--collab` mode is active. In non-collab mode, `POST /api/assets/{*slug}` SHALL return `404 Not Found` with no body, as if the route does not exist.

*Rationale:* Single-user mode has no authentication surface; allowing unauthenticated uploads would create a trivial remote-write vulnerability.

Trace:
- TEST-3501
- CON-3501

---

**REQ-3502: Asset Storage Location**

Uploaded assets SHALL be stored on disk at `{vault_root}/.zetl/assets/{slug}` where `{slug}` is the URL-decoded path component from the upload request. Each asset SHALL have a corresponding metadata sidecar at `{vault_root}/.zetl/assets/{slug}.meta.json` written atomically together with the asset file.

*Rationale:* Storing under `.zetl/assets/` keeps assets inside the git repository alongside vault pages, enabling versioning, backup, and history via the same git workflow already used for Markdown pages.

Trace:
- TEST-3502
- CON-3501
- CON-3506

---

**REQ-3503: MIME Type Determination and Enforcement**

The system SHALL determine the MIME type of an uploaded asset from the request's `Content-Type` header, not from the slug's file extension. The `Content-Type` header MUST be present; if absent, the server SHALL return `400 Bad Request`. The resolved MIME type SHALL be checked against the MIME allowlist (REQ-3507). The resolved MIME type SHALL be stored in the metadata sidecar and used verbatim when serving the asset.

*Rationale:* Relying on the file extension for MIME type is unsafe (an attacker could name a script `photo.png` with `Content-Type: text/html`). Requiring the header puts the contract on the uploader and makes it auditable.

Amended by SIM-035-001-F2.

Trace:
- TEST-3503
- CON-3501

---

**REQ-3504: Upload ACL — `can-upload` Predicate**

The system SHALL check the SPL predicate `(can-upload "{user_id}")` before processing any upload, replace, or delete request. If the predicate is not provably true (i.e., not `+d` or `+D`), the server SHALL return `403 Forbidden`. The built-in default SPL for `can-upload` SHALL be:

```spl
;; Built-in default: editors with full-scope access can upload.
;; Restricted-scope editors (invited with a page-glob scope) cannot,
;; because assets are vault-wide and their scope doesn't cover them.
(normally r-editor-upload
  (role "{user_id}" editor)
  (scope "{user_id}" "**")
  (can-upload "{user_id}"))

;; Owners and admins can always upload.
(always s-owner-upload
  (owner "{user_id}")
  (can-upload "{user_id}"))

(normally r-admin-upload
  (admin "{user_id}")
  (can-upload "{user_id}"))
```

Vault administrators may override this by adding rules to `.zetl/collab/access.spl`.

*Rationale:* `can-upload` is a vault-scoped permission (not per-page) because assets are stored in a shared vault-wide location. Restricted-scope editors should not be able to consume shared storage or introduce vault-wide files without explicit admin grant.

Trace:
- TEST-3504
- CON-3501
- CON-3505

---

**REQ-3505: Asset Listing**

The system SHALL expose `GET /api/assets` to authenticated users with the `can-upload` permission (same gate as upload). The endpoint SHALL return a JSON array of asset metadata objects (CON-3503), sorted lexicographically by slug. An optional `?prefix=` query parameter SHALL filter results to assets whose slug begins with the given prefix.

*Rationale:* Administrators need to audit the asset library. Restricting listing to uploaders (rather than all authenticated users) prevents information leakage about what files exist when not all users can see all assets.

Trace:
- TEST-3505
- CON-3503

---

**REQ-3506: Asset Deletion**

The system SHALL expose `DELETE /api/assets/{*slug}` to authenticated users with the `can-upload` permission. On success, both the asset file and its metadata sidecar SHALL be deleted atomically (rename to tmp, unlink tmp). The deletion SHALL be committed to git (REQ-3513). The server SHALL return `204 No Content` on success, `404` if the slug does not exist, and `403` if the caller lacks `can-upload`.

Trace:
- TEST-3506
- CON-3504

---

**REQ-3507: MIME Type Allowlist**

The system SHALL reject uploads whose `Content-Type` (after stripping parameters such as `; charset=…`) is not in the following allowlist. On rejection, the server SHALL return `415 Unsupported Media Type` with a JSON body enumerating the allowed types.

**Allowlist (v1):**

| Category | MIME types |
|----------|-----------|
| Images | `image/png`, `image/jpeg`, `image/gif`, `image/webp`, `image/svg+xml`, `image/avif`, `image/ico`, `image/x-icon` |
| Documents | `application/pdf` |
| Text/markup | `text/html`, `text/plain`, `text/markdown`, `text/css` |
| Data | `application/json`, `text/csv` |
| Web scripts | `application/javascript`, `text/javascript` |
| Fonts | `font/woff2`, `font/woff`, `font/ttf`, `application/font-woff2` |
| Audio | `audio/mpeg`, `audio/ogg`, `audio/wav`, `audio/webm` |
| Video | `video/mp4`, `video/webm`, `video/ogg` |

*Rationale:* An allowlist prevents upload of executable binaries (`.exe`, `.dmg`, `.sh`), archives that could be extraction-attacked (`.zip`), and other content types that pose unnecessary risk on a file-serving endpoint.

Trace:
- TEST-3507
- CON-3501

---

**REQ-3508: Per-File Size Limit**

The system SHALL reject any upload whose `Content-Length` header (or actual streamed body, whichever is reached first) exceeds the configured per-file limit. The default SHALL be 10 MiB (10,485,760 bytes). The limit SHALL be configurable via the `--asset-max-file-bytes` CLI flag. On rejection the server SHALL return `413 Request Entity Too Large` before completing the read.

*Rationale:* Without a size cap, a single upload can exhaust disk space or memory. Reading the `Content-Length` header before streaming avoids buffering the body.

Trace:
- TEST-3508
- CON-3501

---

**REQ-3509: Vault-Total Storage Limit**

The system SHALL track total asset storage usage (sum of all asset file sizes, excluding metadata sidecars) and reject uploads that would cause total usage to exceed the configured limit. The default SHALL be 100 MiB (104,857,600 bytes). The limit SHALL be configurable via the `--asset-max-total-bytes` CLI flag. On rejection the server SHALL return `507 Insufficient Storage`.

*Rationale:* Prevents vault disk exhaustion by accumulated uploads.

Trace:
- TEST-3509
- CON-3501
- OBS-3505

---

**REQ-3510: Slug Validation**

The system SHALL validate the upload slug before any disk I/O. A valid slug MUST satisfy ALL of the following:

1. Non-empty.
2. Contains only UTF-8 characters; percent-encoding is decoded before validation.
3. No path component is `.` or `..`.
4. Does not begin or end with `/`.
5. No component is empty (no `//` sequences).
6. No null bytes.
7. No component is a device name on any platform (reject `CON`, `PRN`, `AUX`, `NUL`, `COM[0-9]`, `LPT[0-9]` as case-insensitive matches).

On any violation the server SHALL return `400 Bad Request` with `{"error": "invalid_slug", "detail": "…"}`.

After validation, the server SHALL resolve the canonical path as `vault_root/.zetl/assets/{slug}` and verify that it lies strictly within `vault_root/.zetl/assets/` before proceeding.

*Rationale:* Path traversal is a perennial risk in file-serving systems. Slug validation provides a high-level check; the canonical path check provides a low-level defense-in-depth.

Trace:
- TEST-3510
- CON-3501

---

**REQ-3511: Atomic Write with SHA-256 Integrity**

The system SHALL write uploaded assets in two steps:

1. Stream the body to a temporary file at `{vault_root}/.zetl/assets/.tmp/{uuid}`, computing the SHA-256 digest incrementally.
2. If the full body is received without error: rename the temporary file to its final path (`{vault_root}/.zetl/assets/{slug}`), then write the metadata sidecar. If any error occurs: delete the temporary file and return the appropriate HTTP error.

The SHA-256 hex digest SHALL be stored in the metadata sidecar and returned in the upload response.

On `GET /assets/{*path}`, the server SHALL verify the SHA-256 digest of the served file against the metadata sidecar on every request in `--verbose` mode; in production it SHALL perform a periodic background integrity check at a configurable interval (default: off in v1).

*Rationale:* Atomic rename prevents partial files from appearing at the served path. SHA-256 enables integrity auditing and detects silent disk corruption.

Trace:
- TEST-3511
- CON-3501
- NFR-3503

---

**REQ-3512: Asset Serving — Raw, No Template Wrapping**

The system SHALL serve assets at `GET /assets/{*path}` by reading and streaming the file from `{vault_root}/.zetl/assets/{path}` directly, with no Minijinja template wrapping and no injection of zetl navigation chrome. The response SHALL use the MIME type stored in the metadata sidecar as the `Content-Type` header value.

*Rationale:* The primary use case (UP-035-002) requires that an uploaded HTML page renders exactly as built. Wrapping it in the zetl template would corrupt it.

Trace:
- TEST-3512
- CON-3502

---

**REQ-3513: Git Auto-Commit of Asset Operations**

The system SHALL commit uploaded assets to the vault's git repository (if one exists) using the same `git_commit_lock` serialisation as page saves. The commit message format SHALL be:

- Upload (new): `asset: upload {slug} ({size_human}) [user: {user_id}]`
- Replace: `asset: replace {slug} ({size_human}) [user: {user_id}]`
- Delete: `asset: delete {slug} [user: {user_id}]`

Where `{size_human}` is a human-readable size (e.g., `48 KiB`). The commit SHALL include both the asset file and its metadata sidecar (for uploads/replaces) or neither (for deletes, after both are removed).

*Rationale:* The git log is the audit trail for collaborative changes. Asset operations must appear in it with uploader attribution, consistent with page saves.

Trace:
- TEST-3513
- CON-3501

---

**REQ-3514: Serving Headers for Isolation**

Every asset served at `GET /assets/{*path}` SHALL include the following response headers regardless of MIME type:

```
X-Content-Type-Options: nosniff
X-Frame-Options: SAMEORIGIN
```

For assets with MIME type `text/html`:

```
Content-Security-Policy: default-src 'self' 'unsafe-inline' 'unsafe-eval' data: blob:; frame-ancestors 'self'
```

*Rationale:* `X-Content-Type-Options: nosniff` prevents browsers from sniffing MIME type and executing a disguised script. `X-Frame-Options` prevents clickjacking. The HTML CSP restricts what the served page can load while still permitting custom JavaScript within the file (required for the dashboard/report use case). The zetl session cookie is `HttpOnly` (SPEC-020 CON-020-001), so scripts in the uploaded HTML cannot steal it. All state-mutating API endpoints require the `X-CSRF-Token` header (SPEC-020), which `fetch()` in the uploaded HTML cannot supply without colluding with a logged-in session.

Trace:
- TEST-3514
- CON-3502

---

**REQ-3515: Cache Control for Assets**

The system SHALL set `Cache-Control` headers on served assets as follows:

- **New upload (first serve after creation):** `Cache-Control: public, max-age=31536000, immutable` — assets are content-addressed by slug; once uploaded they do not change unless explicitly replaced.
- **After a replace operation (`X-Overwrite: true`):** The metadata sidecar MUST record a `replaced_at` timestamp. The serve handler SHALL check whether a replacement has occurred and, if so, set `Cache-Control: public, max-age=300` (5 minutes) on subsequent serves to allow CDN caches to refresh.

*Rationale:* Immutable caching is correct for assets that never change (first-upload case). After an overwrite, the URL serves new content, so the immutable hint would cause stale serving.

Amended by HP-035-003.

Trace:
- TEST-3515
- CON-3502

---

**REQ-3516: Unauthenticated Asset Access Policy**

The system SHALL allow unauthenticated access to served assets (`GET /assets/{*path}`) under the following default policy:

- If the vault's `visibility-mode` (SPEC-020 REQ-020-030) is `transparent`: unauthenticated requests SHOULD be allowed to read any asset, subject to vault-level SPL overrides.
- If the vault's `visibility-mode` is `mixed` (default) or `hidden`: unauthenticated requests SHALL receive `401 Unauthorized` with a `Location` header pointing to the login page.

Vault administrators MAY override the default by adding SPL rules concluding `(can-read-assets "anonymous" "*")` (allow all) or `(not (can-read-assets …))` (deny specific).

*Rationale:* The report publisher use case (UP-035-002) requires unauthenticated stakeholders to read assets when the vault's overall policy is public-read. The default must not expose assets in vaults that are access-controlled.

Amended by SIM-035-002-F1.

Trace:
- TEST-3516
- CON-3502
- CON-3505

---

**REQ-3517: Overwrite Semantics**

The system SHALL distinguish new-asset creation from asset replacement using HTTP request headers:

- `X-Create: true` — required when creating an asset at a slug that does not yet exist. If the slug already exists and `X-Overwrite: true` is not also present, the server SHALL return `409 Conflict`.
- `X-Overwrite: true` — required when replacing an existing asset. If the slug does not exist and `X-Overwrite: true` is sent without `X-Create: true`, the server SHALL return `404 Not Found`.
- Both `X-Create: true` and `X-Overwrite: true` together: creates if absent, replaces if present (upsert).

*Rationale:* Explicit semantics prevent accidental overwrites (a common mistake when slugs collide) and prevent silent no-ops when an uploader believes they are creating a new asset.

Trace:
- TEST-3517
- CON-3501

---

**REQ-3518: Asset Management UI (Minimal)**

The system SHALL expose a minimal asset management panel at `GET /_admin/assets` accessible to users with the `can-upload` permission. The panel SHALL display:

1. A list of all uploaded assets (slug, MIME type, size, uploader, upload date) with a delete button per asset.
2. A file upload form with fields for:
   - File picker (input type=file)
   - Slug text field (pre-populated from the selected filename, editable)
   - Submit button labelled "Upload"
3. A summary of used / total storage quota.

The UI SHALL derive the slug from the chosen filename by URL-encoding the filename (lowercased). It SHALL display validation errors inline (slug conflicts, MIME rejections, size limit violations) without a page reload.

*Rationale:* Addresses SIM-035-001-F1. Editors who cannot use curl need a browser UI.

Trace:
- TEST-3518

---

**REQ-3519: Storage Usage Tracking**

The system SHALL maintain an in-memory running total of asset storage usage, initialised at server startup by scanning `.zetl/assets/` and summed from all file sizes. Uploads increment this total; deletions decrement it. The total SHALL be consulted (with appropriate locking) before each upload to enforce REQ-3509.

*Rationale:* Re-scanning the directory on every upload would be O(n assets) per request. An in-memory counter updated atomically is O(1) per request.

Trace:
- TEST-3519
- OBS-3505

---

### 5.2 Non-Functional Requirements

---

**NFR-3501: Upload Throughput**

Asset uploads SHALL complete within 10 seconds for a file of exactly 10 MiB on a loopback connection (i.e., client and server on the same host). This sets a lower bound on disk write throughput; the primary latency bottleneck is expected to be network, not disk, in real deployments.

Trace:
- TEST-3528
- OBS-3504

---

**NFR-3502: Serve Latency**

`GET /assets/{*path}` SHALL respond with the first byte within 50 ms at the 95th percentile for assets ≤ 1 MiB on a loopback connection, excluding network transfer time.

Trace:
- TEST-3529
- OBS-3504

---

**NFR-3503: Integrity**

No uploaded file SHALL be returned by `GET /assets/{*path}` with different bytes from those received during upload. This MUST be verified by the integration test suite by uploading a file with a known SHA-256, downloading it, and comparing digests.

Trace:
- TEST-3511

---

**NFR-3504: Partial Upload Safety**

A simulated connection drop during upload (body truncated mid-stream) SHALL NOT result in a file appearing at the served path. The temporary file SHALL be cleaned up. This is verified by the atomic-write contract (REQ-3511).

Trace:
- TEST-3525

---

**NFR-3505: Concurrent Upload Safety**

Two concurrent uploads to different slugs SHALL both succeed and produce correct files. Two concurrent uploads to the same slug (both with `X-Overwrite: true`) SHALL result in exactly one file on disk, not a corrupt interleaving of both. The winner is unspecified (last write wins at the filesystem rename level).

Trace:
- TEST-3526

---

**NFR-3506: Storage Counter Accuracy**

The in-memory storage counter (REQ-3519) SHALL not drift more than one file-size worth of bytes from the true on-disk total across any sequence of uploads and deletes within a single server run. Drift is acceptable if the server is restarted (counter is re-initialised at startup).

Trace:
- TEST-3527

---

## 6. Architecture Decisions

### ADR-3501: Asset Storage in Vault vs External Store

**Decision:** Store assets under `{vault_root}/.zetl/assets/` within the vault directory tree.

**Context:** Assets must be durable, versionable, and accessible to both the HTTP server and the git auto-commit pipeline. Three options were considered: (1) in-vault at `.zetl/assets/`, (2) a separate configurable directory outside the vault, (3) an external object-storage backend (S3-compatible).

**Rationale:**
- Option 1 (chosen): Assets are git-committed alongside pages. Backup, migration, and history use the same git workflow already in place. No additional infrastructure. No configuration required. Consistent with the vault-as-single-source-of-truth principle.
- Option 2: Separate directory complicates backup and migration — two directories must be kept in sync. Configuration adds operator burden. Offers no benefit for the target use case (single-host collab vault).
- Option 3: Introduces an external dependency (S3 credentials, bucket lifecycle policies) that conflicts with zetl's local-first philosophy. Appropriate for a future cloud-hosting tier, not v1.

**Consequences:** Large media files (videos, high-res images) increase the git repository size, which slows clone times. Operators who store large assets should consider `git-lfs` for that content; this is a documented limitation rather than a v1-blocker.

---

### ADR-3502: URL Namespace — `/assets/` Prefix

**Decision:** Assets are served under the `/assets/{*path}` URL prefix, strictly separated from the page slug namespace (`/{slug}`).

**Context:** Two alternatives were considered: (a) a dedicated `/assets/` prefix, (b) serving assets at the root slug namespace alongside pages.

**Rationale:**
- Option (a) (chosen): Namespace isolation is a strong invariant. A page named `images/hero` can coexist with an asset `images/hero.png` without conflict. The distinction is always visually clear in links.
- Option (b): Requires conflict detection between page slugs and asset slugs at every upload and page-create. The ACL layer would need to distinguish page reads from asset reads. Serving assets from the root namespace confuses the MIME-type logic (zetl currently renders all root-namespace content as Markdown pages).

**Consequences:** Asset URLs include the `/assets/` prefix, which some users may find verbose. The trade-off (clear semantics, no namespace collisions) is worthwhile for v1.

---

### ADR-3503: ACL Predicate — `can-upload` vs Reusing `can-edit`

**Decision:** Introduce a new vault-scoped SPL predicate `can-upload` rather than reusing the page-scoped `can-edit`.

**Context:** `can-edit` is a two-argument predicate `(can-edit user page-slug)` checked per page. Asset uploads are vault-wide (not per-page), so a single-argument predicate `(can-upload user)` is the right shape. Even if the default derives from `can-edit`, keeping it separate allows independent policy evolution (e.g., an admin who wants editors to write pages but not consume shared storage can deny `can-upload` without touching `can-edit`).

**Consequences:** A new predicate must be added to the ACL evaluation pipeline and the built-in SPL defaults. The predicate name must be guarded against injection (same escaping rules as existing predicates).

---

### ADR-3504: MIME Type Determination — Header-First

**Decision:** Determine MIME type from the request's `Content-Type` header, not from the slug's file extension.

**Context:** Two approaches: (a) trust the uploader-specified `Content-Type` header, (b) detect MIME type from file magic bytes. A hybrid (validate magic bytes against declared Content-Type) is a third option.

**Rationale:**
- Option (a) (chosen): Simple, auditable, consistent with HTTP semantics. The uploader is the authority on what they uploaded. The MIME type is stored and served verbatim — what you declare is what is served.
- Option (b): Magic-byte detection adds a dependency (`infer`, `tree_magic`, or similar) and is unreliable for text types (plain text, HTML, CSV, Markdown are all valid text that doesn't have a distinctive magic header). It also creates a mismatch between "what the uploader said" and "what the server infers."
- Hybrid: More complex, adds a crate dependency, and can produce false positives for text MIME types. Deferred to a future security hardening pass if needed.

**Consequences:** A malicious uploader can declare a misleading MIME type (e.g., claim `image/png` for an HTML file). Mitigated by: (1) upload requires `can-upload` permission, so only trusted collaborators upload; (2) `X-Content-Type-Options: nosniff` prevents browsers from re-sniffing and overriding the declared type; (3) the allowlist prevents types that are particularly dangerous (no `application/x-executable`, etc.).

---

### ADR-3505: HTML Asset Isolation Strategy

**Decision:** Serve uploaded HTML assets directly (no iframe sandboxing) with `X-Content-Type-Options: nosniff`, `X-Frame-Options: SAMEORIGIN`, and a permissive `Content-Security-Policy` (allows inline scripts and eval, restricts framing).

**Context:** Uploaded HTML from trusted collaborators may contain JavaScript needed for dashboards and visualisations. Three options: (a) serve with sandbox CSP blocking all JS, (b) serve with permissive CSP, (c) require serving via sandboxed iframe.

**Rationale:**
- Option (a): Would break the dashboard use case entirely. A report publisher needs JS to run.
- Option (b) (chosen): Permits the full HTML+JS use case. The session cookie is `HttpOnly` (not accessible to JS). State-mutating API calls require the `X-CSRF-Token` header, which an uploaded page cannot generate without collusion with an authenticated session. The risk surface is restricted to trusted collaborators who already have write access.
- Option (c): Sandboxed iframe semantics differ by browser; cross-origin iframes add complexity; authors would need to know their page will be framed.

**Trust model:** The `can-upload` permission gate means only authorised editors can introduce HTML assets. If an attacker gains upload access, they have already compromised a collaborator account and can make direct API calls — an uploaded malicious HTML page is not the highest-severity vector in this threat model.

**Consequences:** Uploaded HTML pages run JavaScript in the zetl origin. Vault operators who require stricter isolation should use a separate subdomain for asset serving (infrastructure concern, out of scope for v1).

---

### ADR-3506: Storage Counter — In-Memory Running Total

**Decision:** Track total asset storage as an in-memory atomic counter, initialised at startup by directory scan.

**Context:** Two alternatives: (a) re-scan `.zetl/assets/` on every upload, (b) maintain a persistent counter in a file or database.

**Rationale:**
- Option (a): O(n assets) per upload. Acceptable for tens of assets; unacceptable for thousands.
- Option (b): A persistent counter in a file would require its own atomic write + read cycle, adding I/O on every upload. The running total can drift after a crash (files written outside the server), requiring periodic reconciliation anyway.
- Option chosen (in-memory, startup scan): O(1) per upload/delete; accurate within a single server run; re-initialised correctly on restart (which rescans). Simple to implement, no new file format.

**Consequences:** The counter does not account for assets added outside the server (e.g., direct `git clone` followed by a restart). This is acceptable: the server always rescans at startup.

---

## 7. Purity Boundary Map

### Pure Core (no I/O, no shared state, deterministic)

| Module / Function | What it computes |
|-------------------|-----------------|
| `src/assets/validation.rs :: validate_slug(slug: &str) → Result<(), SlugError>` | Checks slug against the rules in REQ-3510 |
| `src/assets/validation.rs :: check_mime_allowlist(mime: &Mime) → bool` | Tests a parsed MIME type against the static allowlist in REQ-3507 |
| `src/assets/metadata.rs :: AssetMeta` | Struct (slug, mime_type, size_bytes, sha256, uploaded_by, uploaded_at, replaced_at?) + serde serialisation |
| `src/assets/metadata.rs :: cache_control_for(meta: &AssetMeta) → &'static str` | Returns the correct `Cache-Control` value per REQ-3515 |
| `src/assets/metadata.rs :: human_size(bytes: u64) → String` | Formats size as `48 KiB`, `1.2 MiB`, etc. for git commit messages |

### Effectful Shell (orchestrates I/O, calls pure core)

| Module / Function | What effects it performs |
|-------------------|-------------------------|
| `src/assets/store.rs :: write_asset(root, slug, body_stream, meta) → Result<AssetMeta>` | Streams body to tmp, computes SHA-256, renames to final path, writes sidecar |
| `src/assets/store.rs :: delete_asset(root, slug) → Result<()>` | Unlinks asset + sidecar atomically |
| `src/assets/store.rs :: list_assets(root, prefix) → Result<Vec<AssetMeta>>` | Walks `.zetl/assets/`, reads all sidecars matching prefix |
| `src/assets/store.rs :: init_storage_total(root) → Result<u64>` | Scans `.zetl/assets/` at startup to initialise the counter |
| `src/assets/store.rs :: serve_asset(root, path) → Result<(AssetMeta, File)>` | Opens the file and reads its sidecar for serving |
| `src/web/routes.rs :: upload_handler` | HTTP handler: parse request, call pure core validation, call store, call git commit, emit observability |
| `src/web/routes.rs :: serve_asset_handler` | HTTP handler: call store, set response headers, stream file |
| `src/web/routes.rs :: list_assets_handler` | HTTP handler: call store, return JSON |
| `src/web/routes.rs :: delete_asset_handler` | HTTP handler: ACL check, call store, call git commit |

### Boundary Data Types

| Type | Direction |
|------|-----------|
| `AssetMeta` | Core → Shell (produced by validation, consumed by store and HTTP layer) |
| `SlugError`, `MimeError` | Core → Shell (validation failures surfaced as HTTP errors) |
| `StorageCounterGuard` | Shell only (wraps `Arc<AtomicU64>` with upload-increment / delete-decrement) |

### Dependency Rule

Shell modules (`store`, `routes`) depend on core (`validation`, `metadata`). Core MUST NOT import from shell. Enforced by module visibility (`pub(crate)` on core types) and `cargo clippy` in CI.

---

## 8. Contracts

### CON-3501: Asset Upload Endpoint

**Endpoint:** `POST /api/assets/{*slug}`

**Authentication:** Required — zetl session cookie or `Authorization: Bearer <agent-token>` (same as page API).

**Pre-conditions:**
- Collab mode is active (REQ-3501).
- `{slug}` passes validation (REQ-3510).
- `Content-Type` header is present and in the allowlist (REQ-3503, REQ-3507).
- `Content-Length` header is present and within per-file limit (REQ-3508).
- Caller has `can-upload` permission (REQ-3504).
- At least one of `X-Create: true` or `X-Overwrite: true` is present (REQ-3517).

**Request headers:**

| Header | Required | Description |
|--------|----------|-------------|
| `X-CSRF-Token` | Yes (browser clients) | CSRF token from session |
| `Content-Type` | Yes | MIME type of the uploaded file |
| `Content-Length` | Yes | File size in bytes |
| `X-Create` | Conditional | `true` when creating a new asset |
| `X-Overwrite` | Conditional | `true` when replacing an existing asset |

**Request body:** Raw file content (not multipart). For multipart uploads from browser forms, the handler MUST extract the file part.

**Success responses:**

| Status | Condition | Body |
|--------|-----------|------|
| 201 Created | New asset created | `AssetResponse` JSON (see below) |
| 200 OK | Existing asset replaced | `AssetResponse` JSON |

**Error responses:**

| Status | Error key | Condition |
|--------|-----------|-----------|
| 400 | `invalid_slug` | Slug fails validation |
| 400 | `missing_content_type` | No `Content-Type` header |
| 400 | `missing_create_or_overwrite` | Neither `X-Create` nor `X-Overwrite` present |
| 401 | `unauthenticated` | No valid session |
| 403 | `forbidden` | `can-upload` not satisfied |
| 404 | — | Collab mode not active |
| 404 | `not_found` | `X-Overwrite` only, slug does not exist |
| 409 | `slug_exists` | `X-Create` only, slug already exists |
| 413 | `file_too_large` | Body exceeds per-file limit |
| 415 | `mime_type_not_allowed` | MIME type not in allowlist |
| 507 | `storage_quota_exceeded` | Upload would exceed vault total limit |

**AssetResponse schema:**

```json
{
  "slug": "diagrams/architecture.png",
  "url": "/assets/diagrams/architecture.png",
  "mime_type": "image/png",
  "size_bytes": 48320,
  "sha256": "3a7bd3e2360a3d29eea436fcfb7e44c735d117c42d1c1835420b6b9942dd4f1b",
  "uploaded_by": "alice-abc12345",
  "uploaded_at": "2026-04-22T11:00:00Z",
  "replaced_at": null
}
```

**Implements:** REQ-3501, REQ-3502, REQ-3503, REQ-3504, REQ-3507, REQ-3508, REQ-3509, REQ-3510, REQ-3511, REQ-3513, REQ-3517

**Verified by:** TEST-3501 through TEST-3517

---

### CON-3502: Asset Serve Endpoint

**Endpoint:** `GET /assets/{*path}`

**Authentication:** Optional — depends on vault visibility mode and SPL (REQ-3516).

**Pre-conditions:**
- Asset at `{path}` exists in `.zetl/assets/{path}`.
- Caller has `can-read-assets` permission OR vault is public (REQ-3516).

**Success response:**

| Header | Value |
|--------|-------|
| `Content-Type` | MIME type from metadata sidecar |
| `X-Content-Type-Options` | `nosniff` |
| `X-Frame-Options` | `SAMEORIGIN` |
| `Cache-Control` | Per REQ-3515 |
| `Content-Security-Policy` | (HTML assets only) `default-src 'self' 'unsafe-inline' 'unsafe-eval' data: blob:; frame-ancestors 'self'` |
| `ETag` | Hex SHA-256 from metadata sidecar, quoted |

**Body:** Raw file bytes, streamed.

**Error responses:**

| Status | Condition |
|--------|-----------|
| 401 | Not authenticated and vault requires auth |
| 403 | Authenticated but `can-read-assets` not satisfied |
| 404 | Asset does not exist |

**Implements:** REQ-3512, REQ-3514, REQ-3515, REQ-3516

**Verified by:** TEST-3512, TEST-3514, TEST-3515, TEST-3516

---

### CON-3503: Asset List Endpoint

**Endpoint:** `GET /api/assets[?prefix={prefix}]`

**Authentication:** Required, `can-upload` permission.

**Query parameters:**

| Parameter | Required | Description |
|-----------|----------|-------------|
| `prefix` | No | Filter to assets whose slug begins with this string |

**Success response:** `200 OK`

```json
[
  {
    "slug": "diagrams/architecture.png",
    "url": "/assets/diagrams/architecture.png",
    "mime_type": "image/png",
    "size_bytes": 48320,
    "sha256": "3a7bd3e2…",
    "uploaded_by": "alice-abc12345",
    "uploaded_at": "2026-04-22T11:00:00Z",
    "replaced_at": null
  }
]
```

Results are sorted lexicographically by `slug`.

**Error responses:** 401 (unauthenticated), 403 (no `can-upload`), 404 (collab not active).

**Implements:** REQ-3505

**Verified by:** TEST-3505

---

### CON-3504: Asset Delete Endpoint

**Endpoint:** `DELETE /api/assets/{*slug}`

**Authentication:** Required, `can-upload` permission.

**Request headers:**

| Header | Required | Description |
|--------|----------|-------------|
| `X-CSRF-Token` | Yes | CSRF token |

**Success response:** `204 No Content`

**Error responses:** 400 (invalid slug), 401, 403, 404 (slug not found).

**Implements:** REQ-3506

**Verified by:** TEST-3506

---

### CON-3505: SPL Predicates

**New predicates introduced by this specification:**

#### `can-upload` (arity 1)

`(can-upload USER)` — user may upload, replace, delete assets, and list the asset library.

**Built-in default rules (injected by zetl, cannot be overridden by page SPL):**

```spl
;; Full-scope editors can upload.
(normally r-editor-upload
  (role USER editor)
  (scope USER "**")
  (can-upload USER))

;; Vault owner always can upload.
(always s-owner-upload
  (owner USER)
  (can-upload USER))

;; Admins can upload.
(normally r-admin-upload
  (admin USER)
  (can-upload USER))
```

Vault administrators may add additional rules to `.zetl/collab/access.spl` to grant or restrict upload access independently of the editor role. Example — grant upload to a restricted-scope editor:

```spl
(given (can-upload "alice-abc12345"))
```

#### `can-read-assets` (arity 2)

`(can-read-assets USER SCOPE)` — user may read (view) assets. `SCOPE` is `"*"` in v1 (all assets share one policy). Per-asset fine-grained ACL is deferred to a future specification.

**Built-in default rules:**

```spl
;; Authenticated users who can upload can also read.
(normally r-upload-implies-read-assets
  (can-upload USER)
  (can-read-assets USER "*"))

;; Authenticated readers can also read assets.
(normally r-reader-reads-assets
  (role USER reader)
  (in-scope "." USER)
  (can-read-assets USER "*"))

;; Unauthenticated access: allowed iff visibility-mode is transparent.
(normally r-public-assets
  (visibility-mode transparent)
  (can-read-assets anonymous "*"))
```

**Implements:** REQ-3504, REQ-3516

---

### CON-3506: Asset Metadata Sidecar Format

**File location:** `{vault_root}/.zetl/assets/{slug}.meta.json`

**Schema:**

```json
{
  "$schema": "https://zetl.dev/schemas/asset-meta-v1.json",
  "version": 1,
  "slug": "diagrams/architecture.png",
  "original_filename": "architecture.png",
  "mime_type": "image/png",
  "size_bytes": 48320,
  "sha256": "3a7bd3e2360a3d29eea436fcfb7e44c735d117c42d1c1835420b6b9942dd4f1b",
  "uploaded_by": "alice-abc12345",
  "uploaded_at": "2026-04-22T11:00:00Z",
  "replaced_at": null
}
```

**Invariants:**
- `version` is always `1` in this specification. Future versions increment this field; readers MUST reject unknown versions.
- `slug` matches the file's path relative to `.zetl/assets/`.
- `sha256` is a lowercase hex string of exactly 64 characters.
- `uploaded_at` and `replaced_at` (when present) are ISO-8601 timestamps in UTC.
- `size_bytes` matches the actual file size on disk (verified at startup during `init_storage_total`).

**Implements:** REQ-3502, REQ-3511

---

## 9. Test Specifications

### TEST-3501: Collab-Only Gate

**Verifies:** REQ-3501  
**Type:** Integration  
**Technique:** Example-based

Start `zetl serve` without `--collab`. Attempt `POST /api/assets/test.txt` with valid headers. Assert `404 Not Found`. Start `zetl serve --collab`. Same request after authentication. Assert `201 Created` or appropriate 4xx (not 404).

---

### TEST-3502: Asset Stored on Disk

**Verifies:** REQ-3502, CON-3506  
**Type:** Integration  
**Technique:** Example-based

Upload a known file. Assert the file exists at `.zetl/assets/{slug}`. Assert the sidecar exists at `.zetl/assets/{slug}.meta.json`. Assert sidecar JSON is valid against CON-3506 schema. Assert `size_bytes` equals the actual file size.

---

### TEST-3503: MIME Type from Header

**Verifies:** REQ-3503  
**Type:** Integration  
**Technique:** Example-based (two cases)

**Case A:** Upload a PNG body with `Content-Type: image/jpeg`. Assert stored MIME type is `image/jpeg` (header wins over magic bytes). Assert `GET /assets/{slug}` responds with `Content-Type: image/jpeg`.

**Case B:** Upload a PNG body without `Content-Type`. Assert `400 Bad Request` with error key `missing_content_type`.

---

### TEST-3504: Upload ACL Enforcement

**Verifies:** REQ-3504, CON-3505  
**Type:** Integration  
**Technique:** Example-based (three cases)

**Case A:** Restricted-scope editor (scope = `projects/*`) attempts upload. Assert `403 Forbidden`.

**Case B:** Full-scope editor (scope = `**`) attempts upload. Assert `201`.

**Case C:** Reader role attempts upload. Assert `403`.

**Case D:** Owner attempts upload. Assert `201`.

**Case E:** Admin with custom SPL grant `(given (can-upload "{reader_id}"))` in `access.spl`, reader attempts upload. Assert `201`.

---

### TEST-3505: Asset Listing

**Verifies:** REQ-3505, CON-3503  
**Type:** Integration  
**Technique:** Example-based

Upload three assets: `a.png`, `b/c.pdf`, `b/d.html`. Call `GET /api/assets`. Assert array length 3, sorted by slug. Call `GET /api/assets?prefix=b/`. Assert array length 2.

---

### TEST-3506: Asset Deletion

**Verifies:** REQ-3506, CON-3504  
**Type:** Integration  
**Technique:** Example-based

Upload `delete-me.txt`. Assert exists. Call `DELETE /api/assets/delete-me.txt` with CSRF token. Assert `204`. Assert file and sidecar no longer exist on disk. Call `GET /assets/delete-me.txt`. Assert `404`.

---

### TEST-3507: MIME Type Allowlist

**Verifies:** REQ-3507  
**Type:** Integration  
**Technique:** Example-based (representative members of denylist)

Attempt uploads with:
- `Content-Type: application/x-msdownload` → assert `415`
- `Content-Type: application/zip` → assert `415`
- `Content-Type: application/octet-stream` → assert `415`
- `Content-Type: text/html` → assert `201` (in allowlist)
- `Content-Type: image/png` → assert `201` (in allowlist)

Assert `415` body includes `"allowed"` array.

---

### TEST-3508: Per-File Size Limit

**Verifies:** REQ-3508  
**Type:** Integration  
**Technique:** Example-based (boundary cases)

**Case A:** Upload body of exactly `max_bytes - 1`. Assert `201`.  
**Case B:** Upload body of exactly `max_bytes`. Assert `201`.  
**Case C:** Upload body of `max_bytes + 1`. Assert `413` with `{"error": "file_too_large", "max_bytes": N, "received_bytes": N+1}`.

For Case C, verify the server returns `413` before reading the full body (check that the connection is aborted promptly) by setting a short per-test timeout.

---

### TEST-3509: Vault Storage Quota

**Verifies:** REQ-3509  
**Type:** Integration  
**Technique:** Example-based

Configure `--asset-max-total-bytes 20000`. Upload a 10 000-byte file. Assert `201`. Upload another 10 000-byte file. Assert `201`. Attempt a third 1-byte file. Assert `507` with `{"error": "storage_quota_exceeded"}`. Delete one file. Attempt the 1-byte upload again. Assert `201`.

---

### TEST-3510: Slug Validation — Path Traversal

**Verifies:** REQ-3510  
**Type:** Integration  
**Technique:** Adversarial (representative traversal inputs)

Attempt uploads with the following slugs (URL-encoded as appropriate):

| Slug | Expected |
|------|----------|
| `../../../etc/passwd` | 400 |
| `a/../b` | 400 |
| `a//b` | 400 |
| `.hidden` | 200 (leading dot is allowed — it's a hidden file, not `.`) |
| `NUL` | 400 (Windows device name) |
| `COM1.txt` | 400 (Windows device name) |
| `valid/slug.txt` | 201 |
| `` (empty) | 400 |

Also assert that after a successful upload to `valid/slug.txt`, the resolved path is strictly within `.zetl/assets/`.

---

### TEST-3511: Atomic Write and SHA-256

**Verifies:** REQ-3511, NFR-3503  
**Type:** Integration  
**Technique:** Example-based

Upload a file. Compute SHA-256 of the uploaded bytes. Assert the returned `sha256` field matches. Download via `GET /assets/{slug}`. Compute SHA-256 of downloaded bytes. Assert it matches.

---

### TEST-3512: Raw Serving — No Template Wrapper

**Verifies:** REQ-3512  
**Type:** Integration  
**Technique:** Example-based

Upload an HTML file. Download via `GET /assets/{slug}.html`. Assert response body does NOT contain any zetl navigation chrome (`<nav`, zetl CSS class names, etc.). Assert response body contains the verbatim content of the uploaded file.

---

### TEST-3513: Git Commit Attribution

**Verifies:** REQ-3513  
**Type:** Integration  
**Technique:** Example-based

In a vault with a git repository, upload an asset as user `alice-abc12345`. After upload, run `git log --oneline -1` in the vault. Assert the commit message contains `asset: upload`, the slug, and `user: alice-abc12345`. Assert the commit author matches the git user identity (from the vault's `user.email` / `user.name` git config).

---

### TEST-3514: Serving Headers

**Verifies:** REQ-3514  
**Type:** Integration  
**Technique:** Example-based (two cases)

**Case A (HTML asset):** Upload an HTML file. GET it. Assert headers:
- `X-Content-Type-Options: nosniff`
- `X-Frame-Options: SAMEORIGIN`
- `Content-Security-Policy` contains `frame-ancestors 'self'`

**Case B (PNG asset):** Upload a PNG. GET it. Assert `X-Content-Type-Options: nosniff`, assert NO `Content-Security-Policy` header.

---

### TEST-3515: Cache-Control Headers

**Verifies:** REQ-3515  
**Type:** Integration  
**Technique:** Example-based

Upload asset (first create). GET it. Assert `Cache-Control: public, max-age=31536000, immutable`.

Replace asset with `X-Overwrite: true`. GET it again. Assert `Cache-Control: public, max-age=300`.

---

### TEST-3516: Unauthenticated Access Policy

**Verifies:** REQ-3516  
**Type:** Integration  
**Technique:** Example-based (two cases)

**Case A (visibility-mode transparent):** Set vault SPL to `(given (visibility-mode transparent))`. Upload an asset. Send unauthenticated GET to `/assets/{slug}`. Assert `200 OK`.

**Case B (visibility-mode mixed, the default):** Do not set visibility-mode. Upload an asset. Send unauthenticated GET. Assert `401 Unauthorized` with `Location: /auth/login`.

---

### TEST-3517: Overwrite Semantics

**Verifies:** REQ-3517  
**Type:** Integration  
**Technique:** Example-based (all combinations)

| `X-Create` | `X-Overwrite` | Slug exists | Expected |
|------------|---------------|-------------|----------|
| true | false | No | 201 |
| true | false | Yes | 409 |
| false | true | Yes | 200 |
| false | true | No | 404 |
| true | true | No | 201 |
| true | true | Yes | 200 |
| false | false | No | 400 (`missing_create_or_overwrite`) |

---

### TEST-3518: Asset Management UI

**Verifies:** REQ-3518  
**Type:** Integration (browser-level via headless Chromium or similar)  
**Technique:** Example-based

Navigate to `/_admin/assets` as an authenticated editor. Assert page contains:
- A table or list of uploaded assets.
- A file input and slug text field.
- A storage usage summary.

Upload a file via the form. Assert the asset appears in the list. Click the delete button. Assert the asset disappears from the list. Assert `GET /assets/{slug}` returns 404.

---

### TEST-3519: Storage Counter Accuracy

**Verifies:** REQ-3519, NFR-3506  
**Type:** Integration  
**Technique:** Example-based

Upload N files of known sizes. Verify the in-memory counter equals the sum of sizes. Delete one file. Verify counter decrements. Restart the server. Verify counter is re-initialised to the correct sum.

---

### TEST-3525: Partial Upload Cleanup (NFR-3504)

**Verifies:** NFR-3504  
**Type:** Integration  
**Technique:** Example-based

Simulate a connection drop mid-upload by truncating the body. Assert that after the failed request, no file exists at `.zetl/assets/{slug}` and no file exists in `.zetl/assets/.tmp/`.

---

### TEST-3526: Concurrent Uploads (NFR-3505)

**Verifies:** NFR-3505  
**Type:** Integration  
**Technique:** Example-based

Spawn two concurrent upload tasks to different slugs. Assert both complete with 201. Assert both files exist on disk with correct content. Spawn two concurrent upload tasks to the same slug with `X-Overwrite: true`. Assert exactly one file exists on disk (not corrupt).

---

### TEST-3527: Storage Counter Under Concurrency (NFR-3506)

**Verifies:** NFR-3506  
**Type:** Integration  
**Technique:** Property-based (invariant check)

Run 50 concurrent uploads of random sizes, then 50 concurrent deletes. Assert that at no point does the counter drop below 0. Assert that at the end the counter equals the sum of sizes of surviving files.

---

### TEST-3528: Upload Throughput (NFR-3501)

**Verifies:** NFR-3501  
**Type:** Integration, performance  
**Technique:** Example-based (benchmark)

On loopback, upload a 10 MiB file. Assert the response is received within 10 seconds. Run 5 times; assert all 5 complete within budget.

---

### TEST-3529: Serve Latency (NFR-3502)

**Verifies:** NFR-3502  
**Type:** Integration, performance  
**Technique:** Example-based (benchmark)

On loopback, upload a 1 MiB file. Measure time-to-first-byte for 100 sequential GET requests. Assert 95th percentile TTFB ≤ 50 ms.

---

## 10. Observability

---

**OBS-3501: Asset Upload Counter**

**Signal type:** Counter  
**Name:** `zetl_asset_uploads_total`  
**Labels:** `mime_category` (e.g., `image`, `document`, `text`, `data`, `font`, `audio`, `video`), `user_id`  
**Emitted:** On each successful upload (201 or 200)  
**Log line:** `[zetl] asset_upload: slug={slug} size={bytes} mime={mime_type} user={user_id}`

---

**OBS-3502: Asset Upload Failure Counter**

**Signal type:** Counter  
**Name:** `zetl_asset_upload_failures_total`  
**Labels:** `reason` (one of: `acl_denied`, `mime_rejected`, `size_exceeded`, `storage_exceeded`, `invalid_slug`, `missing_content_type`)  
**Emitted:** On each failed upload attempt  
**Log line:** `[zetl] asset_upload_failed: reason={reason} slug={slug} user={user_id}`

---

**OBS-3503: Asset Deletion Counter**

**Signal type:** Counter  
**Name:** `zetl_asset_deletes_total`  
**Labels:** `user_id`  
**Emitted:** On each successful delete  
**Log line:** `[zetl] asset_delete: slug={slug} user={user_id}`

---

**OBS-3504: Asset Serve Latency**

**Signal type:** Histogram (buckets: 5ms, 10ms, 25ms, 50ms, 100ms, 250ms, 500ms, 1s)  
**Name:** `zetl_asset_serve_latency_ms`  
**Labels:** `mime_category`, `cached` (boolean, from ETag 304 response)  
**Emitted:** On each GET /assets/{path} response  

---

**OBS-3505: Asset Storage Gauge**

**Signal type:** Gauge  
**Name:** `zetl_asset_storage_bytes`  
**Labels:** (none)  
**Updated:** On upload, delete, and server startup (re-scan)  
**Log line at startup:** `[zetl] assets: storage_bytes={N} max_bytes={M} count={C}`  
**Log line at 90% capacity:** `[zetl] assets: storage_warning: used={N} max={M} (90%)`

---

## 11. Security Analysis

### 11.1 Threat Model

Assets are uploaded by authenticated, authorised collaborators. The primary threat is not external attackers (they cannot upload) but rather:

1. **Privilege escalation via malicious HTML:** A compromised collaborator account uploads an HTML page designed to steal session tokens or perform API mutations.
   - **Mitigated by:** `HttpOnly` session cookies (SPEC-020 CON-020-001) prevent JS from reading the cookie. All state-mutating API endpoints require `X-CSRF-Token` which cannot be synthesised by JS in an uploaded page without access to the session cookie.
   
2. **Disk exhaustion:** A collaborator repeatedly uploads large files to fill the vault's disk.
   - **Mitigated by:** Per-file (REQ-3508) and total (REQ-3509) storage limits. OBS-3505 emits a warning at 90% capacity.
   
3. **Path traversal:** A crafted slug causes a write outside `.zetl/assets/`.
   - **Mitigated by:** Two-layer defense: high-level slug validation (REQ-3510) rejects known traversal patterns; canonical path resolution verifies the final path is within the assets root before any I/O.

4. **MIME type confusion / content sniffing:** A file uploaded with a benign MIME type is re-interpreted by the browser as a dangerous type.
   - **Mitigated by:** `X-Content-Type-Options: nosniff` (REQ-3514) prevents browser content-type sniffing.

5. **Symlink attack:** An attacker creates a symlink in `.zetl/assets/` pointing outside the vault.
   - **Mitigated by:** The serve handler resolves the canonical path and checks it lies within `.zetl/assets/` before opening the file. Symlinks that escape the assets directory are rejected.

6. **Storage side-channel (timing attack on slug existence):** The 404 vs 409 distinction reveals whether a slug exists to unauthenticated callers.
   - **Assessment:** Upload endpoints require authentication; unauthenticated callers cannot reach the slug-exists check. Non-issue.

7. **Git bomb / large-file history inflation:**
   - **Mitigated by:** The per-file limit (REQ-3508) caps individual asset size. Operators who need large binary assets are advised to use `git-lfs`.

### 11.2 AI Trust Boundary

This specification was produced by claude-sonnet-4-6 under USDD Protocol v1.3.0. Per the protocol's review tiers:

- Security-critical sections (§11, REQ-3510, REQ-3514, CON-3501/3502, ADR-3504/3505) are **Tier 2** — require cross-model review plus human review before implementation.
- Core feature logic (storage, ACL, serving) is **Tier 3** — requires fresh-context review.
- UI and documentation are **Tier 4**.

---

## 12. Known Limitations (v1)

1. **Multi-file HTML bundles:** An HTML page with sibling JS/CSS files must have each file uploaded separately at the correct relative slug. Recommend inlining assets at build time.
2. **No per-asset ACL:** All assets share the vault-wide `can-read-assets` policy. Fine-grained per-folder or per-file ACL is a future extension.
3. **No asset search:** Assets are not indexed in Tantivy. They are not returned by `/api/search`.
4. **No asset backlinks:** The vault link graph does not track Markdown links to `/assets/…` paths. A future extension could build an asset reference index.
5. **Git repository size:** Uploaded binary assets inflate git history. Large media workflows should consider `git-lfs` (outside zetl's scope).
6. **No directory listing:** `GET /assets/` without a slug returns 404. Browsing the asset tree requires the `/_admin/assets` UI or the `GET /api/assets` API.

---

## 13. Open Questions

1. **Agent token upload:** Should agent tokens (used by LLM agents, SPEC-020) have `can-upload` by default? The current SPL default grants it to editors with global scope; agents derive from an editor's token. This may be intentional (agents upload images for the pages they edit) or undesirable (agents could exhaust storage). **Proposed resolution:** add to v1 default SPL: `(except d-agent-no-upload (is-agent) (not (can-upload USER)))` — agents cannot upload unless an explicit SPL rule grants it. Human review required.

2. **Replace semantics and ETag caching:** When an asset is replaced, existing browsers with the immutable-cached version will not re-fetch for up to 1 year. The `max-age=300` post-replace header helps for new fetches, but cached clients are unaffected. Should the slug change on replace (content-addressed slugs)? **Proposed resolution:** Document as a limitation. Operators who need strict cache invalidation should append a version suffix to the slug.

3. **`/_admin/assets` access gate:** Should the admin assets UI be restricted to users with `can-upload`, or to admins/owners only? The `can-upload` default grants it to full-scope editors, which may be too broad. **Proposed resolution:** Use `can-upload` as the gate for the full list (editors can see and upload their own files); restrict the delete button to uploaders and admins (`can-upload AND (uploaded_by == me OR admin)`). Deferred to implementation.

---

## 14. Out of Scope

See §1.4 for the v1 scope boundary. Items explicitly deferred:

- **CDN/object-storage backend:** Future multi-tenant or cloud-hosting tier.
- **Image resizing pipeline:** Deferred. Recommend client-side tooling.
- **Asset versioning UI:** Git provides the history; the web UI does not surface it in v1.
- **Asset search integration:** Requires Tantivy indexing of asset metadata, separate effort.
- **WebDAV / S3-compatible protocol:** Separate specification if demanded.
- **Zip extraction:** Security risk surface exceeds v1 scope.
- **Video streaming (range requests):** HTTP range requests enable video scrubbing; not implemented in v1. Videos are served as complete responses.
