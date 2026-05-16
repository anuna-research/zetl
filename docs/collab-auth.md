# Collab-mode authentication — operator guide

[SPEC-041](../specs/SPEC-041-pluggable-collab-auth.md) makes `zetl serve
--collab` authentication **pluggable**. A vault picks one or more methods
in `.zetl/config.toml`; zetl walks them in declared order on every request,
first-match-wins. The default is the pre-SPEC-041 behaviour (`passkey` +
`agent-token`) so an upgrade with no config change is a no-op.

This guide is per-method operator material. The threat model is in a
separate document: [`research/SPEC-041-threat-model.md`](../research/SPEC-041-threat-model.md).

## The chain mental model

```toml
[collab.auth]
methods = ["oidc", "agent-token"]   # ordered precedence chain
```

When a request arrives, `auth_resolve` walks `methods` once, first-match-wins:

* **Authenticated** — the first method that recognises a credential wins;
  every later method is skipped.
* **Abstain** — the method doesn't see a credential it can act on (no
  `?cap=` for `capability-url`, no Bearer header for `agent-token`, no
  cookie for `passkey`); the chain proceeds.
* **Reject** — the method hard-rejects (a present-but-invalid credential
  the operator wants to fail closed on); the chain terminates with 401.

`auth_resolve` stashes the resulting `Principal` in the request extensions
exactly once. Every downstream gate (`collab_gate`, `admin_gate`,
`csrf_guard`) and extractor reads it from there — there is no second
header parse, no fallback resolution.

A startup line lists the assembled chain and compiled auth features:

```
[zetl] collab auth: methods=[passkey, agent-token] features=[collab]
```

If something is wrong — unknown method name, missing sub-table for a
method that needs one, `proxy-header` without `--trust-proxy`, `oidc`
without the `collab-oidc` feature — startup fails with a named error
naming the offending key and the corrective action.

## Methods

### `passkey` (default)

WebAuthn passkey login → `SessionStore` session cookie (`zetl_session`).
The default chain. No `[collab.auth.passkey]` sub-table — no config.

### `agent-token` (default)

Ed25519-signed Bearer token bound to a `UserProfile` (the agent-token
format is a bespoke binary blob with its own recogniser in
`src/user/agent_token.rs`; not a JWT). Stateless principal — CSRF guard
is exempt by construction. No sub-table.

### `proxy-header`

For deployments behind an authenticating proxy (oauth2-proxy, Authelia,
Tailscale Serve, Cloudflare Access, an SSO ingress). The proxy
authenticates the user and forwards their identity in a header.

```toml
[collab.auth]
methods = ["proxy-header", "agent-token"]

[collab.auth.proxy_header]
user_header    = "X-Forwarded-User"          # default
peer_allow     = ["127.0.0.1/32", "10.0.0.0/8"]   # required, non-empty
auto_provision = true                         # opt-in (REQ-4111)
```

Run `zetl serve --collab --trust-proxy`. **Both** are required:
`--trust-proxy` AND a peer IP in `peer_allow`. The header is ignored on
any other peer.

**Your proxy MUST strip inbound copies of `user_header`.** zetl's
`peer_allow` is a second-layer defence; the first layer is the proxy
removing client-supplied copies before they reach zetl. Test this with
a direct (non-proxy) request that sets the header — zetl should reject
it.

### `password`

Static argon2id passwords. For small teams without an IdP.

```toml
[collab.auth]
methods = ["password", "agent-token"]
```

Operator manages credentials with `zetl collab passwd`:

```
zetl collab passwd add <user>      # TTY-prompted, never argv
zetl collab passwd list            # user_ids only — no hashes
zetl collab passwd remove <user>
```

Users hit `/auth/password`, submit name + password, get a normal session
cookie. Verification is constant-time + cause-indistinguishable (unknown
user looks like wrong password, in both response shape and timing).
Storage: `.zetl/collab/passwords.json`, mode 0600, argon2id PHC strings
(params embedded so they can be raised without invalidating existing
hashes).

### `capability-url`

The "anyone with the link" path. Mint a signed `?cap=<token>` URL bound
to a scope glob and a role; recipients open the URL and get scoped,
time-bounded, pseudonymous access — no account, no profile.

```toml
[collab.auth]
methods = ["passkey", "capability-url", "agent-token"]

[collab.auth.capability_url]
default_ttl = "7d"     # default
max_ttl     = "90d"    # default upper bound
```

Mint, list, revoke:

```
zetl collab share mint --scope "review/draft-7/**" --role reader \
                       --expires 7d \
                       --site-url https://wiki.example.com
zetl collab share list
zetl collab share revoke <jti>
```

