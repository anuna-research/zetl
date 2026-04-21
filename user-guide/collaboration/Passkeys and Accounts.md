---
title: Passkeys and Accounts
tags: [collaboration, auth, recovery]
---

# Passkeys and Accounts

zetl doesn't do passwords. Instead, every collab account is backed by one or more **WebAuthn passkeys** — Touch ID, Face ID, Windows Hello, a YubiKey, or any FIDO2 authenticator. A 12-word BIP39 **recovery phrase** is the break-glass fallback, and the same phrase deterministically derives the collab server's signing key and a git SSH key.

## Why passkeys

Passwords get phished, reused, and leaked. Passkeys are bound to the origin (so a phishing site can't steal them), require a local user gesture (touch, face, PIN), and never leave your device in usable form. For a small team vault, this is the right default: lower operational burden than password-plus-2FA, and nothing to leak in a database breach.

## Registering on first login

The first time you open a collab server, you log in by **display name** (e.g. `Alice`) and then register a passkey. The browser prompts whatever authenticator is available — Touch ID on a Mac, Windows Hello on a PC, a hardware key, or a phone acting as a security key.

Subsequent logins are one tap. No username/password fields.

You can register **multiple passkeys** on one account — laptop Touch ID, phone Face ID, a YubiKey in a drawer. If one device dies, the others still work. Manage them from `/_admin/passkeys` in the web UI.

## Recovery

If you lose access to every passkey — laptop stolen, phone dead, YubiKey at the bottom of a river — you recover using your 12-word phrase:

1. Open `/auth/recovery` on the server.
2. Enter your display name and the 12 words.
3. zetl issues a fresh session and deletes all your existing passkeys.
4. Register a new passkey on whatever device you're on now.

Write the phrase down when you first see it. Don't type it into a browser extension. Don't email it to yourself. Paper in a drawer, or a line in your password manager, is the sweet spot for a small team.

## BIP39 and SLIP-0010: one seed, three keys

The same 12 words derive three separate keys, at distinct SLIP-0010 paths:

| Path | Purpose | How it's used |
|------|---------|---------------|
| `m/44'/0'/0'` | User account | Generated at `--init-owner`; recovers the passkey-holding account. |
| `m/44'/1'/0'` | Server signing key | Passed to `serve --collab --server-key-seed`. See [[Running a Team Server]]. |
| `m/44'/2'/0'` | Git SSH key | Derived on demand with `zetl derive-ssh-key`. |

The paths are deterministic: given the phrase, the key is always the same. This is why a single mnemonic safely covers an ephemeral redeploy — the new container derives the identical server key, so existing sessions keep working.

## Reproducing the SSH key anywhere

If you push the vault's git repo to a remote (GitLab, GitHub, Forgejo, codeberg), you want a stable SSH key for the remote. `zetl derive-ssh-key` materialises the ed25519 key from the mnemonic:

```bash
zetl derive-ssh-key \
  --mnemonic "palm tornado ladder oyster casual umbrella \
              desert finger enlist brave coconut strong" \
  --out ~/.ssh/id_ed25519_zetl
```

It writes the private key to `--out` (or stdout if omitted) and prints the public key so you can paste it into your git host's SSH settings. Redeploy the server a year later, derive again, same key — no need to rotate the git remote.

You can also derive headless **agent tokens** from the same seed for CI and bots:

```bash
zetl agent-token --mnemonic "palm tornado ladder …"
```

Use the printed token as `Authorization: Bearer <token>` against the zetl API.

## A note on threat models

Passkeys protect you from remote phishing. They don't protect you from a coworker who sits at your unlocked laptop. The recovery phrase is the single highest-value secret in the system — treat it as such. See [[Access Control]] for server-side scoping of what each account can actually see.

## Related

- [[Running a Team Server]]
- [[Invitations]]
- [[Access Control]]
- [[Co-editing]]
- [[Installation]]
