---
title: Invitations
tags: [collaboration, auth, roles]
---

# Invitations

New collaborators join a vault via a single-use, signed **invitation token**. You generate one with `zetl invite` (or the web UI), send the resulting link by whatever out-of-band channel you prefer, and the recipient uses it exactly once to register a passkey and pick a display name.

## Generating an invite from the CLI

The minimal form takes your own username (the inviter) and the role the invitee will have:

```bash
zetl invite --as Alice --role editor
```

This creates the token and copies the full invitation URL to the clipboard. Paste it into Signal, email, a sticky note — whatever gets it to the recipient without a third party logging it along the way.

Three roles are available:

| Role | What they can do |
|------|------------------|
| `reader` | View pages; no editing, no invitations. |
| `editor` | View and edit pages; cannot invite new users or change access. |
| `admin` | Full control, including issuing invitations and managing passkeys. |

## Scoping an invite to specific pages

By default an invite gives the recipient access to the whole vault at their role. For a contractor who should only touch one project, or a reviewer who only needs to see a single folder, use `--pages` with a glob:

```bash
# A reader who only sees pages under projects/website/
zetl invite --as Alice --role reader --pages "projects/website/*"

# An editor scoped to a specific topic
zetl invite --as Alice --role editor --pages "research/biology/**"
```

Globs match on the vault-relative path. `projects/*` matches direct children; `projects/**` matches everything underneath, recursively. For more expressive rules — "editor on everything *except* `billing/**`", or "editor only between 9–5" — see SPL-based deontic rules in [[Access Control]].

## Custom expiry

Invitation links expire. The default is 72 hours, which is long enough for someone to find the email but short enough that a forgotten link doesn't become a permanent back door:

```bash
# One-day window for a meeting-day onboarding
zetl invite --as Alice --role editor --expires 24h

# A week for someone who travels
zetl invite --as Alice --role reader --expires 7d
```

Accepted units are `h` (hours) and `d` (days). After expiry the token refuses to redeem even if it hasn't been used.

## Using the web UI instead

If the CLI isn't handy, `/_admin/invite` does the same thing with a form. Admins can also see a list of outstanding invites, revoke un-redeemed ones, and check which tokens have been used. This is the easier surface for day-to-day invitations; the CLI is better for scripting and one-liners.

## What's in the token

Each invite is an Ed25519-signed blob containing the role, optional page scope, expiry, and a single-use nonce. The server refuses to redeem a token whose nonce it has already seen, so a link can't be replayed even if someone intercepts it.

The server key that signs these tokens is the same key derived from your `--server-key-seed` mnemonic — see [[Passkeys and Accounts]]. If you rotate the seed, all outstanding invites become invalid.

## Accepting an invite

The recipient opens the link, enters a display name, registers a passkey, and is dropped into the vault at their scoped role. They don't need their own 12-word phrase at this point — zetl generates one and shows it to them during onboarding, to save for recovery. (They should save it. See [[Passkeys and Accounts]].)

## Revoking access

To remove a collaborator entirely, an admin deletes the user from `/_admin/users`. To narrow scope, issue a new, more restrictive invite and revoke the old role. There's no separate "suspend" state in v0.5; access is either on or off.

## Related

- [[Running a Team Server]]
- [[Passkeys and Accounts]]
- [[Access Control]]
- [[Co-editing]]
- [[Capability URLs]]