**The URL is a bearer token.** The mint CLI prints a security notice on
stderr — read it. A leaked URL exposes the granted slice (scope + role)
until expiry or revocation; it does **not** expose the rest of the vault.
Capability principals never satisfy `admin_gate` regardless of the
encoded role.

**Sensitivity ceiling.** Use capability URLs for low-to-moderate
sensitivity scopes — public draft reviews, share-with-a-friend, content
that's mostly public anyway. For anything sensitive prefer `oidc` or
`passkey`. The full trade-off is documented in
[Threat Model H](../research/SPEC-041-threat-model.md#h-capability-url-leakage).

zetl sets `Referrer-Policy: no-referrer` on every response to a
`?cap=`-bearing request so the token doesn't leak via the `Referer`
header to outbound links. The token is stripped from the audit log
(`auth-audit.log` records the pseudonymous `cap-<jti-prefix>` handle, not
the URL).

### `oidc`

Build with `--features collab,collab-oidc` (the OIDC code path pulls
`jsonwebtoken` + `reqwest`; the default `collab` build does not).

```toml
[collab.auth]
methods = ["oidc", "agent-token"]

[collab.auth.oidc]
issuer                 = "https://accounts.google.com"
client_id              = "<your client id>"
client_secret_file     = "~/.config/zetl/oidc-secret"   # path, never inline
user_id_claim          = "email"   # default
auto_provision         = true      # opt-in (REQ-4111)
provision_domain_allow = ["example.com"]   # required when auto_provision = true
```

Standard OIDC authorization code flow with PKCE: `/auth/oidc/login`
redirects to the IdP; `/auth/oidc/callback` validates the ID token
(signature against JWKS, `iss`, `aud`, `exp`, `nonce`), extracts
`user_id_claim`, provisions if allowed (Reader role only, domain-gated),
and mints a normal session cookie.

`state` and `nonce` are single-use, the PKCE verifier is fresh per
login, and the client secret is loaded from a file at startup (never
inlined).

## `[collab.auth]` reference

```toml
[collab.auth]
# Ordered precedence chain. Absent block ⇒ ["passkey", "agent-token"].
methods = ["..."]

[collab.auth.proxy_header]
user_header    = "X-Forwarded-User"   # any HTTP header name; default shown
peer_allow     = ["..."]               # required, non-empty CIDR list
auto_provision = false                 # default

[collab.auth.password]
# no fields — store path is fixed (.zetl/collab/passwords.json)

[collab.auth.capability_url]
default_ttl = "7d"
max_ttl     = "90d"

[collab.auth.oidc]
issuer                 = "..."
client_id              = "..."
client_secret_file     = "..."
user_id_claim          = "email"
auto_provision         = false
provision_domain_allow = ["..."]
redirect_uri           = "..."         # optional; defaults to derive from Host
```

`deny_unknown_fields` is enforced inside `[collab.auth]` and every
sub-table — a typo at the schema fails startup naming the offending key.

## Observability

* **Per-decision log line** (stderr): `[zetl] auth: method=<id>
  outcome=<authenticated|abstained|rejected> identity=<handle>
  [cause=<category>]`. `cause` is operator-channel only.
* **Audit log**: `.zetl/collab/auth-audit.log` (mode 0600, append-only)
  records every successful authentication and every reject with an
  ISO 8601 timestamp.
* **Startup line**: lists the assembled chain + compiled features.

No log or audit line ever contains a password, an agent token, an OIDC
ID token, a PKCE verifier, or a `?cap=` token — the primitives that
write logs are typed so secrets cannot be passed through them by
mistake.

## Combining methods

A vault can list multiple methods. The chain order matters:

```toml
methods = ["passkey", "agent-token", "capability-url"]
```

* Members log in via passkey (cookie session, CSRF-guarded).
* Automation uses agent tokens (Bearer header, stateless, CSRF exempt).
* Outside reviewers use capability URLs (scoped + revocable + pseudonymous).

`auth_resolve` walks the chain once; the first non-abstaining method
wins. There is no implicit fallback and no method negotiation — what you
list is what you get, in the order you list it.

## Migration from pre-SPEC-041

If your vault has no `[collab.auth]` block, nothing changes — passkey +
agent-token continue to work exactly as before. You can add new methods
incrementally:

1. Add `[collab.auth.proxy_header]` and run `--trust-proxy`.
2. `zetl collab passwd add` + set `methods = ["password", "agent-token"]`.
3. Build with `--features collab-oidc` + configure `[collab.auth.oidc]`.
4. Mint capability URLs ad hoc with `zetl collab share mint`.

The `Principal` extension shape is identical for every method; gates,
extractors, and SPL policy don't change.
