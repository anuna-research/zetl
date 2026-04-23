# Asset Upload ACL Examples

Three ready-to-use SPL rule sets for controlling who can upload and read static assets. All examples assume `--features reason` is enabled.

---

## 1. All editors can upload

This is the default behaviour when no custom rules are present. Owners, admins, and regular editors can upload; agents cannot.

```spl
(s-owner-upload ?u)
(s-admin-upload ?u)
(r-editor-upload ?u)

(normally can-upload
  (and (or (s-owner-upload ?u)
           (s-admin-upload ?u)
           (r-editor-upload ?u))
       (not (is-agent ?u)))
  (can-upload ?u))
```

---

## 2. Restrict uploads to admins only

Prevent regular editors from uploading assets while preserving their ability to edit pages.

```spl
(s-owner-upload ?u)
(s-admin-upload ?u)

(normally can-upload
  (or (s-owner-upload ?u)
      (s-admin-upload ?u))
  (can-upload ?u))
```

Place this in a file with an `!acl` tag, e.g. `vault/admin-only-assets.md`:

```markdown
---
tags: [acl]
---

# Admin-only asset uploads

```spl
(s-owner-upload ?u)
(s-admin-upload ?u)

(normally can-upload
  (or (s-owner-upload ?u)
      (s-admin-upload ?u))
  (can-upload ?u))
```
```

---

## 3. Public unauthenticated reads

Allow anyone to view assets even when the vault uses mixed or private visibility. Uploading still requires authentication and `can-upload`.

```spl
(normally can-read-assets
  (true)
  (can-read-assets anonymous "*"))
```

This opens **only** the `/assets/{*path}` endpoints; all other pages remain governed by the vault's visibility mode.

---

## How it works

The asset system introduces two SPL predicates:

| Predicate | Evaluated when | Default behaviour |
|-----------|----------------|-------------------|
| `can-upload` | `POST /api/assets/{slug}` | Owners, admins, editors allowed; agents denied |
| `can-read-assets` | `GET /assets/{path}` | Mirrors vault visibility mode |

Both predicates are injected with built-in default rules. Custom rules in `!acl`-tagged pages override or extend these defaults through the normal SPL defeasible reasoning pipeline.

---

## Verifying your rules

Test your ACL configuration before trusting it in production:

```bash
zetl -d ./my-vault reason status
```

Look for `can-upload` conclusions. If a user should be able to upload but is not listed, check that their role facts are present and that no superior rule is defeating the grant.

For debugging, run the built-in integration tests:

```bash
cargo test --test asset_integration
```
