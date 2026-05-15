---
title: Authentication Methods
tags: [collaboration, auth, config, security]
---

# Authentication Methods

zetl's `--collab` mode picks how a request gets authenticated from an
**ordered list** in `.zetl/config.toml`:

```toml
[collab.auth]
methods = ["passkey", "agent-token"]   # the default
```

A vault with no `[collab.auth]` block behaves exactly as
[[Passkeys and Accounts]] and [[Running a Team Server]] describe — this
page documents what you can put in the `methods` list to bring other
options online.

The full spec is
[SPEC-041](https://codeberg.org/anuna/zetl/src/branch/main/specs/SPEC-041-pluggable-collab-auth.md);
the operator-facing guide ships at
[`docs/collab-auth.md`](https://codeberg.org/anuna/zetl/src/branch/main/docs/collab-auth.md);
the threat model is in
[`research/SPEC-041-threat-model.md`](https://codeberg.org/anuna/zetl/src/branch/main/research/SPEC-041-threat-model.md).

## The chain mental model

Every request walks the `methods` list **once**, first-match-wins. Each
method returns one of:

- **Authenticated** — the request proceeds as that method's principal.
- **Abstain** — the method has nothing to say about this request; the
  chain proceeds to the next entry.
- **Reject** — the method hard-fails the request (a present-but-invalid
  credential it wanted to fail closed on); the chain stops.

A single `auth_resolve` middleware does this once per request. Every
downstream gate (`collab_gate`, `admin_gate`, the CSRF guard) reads the
resolved principal — they never re-parse cookies or headers.

A startup line shows you the assembled chain:

```
[zetl] collab auth: methods=[passkey, agent-token] features=[collab]
```

## The six methods

### passkey (default)

WebAuthn passkey login → cookie session. Identical to pre-SPEC-041
behaviour. See [[Passkeys and Accounts]] for everything about
registration, recovery, and multi-device. No `[collab.auth.passkey]`
sub-table — no config.

### agent-token (default)

Ed25519-signed bearer tokens for headless / programmatic access.
Identical to pre-SPEC-041 behaviour. See [[Invitations]] (under
"Agent tokens") and the `zetl agent-token` reference. Stateless principal
— the CSRF guard exempts agent-token requests automatically. No sub-table.

### proxy-header

For deployments behind an authenticating proxy (oauth2-proxy, Authelia,
Tailscale Serve, Cloudflare Access, an SSO ingress). The proxy
authenticates and forwards the user's identity in a header:

```toml
[collab.auth]
methods = ["proxy-header", "agent-token"]

[collab.auth.proxy_header]
user_header    = "X-Forwarded-User"            # default
peer_allow     = ["127.0.0.1/32", "10.0.0.0/8"] # required, non-empty
auto_provision = true                            # opt-in
```

Run `zetl serve --collab --trust-proxy`. **Both** must hold for the
header to be honoured: `--trust-proxy` AND the request's peer IP in
`peer_allow`. Headers from any other peer are ignored. The header value
itself is recognised against a strict grammar — values outside
`A-Z a-z 0-9 . _ - @ +` (and not `.`/`..`) are rejected, not normalised.

Your proxy MUST also strip inbound copies of the configured header from
client requests — zetl's peer check is the second line of defence.

### password

argon2id static passwords, for small teams without an IdP:

```toml
[collab.auth]
methods = ["password", "agent-token"]
```

Manage credentials with `zetl collab passwd` (see
[[#zetl collab|the CLI reference]]):

```
zetl collab passwd add <user>      # TTY prompt — never argv, never env
zetl collab passwd list            # user_ids only — no hashes
zetl collab passwd remove <user>
```

Users hit `/auth/password`, submit name + password, get an ordinary
session cookie indistinguishable downstream from a passkey session.
Verification is constant-time and cause-indistinguishable — wrong
password and unknown user produce the same response shape and timing.
Storage: `.zetl/collab/passwords.json`, mode 0600, argon2id PHC
strings (parameters embedded so cost can be raised without breaking
existing hashes).

### capability-url

The "anyone with the link" path. Mint a signed `?cap=<token>` URL bound
to a scope glob and a role; recipients open the URL and get scoped,
time-bounded, pseudonymous access — no account, no profile, no login.

```toml
[collab.auth]
methods = ["passkey", "capability-url", "agent-token"]

[collab.auth.capability_url]
default_ttl = "7d"
max_ttl     = "90d"
```

Mint, list, revoke:

```
zetl collab share mint --scope "review/draft-7/**" --role reader \
                       --expires 7d \
                       --site-url https://wiki.example.com
zetl collab share list
zetl collab share revoke <jti>
```

> ⚠ **The URL is a bearer token.** The mint command prints a SECURITY
> notice on stderr — read it. A leaked URL exposes the granted slice
> (scope + role) until expiry or revocation; it does **not** expose the
> rest of the vault. Capability principals never satisfy `admin_gate`
> regardless of the encoded role, and the gate rejects request paths
> with `..` segments (raw or percent-encoded) so the bound scope can't
> be escaped.

**Sensitivity ceiling.** Use this method for low-to-moderate
sensitivity scopes — public draft reviews, share-with-a-friend, content
that's mostly public anyway. For anything sensitive prefer passkey or
OIDC. The full trade-off (URL = bearer, leakage vectors and their
mitigations) is documented in `research/SPEC-041-threat-model.md`
section H.

zetl sets `Referrer-Policy: no-referrer` on every response to a
`?cap=`-bearing request so the token does not leak via outbound
`Referer` headers. The token is stripped from the audit log
(`auth-audit.log` records the pseudonymous `cap-<jti-prefix>` handle).

> **Disambiguation.** SPEC-041's `capability-url` is **not** the same
> thing as SPEC-034 [[Capability URLs|capability mode]]. SPEC-034 is a
> *static site* with reader-side fragment-decryption — no running
> server. SPEC-041's `capability-url` is a *server-side authenticator*
> for `--collab` — the URL is a query parameter and the live server
> verifies the token, gates the scope, and renders the same dynamic
> pages everyone else sees.

### oidc

Build with `--features collab,collab-oidc` (the OIDC code path pulls
`jsonwebtoken` + `reqwest`; the default `collab` build adds zero new
deps).

```toml
[collab.auth]
methods = ["oidc", "agent-token"]

[collab.auth.oidc]
issuer                 = "https://accounts.google.com"
client_id              = "<your client id>"
client_secret_file     = "~/.config/zetl/oidc-secret"   # path — never inlined
user_id_claim          = "email"                          # default
auto_provision         = true                             # opt-in
provision_domain_allow = ["example.com"]                  # required if auto-provision
```

Standard OIDC authorization code flow with PKCE: `/auth/oidc/login`
redirects to the IdP; `/auth/oidc/callback` validates the ID token
(signature against the IdP's JWKS, plus `iss`, `aud`, `exp`, `nonce`),
extracts `user_id_claim`, provisions if allowed (Reader role only,
domain-gated), and mints a normal session cookie.

`state` and `nonce` are single-use; the PKCE verifier is fresh per
login; the post-login redirect target is restricted to same-origin
local paths (no `?next=https://evil.example.com` open redirects).

## Combining methods

A vault can list multiple methods in any order:

```toml
methods = ["passkey", "agent-token", "capability-url"]
```

Each request walks the chain once; the first non-abstaining method
wins. There's no implicit fallback and no negotiation — what you list
is what you get, in the order you list it.

```toml
# Mixed example: SSO for staff, agent tokens for automation,
# capability URLs for outside reviewers.
methods = ["oidc", "agent-token", "capability-url"]
```

## Auto-provisioning

`proxy-header` and `oidc` can present a principal with no pre-existing
`UserProfile`. The `auto_provision` flag (opt-in, default `false`)
admits them at the **Reader role only** — never higher. Elevation to
Editor or Admin is always an explicit operator action (edit
`.zetl/collab/access.spl`). For OIDC, the identity-claim domain must
match `provision_domain_allow` — `auto_provision = true` with an empty
allowlist denies every unknown principal.

Capability-URL principals are **pseudonymous** — they never go through
auto-provisioning and never create a `UserProfile`.

## Observability

- **Per-decision log** (stderr): `[zetl] auth: method=<id>
  outcome=<authenticated|abstained|rejected> identity=<handle>
  [cause=<category>]`. The `cause` field is operator-channel only and
  is never visible to the user.
- **Audit log**: `.zetl/collab/auth-audit.log` (mode 0600, append-only)
  records every authentication success and reject with an ISO 8601
  timestamp.
- **Startup line**: lists the assembled chain + compiled features.

No log or audit line ever contains a password, agent token, OIDC ID
token, PKCE verifier, or `?cap=` token. The primitives that write logs
are typed so secrets cannot be passed through them by mistake.

## Migration from pre-SPEC-041

If your vault has no `[collab.auth]` block, nothing changes — passkey +
agent-token continue to work exactly as before. You can add new methods
incrementally:

1. Add `[collab.auth.proxy_header]` + run `--trust-proxy`.
2. `zetl collab passwd add` + set `methods = ["password", "agent-token"]`.
3. Build with `--features collab-oidc` + configure `[collab.auth.oidc]`.
4. Mint capability URLs ad hoc with `zetl collab share mint`.

The principal extension shape is the same for every method; existing
[[Access Control|SPL policy]], extractors, and the admin gate don't
change.
