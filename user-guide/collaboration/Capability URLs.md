---
title: Capability URLs
tags: [collaboration, static, sharing, capability]
---

# Capability URLs

**Capability mode** is zetl's answer to: "I want to share my wiki with a dozen friends, read-only, without running a server, but I don't want the whole internet reading it." The output is a plain static site — hostable on any CDN, any S3 bucket, any sftp'd directory — where access is gated by a **secret in the URL fragment**. No accounts, no server-side auth, nothing to rotate but the URLs themselves.

If you want multi-user editing, you want [[Running a Team Server]]. Capability mode is for publishing.

## The idea in one paragraph

Each page is encrypted at build time with a cohort-specific key. Invite URLs look like `https://wiki.example.com/welcome.html#k=<base64-secret>`. The `#fragment` — by design of the HTTP spec — is never sent to the server. The reader's browser, executing a small JavaScript shim, pulls the key out of the fragment, decrypts the page locally, and verifies a vault-level Ed25519 signature so nobody can serve them a forged page. Revocation is per-cohort: you rotate the cohort's salt and re-deploy, old URLs stop decrypting.

Formal spec: see `docs/capability-mode.md` in the zetl repo.

## Setting up a capability site

### 1. Generate the keys

```bash
zetl cap genkey
```

This prints two secrets to stdout, exactly once:

- `ZETL_CAP_SECRET` — the 48-byte content-encryption secret.
- `ZETL_CAP_SIGNING_KEY` — the Ed25519 vault-signing private key.

Capture them somewhere safe (a secret store, a password manager, a sealed envelope). You'll need `ZETL_CAP_SECRET` on every build and `ZETL_CAP_SIGNING_KEY` whenever you rotate the signing key.

### 2. Issue an invite

```bash
zetl cap invite alice \
  --cohort eng \
  --site-url https://wiki.example.com
```

`--cohort` groups grants that share a content key. Rotate a cohort without touching the others. `--site-url` is the canonical hostname the invite URL points at (also settable via `ZETL_CAP_SITE_URL`).

The command prints an invite URL like:

```
https://wiki.example.com/welcome.html#k=TzN1ej...
```

Send it by whatever channel works — Signal, in-person, a QR code. The part after `#` is the secret; if you leak the URL, you leak read access.

Narrow an invite to a subset of pages with `--pages`, and set a TTL with `--expires`:

```bash
zetl cap invite bob \
  --cohort ops \
  --expires 7d \
  --pages 'projects/*' \
  --site-url https://wiki.example.com
```

### 3. List, revoke, rotate

```bash
# See who you've invited
zetl cap list
zetl cap list --cohort eng

# Revoke a specific grant by id (see list output)
zetl cap revoke <grant-id>

# Rotate a cohort's content-key salt — URLs stay the same by design,
# but old readers need re-issued keys (see SPEC-034 REQ-3402).
zetl cap rotate --cohort eng
```

After revoke or rotate, **rebuild and redeploy**. The old ciphertext is still out there on anyone's browser cache, but new ciphertext won't decrypt under an old key, and a well-configured CDN will have dropped the stale response by the time a reader returns.

## Two-operator handoff: `zetl cap pair`

For higher-stakes vaults, you don't want the inviter to see the reader's private key even briefly. `zetl cap pair` runs a **SPAKE2 pubkey handoff** authenticated by a short spoken phrase:

- **Grantor** (you) runs `zetl cap pair --grantor`. It generates a fresh 4-word BIP39 phrase, prints it, and starts a SPAKE2 session. You read the phrase aloud to the other operator.
- **Grantee** (the other operator) runs `zetl cap pair --grantee --peer <handshake> --phrase "<4 words>" --pubkey <their-pubkey>`. They send their outbound handshake and HMAC tag back to you.
- You paste those into the blocked grantor session. On a matching phrase, SPAKE2 derives the same shared key on both sides and the HMAC verifies; the authenticated pubkey is now yours to use with `zetl cap invite --recipient`.

If the phrase differs — because someone in the middle tried to substitute one — the HMAC fails and nothing goes through. This is the "high-value vault" workflow; for friends-and-family scale, the default delegated URL mode is fine.

## Split-key mode for high-value vaults

A capability URL is bearer-auth: anyone with the URL reads. For truly sensitive cohorts, `--split-key` divides the private key between the URL fragment and a **second factor** — either a spoken phrase the reader types in, or a QR code from a separate device — so neither alone unlocks the vault:

```bash
zetl cap invite carol \
  --cohort board \
  --split-key \
  --site-url https://wiki.example.com
```

Split-key mode must be enabled in the vault config first (`[access.split_key] enabled = true`). Without that, `--split-key` is rejected. The second factor is a build-time configuration; default is `spoken-phrase` (the reader types it in), and `qr` is a planned alternative.

## Other useful subcommands

| Command | What it does |
|---------|--------------|
| `zetl cap finalise <grant>` | Mark a grant as operator-confirmed after first use, so the capability binds to a known device (TOFU). |
| `zetl cap check` | Stale-grant and public-safety audit. Catches capabilities drifting past their useful life. |
| `zetl cap sweep` | Bulk-revoke past-expiry grants. |
| `zetl cap audit-diff` | Scan a vault diff for malicious-content patterns (e.g. a hostile theme attempting exfiltration). |
| `zetl cap rotate-signing-key` | Rotate the vault signing key. Rebuild required — every page is re-signed. |
| `zetl cap emergency-shutdown` | Print the operator checklist for pulling a capability site offline in a hurry. Doesn't modify files. |

## When to use capability URLs vs a collab server

- **Capability URLs**: publishing read-only to a known group. No server to run. No accounts to manage. Hostable anywhere static. Revocation is per-cohort, not per-user-finely — if you need to yank one reader mid-cohort, that's a rotate.
- **Collab server**: two or more people actively editing. Real-time merge. Per-user commits. Needs a box to run `zetl serve --collab` on.

For the full operator spec — threat model, deploy recipes for Nginx/CloudFront, rotation ordering, CSP headers — see `docs/capability-mode.md` in the zetl repo.

## Related

- [[Running a Team Server]]
- [[Static Site Export]]
- [[Invitations]]
- [[Access Control]]
- [[Web Server]]
