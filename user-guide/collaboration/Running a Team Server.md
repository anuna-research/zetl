---
title: Running a Team Server
tags: [collaboration, setup, server]
---

# Running a Team Server

ztl's `--collab` flag turns `ztl serve` into a multi-user editing server: passkey login, real-time CRDT co-editing, per-user git commits, and scoped invitations. Everything is in-tree — there's no feature flag to enable at install time. If you can run `ztl serve`, you can run a team server.

## The shape of a collab vault

A collab vault is a normal Markdown vault plus a `.ztl/collab/` directory:

- `.ztl/collab/server.key` — the server's Ed25519 signing key (for session cookies and invitations).
- `.ztl/collab/users.db` — SQLite store for passkeys, sessions, and invitation nonces.
- `.ztl/collab/wal/` — write-ahead log for the CRDT engine. Survives crashes.

These files are created on first run. You never edit them by hand.

## First run: bootstrap the owner

The very first time you start a collab server, you need to bootstrap an owner account. `--init-owner` creates one, prints a 12-word recovery phrase, and exits into the normal serve loop:

```bash
ztl -d ~/notes serve --collab --init-owner --owner-name Alice
```

The terminal will print something like:

```
Owner created: Alice
Recovery phrase (write this down — you will not see it again):

  palm  tornado  ladder  oyster  casual  umbrella
  desert  finger  enlist  brave  coconut  strong

Server listening on http://localhost:3000
```

**Save that phrase.** It's your only way back into the account if you lose your passkey, and ztl will not show it to you a second time. Paper, password manager, encrypted notes file — pick one and commit.

Open `http://localhost:3000` in your browser, log in as Alice, and register a passkey (Touch ID, YubiKey, or any WebAuthn authenticator). That's the end of setup. See [[Passkeys and Accounts]] for the full dance.

## Subsequent starts

After the owner exists, you start the server with just `--collab`:

```bash
ztl -d ~/notes serve --collab
```

The default port is 3000 and the default bind is localhost. Most of the plain-`serve` flags still work — `--port`, `--theme`, `--public`. Two extras are worth knowing:

| Flag | When you want it |
|------|------------------|
| `--hostname mysite.fly.dev` | Public hostname for WebAuthn. Passkeys are bound to a hostname — if you intend to serve over a real domain, set it. Also env: `ztl_HOSTNAME`. |
| `--git-poll-interval 30s` | How often ztl checks `git` for external commits (so an edit you push from another machine shows up in the UI). Set `0` to disable. |

## Containerised or ephemeral deployments

If your filesystem is ephemeral (Docker, Fly.io, Nomad, a VM you redeploy), the server key in `.ztl/collab/server.key` gets wiped on every redeploy — which invalidates every live session. Not ideal.

The fix is a deterministic server key, derived from a BIP39 mnemonic you control:

```bash
ztl -d /vault serve --collab \
  --server-key-seed "palm tornado ladder oyster casual umbrella \
                     desert finger enlist brave coconut strong"

# Or via environment, which is what you usually want in a container
export ztl_SERVER_KEY_SEED="palm tornado ladder …"
ztl -d /vault serve --collab
```

The same mnemonic also derives the git SSH key (via `ztl derive-ssh-key`) and headless agent tokens (via `ztl agent-token`). One seed, three uses. See [[Passkeys and Accounts]] for the derivation paths and [[Co-editing]] for how git push fits in.

## What's next

- **Making accounts:** [[Passkeys and Accounts]].
- **Letting people in:** [[Invitations]].
- **Editing simultaneously:** [[Co-editing]].
- **Scoping who can see what:** [[Access Control]].
- **Publishing read-only without a server at all:** [[Capability URLs]].

## Related

- [[Web Server]]
- [[Installation]]
- [[Passkeys and Accounts]]
- [[Invitations]]
- [[Capability URLs]]
