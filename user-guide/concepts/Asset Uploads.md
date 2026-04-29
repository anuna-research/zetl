---
title: Asset Uploads
tags: [collaboration, assets, concepts]
---

# Asset Uploads

In collaborative mode, authorised editors can upload static assets — images, PDFs, audio, video, fonts, data files, and even standalone HTML pages — directly into the vault. Assets are stored under `.zetl/assets/` and served at `/assets/{*path}` with the same vault-wide access controls that govern page editing.

---

## What you can upload

The MIME-type allowlist covers the categories most knowledge bases need:

| Category | Example MIME types |
|----------|-------------------|
| Images | `image/png`, `image/jpeg`, `image/webp`, `image/svg+xml`, `image/gif` |
| Documents | `application/pdf`, `application/epub+zip` |
| Text | `text/plain`, `text/html`, `text/css`, `text/javascript` |
| Data | `application/json`, `text/csv`, `application/xml` |
| Fonts | `font/woff2`, `font/otf` |
| Audio | `audio/mpeg`, `audio/ogg`, `audio/wav` |
| Video | `video/mp4`, `video/webm` |

If you need a type that is not on the list, convert it to an allowed format or open an issue.

---

## How to upload an asset

### Via the admin UI (browser)

1. Start the server in collaborative mode: `zetl serve --collab`
2. Log in and navigate to `/_admin/assets`
3. Pick a file, edit the slug if desired, and click **Upload**
4. The asset is immediately available at `/assets/{slug}`

### Via the API (curl / script)

```bash
# Upload a new image
curl -X POST \
  -H "Content-Type: image/png" \
  -H "X-Create: true" \
  -H "Cookie: zetl_session=$SESSION" \
  -H "X-CSRF-Token: $CSRF" \
  --data-binary @diagram.png \
  http://localhost:3000/api/assets/images/diagram.png

# Replace an existing image
curl -X POST \
  -H "Content-Type: image/png" \
  -H "X-Overwrite: true" \
  -H "Cookie: zetl_session=$SESSION" \
  -H "X-CSRF-Token: $CSRF" \
  --data-binary @diagram-v2.png \
  http://localhost:3000/api/assets/images/diagram.png

# List all assets
curl -H "Cookie: zetl_session=$SESSION" \
  http://localhost:3000/api/assets

# Delete an asset
curl -X DELETE \
  -H "Cookie: zetl_session=$SESSION" \
  -H "X-CSRF-Token: $CSRF" \
  http://localhost:3000/api/assets/images/diagram.png
```

---

## Publish a static HTML report

HTML assets are treated specially: they are served with a `Content-Security-Policy` header that restricts them to the same origin, preventing uploaded pages from reading session cookies or forging API requests.

To publish a standalone HTML report:

1. Generate your report as a single `.html` file (inline all CSS/JS).
2. Upload it with `Content-Type: text/html`.
3. Share the `/assets/{slug}` URL.

Because of the CSP, the page cannot make cross-origin requests or access `HttpOnly` cookies, so it is safe to share even with untrusted readers.

---

## Storage limits

Two limits protect the server from disk exhaustion:

- **Per-file limit** (default: 10 MiB) — rejects individual files larger than the limit with HTTP 413.
- **Total vault limit** (default: 100 MiB) — rejects uploads that would push the vault past the limit with HTTP 507.

Both can be configured at startup:

```bash
zetl serve --collab --asset-max-file-bytes 52428800 --asset-max-total-bytes 1073741824
```

When usage exceeds 90 % of the total limit, a warning is logged:

```
[zetl] assets: storage_warning: used=94371840 max=104857600 (90%)
```

---

## Access control

Uploading requires the `can-upload` SPL predicate. By default, owners, admins, and regular editors can upload. Agents (synthetic users) are denied unless explicitly granted.

Reading assets requires `can-read-assets`. In **transparent** visibility mode, anyone can read. In **mixed** or **private** mode, only authenticated users with the predicate can read.

See [[Access Control]] for the full ACL model, and `docs/asset-acl.md` for ready-to-paste SPL rules.

---

## Git integration

Every upload, replace, and delete is automatically committed to the vault's git repository (if one exists). Commit messages look like:

```
asset: upload images/diagram.png (12 KiB) [user: alice-…]
asset: replace images/diagram.png (15 KiB) [user: alice-…]
asset: delete images/diagram.png [user: alice-…]
```

This keeps asset history alongside page history. Large binary assets will inflate git history over time; consider `git-lfs` for media-heavy workflows.

---

## Troubleshooting

| Error | HTTP status | What to do |
|-------|-------------|------------|
| `missing_content_type` | 400 | Add a `Content-Type` header matching the file type. |
| `mime_type_not_allowed` | 415 | Convert the file to an allowed MIME type. |
| `file_too_large` | 413 | Compress the file, or raise `--asset-max-file-bytes`. |
| `storage_quota_exceeded` | 507 | Delete old assets, or raise `--asset-max-total-bytes`. |
| `invalid_slug` | 400 | Use a safe path: no `..`, no empty components, no leading/trailing slashes. |
| `slug_exists` | 409 | Use `X-Overwrite: true` to replace, or choose a different slug. |
| `missing_create_or_overwrite` | 400 | Include either `X-Create: true` or `X-Overwrite: true`. |
| `unauthenticated` | 401 | Log in first. |
| `forbidden` | 403 | Your role does not have `can-upload`. Ask an admin. |

---

## Known limitations (v1)

- **No per-asset ACL:** All assets share the vault-wide `can-read-assets` policy.
- **No asset search:** Assets are not indexed in Tantivy and do not appear in search results.
- **No asset backlinks:** The link graph does not track Markdown links to `/assets/…` paths.
- **Multi-file HTML bundles:** An HTML page with sibling JS/CSS files must have each file uploaded separately. Inline assets at build time for best results.
