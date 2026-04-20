# Capability-URL Security Model

This document is the operator-facing security reference for `zetl build
--capability`. It explains the threat model, the trust assumptions, the
deployment modes, what capability mode is *not* intended to defend, and
the quantitative bounds operators can rely on. It is derived from
SPEC-034 §1, §11, and §12; the spec is the normative source of truth,
this document is the long-form discussion.

If you are choosing between `zetl serve`, a reverse proxy with SSO, and
`zetl build --capability`, read §3 first.

## 1. What Capability Mode Is

Capability mode is a **build-time ACL**. `zetl build --capability`
encrypts every published page with [`age`][age] against the current
cohort's recipient list and signs the ciphertext with the operator's
Ed25519 vault-signing key. The deploy target is an ordinary static host
(S3, Cloudflare Pages, GitHub Pages, netlify, …). No server-side code
runs. Readers decrypt in-browser with either:

1. **Delegated-URL mode (default).** A per-grant X25519 private key is
   distributed to the reader in the URL fragment (`#k=…`). On first
   visit the shim binds that key to a WebAuthn passkey via
   Trust-on-First-Use (TOFU) and stores it — wrapped by the passkey's
   PRF output — in IndexedDB. Thereafter the passkey is the durable
   credential.
2. **Hardened mode (opt-in).** The reader self-enrols at a static
   `/enroll.html?cohort=<id>`. Their X25519 identity is derived from a
   **cohort-scoped** WebAuthn PRF output. The URL carries no
   cryptographic material. The reader sends their public key to the
   operator out-of-band before the operator grants access.

Both modes use the same ciphertext format and the same signature layer.
Ed25519 verification is mandatory and happens **before** decryption.

[age]: https://age-encryption.org

## 2. Do NOT Use Capability Mode For

Capability mode is a wiki-grade confidentiality layer. **Do not use it
for the following content classes.** The list is deliberately concrete;
if your use case resembles any entry, stop and pick a different
architecture.

| Don't use capability mode for                                                         | Why                                                                                                                                                                                                                    |
| ------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Regulated PII under GDPR Art. 9, HIPAA ePHI, or equivalent**                         | You cannot evidence access (no per-user read audit), cannot enforce real-time revocation, and cannot guarantee deletion on the reader's device. Regulators expect server-side enforcement.                              |
| **Cardholder data in PCI-DSS scope**                                                   | PCI-DSS requires per-user accountability, logged access, and key-management procedures that a static host cannot satisfy. The vault-signing key is a single SPOF; PCI expects key ceremony and HSM-backed signing.      |
| **Data subject to export controls (EAR/ITAR)**                                         | Revocation is rebuild-plus-cache-TTL; an exfiltrated invite URL can grant access across national borders before the next rotation.                                                                                      |
| **Content with legal hold or litigation-relevance**                                    | Readers retain decrypted content on their device. You cannot compel deletion. Access logs are CDN-provider-dependent and do not identify *which reader* read the content.                                              |
| **Anything requiring forward secrecy**                                                 | Revoked readers retain decryption capability for every ciphertext they downloaded pre-revocation. See NFR-3414 and §5 below.                                                                                            |
| **Anonymous whistleblowing / source protection**                                       | Membership of a cohort is not hidden from someone with operator-side access to `grants.toml`. Padding defeats outsiders, not insiders (§4).                                                                             |
| **Cryptocurrency wallet recovery material, bearer credentials, or long-term secrets** | Harvest-now-decrypt-later against classical X25519 is a real 2035+ concern; there is no PQ path in v1.                                                                                                                   |
| **Content you cannot afford to sign once with a long-lived key**                       | The vault-signing key is operator-held and rotation requires a coordinated CDN cache invalidation. Operators unwilling to accept key-rotation downtime must not use capability mode.                                    |
| **Content mixed with untrusted contributions**                                         | A malicious PR can inject content that ciphers and signs exactly like legitimate content. `zetl cap audit-diff` + the sanitiser (REQ-3421, REQ-3424) raise the bar, but a compromised contributor who also controls CI can defeat it. |

If any of the above applies: use `zetl serve` (server-side ACL, real
revocation, per-user audit) or a reverse-proxy-authenticated deployment.
Capability mode is for content whose **loss** is recoverable and whose
**leakage** is bounded by the sensitivity of things like internal
engineering wikis, member-only community notes, or course material.

## 3. Choosing a Deployment Mode

Four common deployment shapes address the "how do I restrict who reads
this wiki" question. They are not substitutes for each other.

| Dimension                                 | Delegated-URL (default)                                    | Hardened (opt-in)                                        | `zetl serve` (SPEC-020)                           | Reverse-proxy auth (e.g. oauth2-proxy, Cloudflare Access, Tailscale) |
| ----------------------------------------- | ---------------------------------------------------------- | -------------------------------------------------------- | ------------------------------------------------- | -------------------------------------------------------------------- |
| **Server-side component**                 | None — CDN / static host only                              | None — CDN / static host only                            | `zetl serve` (process on a host)                  | Proxy + IdP                                                          |
| **Identity primitive**                    | Per-grant X25519 + TOFU-pinned passkey                     | Cohort-scoped WebAuthn-PRF-derived X25519                | SPEC-020 ACL (session cookie / token)             | IdP (OIDC, SAML, …)                                                  |
| **URL carries secret?**                   | Yes (`#k=…` fragment)                                      | No                                                       | No                                                | No                                                                   |
| **Invite medium**                          | Operator-generated URL                                     | Pubkey handoff out-of-band + enrolment page              | Operator adds user to ACL                         | IdP user-add                                                         |
| **Per-user audit of reads**                | No                                                         | No                                                       | Yes (server log)                                  | Yes (proxy log)                                                      |
| **Real-time revocation**                   | No (≤ rebuild + `max-age`; NFR-3409)                       | No (≤ rebuild + `max-age`; NFR-3409)                     | Yes                                               | Yes                                                                  |
| **Forward secrecy**                        | No                                                         | No                                                       | Effectively yes (content never leaves server)     | Effectively yes                                                      |
| **CDN-substitution resistance**            | Ed25519 signature verification (REQ-3427)                  | Ed25519 signature verification (REQ-3427)                | TLS + origin trust                                | TLS + proxy trust                                                    |
| **Works when your server is down**         | Yes — CDN-static                                           | Yes — CDN-static                                         | No                                                | No (proxy depends on upstream)                                       |
| **Hide cohort membership from outsiders?** | Yes (per-grant keypair, X25519 padding to tier)            | Yes (per-cohort PRF salt, X25519 padding to tier)        | N/A — outsiders never see ciphertexts             | N/A                                                                  |
| **Hide cohort membership from insiders?**  | No (`grants.toml` maps names → grants)                     | No (operator holds pubkey ↔ reader map)                  | Depends on `zetl serve` logging                   | Depends on IdP                                                       |
| **Leak surface if invite URL escapes**     | TOFU-window exposure — see §5                              | None — URL has no secret                                 | Session cookie exposure                           | OIDC token exposure                                                  |
| **Operator machine compromise blast**      | Catastrophic (signing key + ZETL_CAP_SECRET)                | Catastrophic (same)                                      | Catastrophic (keys + runtime state)               | IdP-dependent                                                        |
| **Works for >1000-reader cohorts**         | Yes, but count-tier padding grows                           | Yes                                                      | Yes                                               | Yes                                                                  |
| **Post-quantum posture**                   | Classical only (X25519/Ed25519)                             | Classical only                                           | Depends on deployment                             | IdP-dependent                                                        |
| **Operator cost**                           | Zero-ops                                                     | Zero-ops (plus out-of-band pubkey collection)           | One process, one DB                                 | Proxy + IdP subscription                                             |

**Use delegated-URL** when: inviting people is the friction point
("send them a link"), readers are non-technical, and the cohort is
trusted to use org channels (direct message, email without link
rewriting) for invites.

**Use hardened** when: your threat model includes URL-harvesting
adversaries (Safe Browsing, link preview bots, browser sync) and you
can accept the one-off friction of asking readers to paste a pubkey
out-of-band.

**Use `zetl serve`** when: you need per-user audit, real-time
revocation, or forward secrecy.

**Use reverse-proxy auth** when: you already run an IdP; the wiki is
one of many internal apps; zero-ops static hosting is not a
differentiator for you.

Delegated-URL and hardened can be mixed **per cohort** in the same
vault. Picking a mode is not a vault-wide decision.

## 4. Threat Model (Long Form)

SPEC-034 §11.1 defines seven adversaries. This section expands on each.

### Adversary Roster

| # | Adversary                       | Concrete example                                                                                |
| - | ------------------------------- | ----------------------------------------------------------------------------------------------- |
| A1 | **Passive web observer**        | Anyone with URL-harvesting access: Safe Browsing servers, link unfurl bots, browser sync sinks. |
| A2 | **Ciphertext holder**           | Anyone who can fetch `/c/<path-cap>/<slug>.html`. Includes search engines, CDN employees.       |
| A3 | **CDN-compromised**             | Attacker who substitutes bytes at the CDN layer: stolen CDN credentials, rogue CDN employee.    |
| A4 | **Authenticator thief**         | Physical theft of an unlocked device with a paired authenticator; passkey takeover.             |
| A5 | **Malicious contributor**       | A PR author who tries to inject XSS, exfiltrating CSS, or otherwise hostile markdown.           |
| A6 | **Compromised CI**              | Attacker controlling the build environment: leaked `ZETL_CAP_SECRET`, leaked signing key.      |
| A7 | **Signing-key compromiser**     | Attacker who extracts `ZETL_CAP_SIGNING_KEY` from CI, ops machine, or backup.                  |

### Per-Attack Mitigation Surface

The attack/mitigation matrix in SPEC-034 §11.1 is normative. Notable
mitigations and their honest limits:

- **CDN substitution (A3).** Mitigated by Ed25519 signature
  verification: the shim refuses to decrypt or render any ciphertext
  whose signature does not verify against the pubkey embedded in the
  shim bundle (REQ-3427, CON-3411, NFR-3413). The *signing layer is the
  primary defence against CDN substitution*; confidentiality (`age`)
  alone does not bind ciphertext to the operator.

- **URL harvest (A1, delegated-URL).** *Acknowledged residual.*
  `history.replaceState` is JS-driven and runs after the browser has
  already processed the URL for history, sync, extensions, and Safe
  Browsing. The fragment may be captured before the scrub. Mitigations
  are **bounding, not eliminating**:

  - `NFR-3412` caps the invite-URL usability window (default 7 days,
    minimum 60 s).
  - `zetl cap finalise` marks a grant as post-onboarding; operators
    may (out-of-band) retire the URL.
  - Opt-in **split-key mode** (REQ-3430) sends the URL and a second
    factor via separate channels; an attacker needs both to TOFU-bind.

  **Enumerated leak channels** (non-exhaustive; operators should treat
  any URL-logging surface as in scope):

  | Layer           | Specific channels                                                                 |
  | --------------- | --------------------------------------------------------------------------------- |
  | Network         | Google Safe Browsing (Chrome ESB), Microsoft SmartScreen (Edge), translation APIs |
  | Browser-local   | History DB, Chrome Sync, iCloud Tabs, Firefox Sync, tab recovery, session restore |
  | Extension-level | Any extension with `tabs` or `webNavigation` permission sees `location.href`      |
  | OS-level        | Screen readers reading URL aloud; iOS Universal Clipboard; macOS Handoff          |
  | Third-party     | Password managers (autofill URL logging); link preview bots; URL shorteners       |

  For content where these channels are unacceptable, use **hardened
  mode**. It is the only URL-harvest-immune path.

- **Cross-cohort pubkey linkage (A1, A2, hardened mode).** Mitigated by
  per-cohort PRF salt (REQ-3414). A reader in N hardened cohorts enrols
  N times and produces N distinct pubkeys.

- **Cross-cohort pubkey linkage (A1, A2, delegated-URL mode).** Not an
  issue: each grant produces a fresh keypair, so the per-ciphertext
  recipient entries are already unlinkable across cohorts for the same
  reader. (An operator with `grants.toml` access sees the mapping;
  that's the next bullet.)

- **Insider linkage via operator data.** Operators with access to
  `grants.toml` can map names → grants → cohort membership. Inherent
  to the design. Operators for whom this is unacceptable must split
  operational roles (grant-issuer vs. content-author) and/or
  `.gitignore` `grants.toml` (REQ-3423 permits both).

- **Recipient-count inference.** See §4 below — quantitative.

- **Authenticator loss (A4).** Standard WebAuthn recovery applies; the
  spec adds no controls. Operators should document "revoke and re-enrol
  on a new authenticator" in their onboarding runbook.

- **Malicious PR (A5).** Mitigated in layers:

  1. `ammonia`-based HTML sanitiser with an OWASP-aligned allowlist
     (REQ-3421; config at `tools/sanitiser-config.toml`).
  2. Content Security Policy `default-src 'none'; script-src 'self';
     trusted-types 'none'; …` (CON-3410).
  3. `zetl cap audit-diff <old-ref> <new-ref>` PR gate against a
     malicious-content corpus (REQ-3424).

  None of these stops a contributor who *also* controls CI and can push
  a shim with the sanitiser disabled. That is adversary A6 and is
  mitigated organisationally (branch protection, review).

- **Signing-key compromise (A7).** Rotate via `zetl cap
  rotate-signing-key`; rebuild all pages; deploy; invalidate the CDN
  cache for `/assets/shim.js`. The rotation window is the exposure
  window — design your incident-response SLA accordingly.

- **Outbound Referer leak of the path-cap (REQ-3413, OBS-3407).** When
  a reader clicks an external link, the browser would by default send
  the current URL (including `/c/<path-cap>/<slug>.html`) to the
  destination site's `Referer` header. That hands the path-cap — a
  cohort-scoped secret — to a third party. Two defences ship in v1:

  1. Every external `<a>` is rewritten during build to carry
     `rel="noopener noreferrer"`. Internal links (root-relative,
     relative, or anchor-only) are left byte-identical so same-site
     `Referer` remains available for operator analytics.
  2. The capability HTML shell carries `<meta name="referrer"
     content="no-referrer">` as the document-wide default, honoured
     by every modern browser even for links the rewrite missed.

  Operators who need the browser's default referrer behaviour on
  external clicks (e.g. for a trusted analytics partner that has been
  briefed on the exposure) can opt out with `[access] rel_noreferrer
  = false`. The `<meta>` default stays in the shell either way, so
  opting out weakens path-cap privacy without fully removing the
  document-level defence. The trade-off is explicit: **disabling
  `rel_noreferrer` reduces path-cap privacy — the destination site
  and any on-path observer will see the path-cap in their `Referer`
  logs.** `make ref-leak-test` is the CI canary.

- **ZETL_CAP_SECRET compromise (A6).** Catastrophic. `zetl cap
  emergency-shutdown` prints the operator checklist (DNS, CDN purge,
  secret rotation, reader notification).

### Non-Goals (normative — SPEC-034 §1.3)

The following are **explicit non-goals**. Reading them as gaps is a
misreading: the spec will not attempt them, and operators who need them
should not use capability mode.

- Per-user audit of reads.
- Real-time revocation (the latency bound is `rebuild + max-age`).
- Per-user visibility overrides.
- Write access.
- Forward secrecy.
- Post-quantum security (a v2 may adopt `age-plugin-pq` or similar;
  v1 is classical X25519/Ed25519 only).
- Concealment of cohort-membership size from insiders.
- Concealment of multi-cohort membership in delegated-URL mode from
  operators with `grants.toml` access.
- Replacing OIDC / SSO / proxy auth for general apps.

## 5. Quantitative Bounds — Padding, Brute-Force, and Observable Tiers

This section pins down what NFR-3410 ("recipient-count observable
tier") guarantees and what it does not. The numbers here are the
honest ones; the spec's §11.2 acknowledges this and it is reproduced
here long-form for operator reference.

### Padding Scheme (REQ-3422 / ADR-3413)

Each cohort's `age` ciphertext is padded so the **observable recipient
count** from the ciphertext header is always one of:

```
TIER ∈ { 10, 30, 100, 300, 1000 }
```

The smallest tier ≥ (real recipient count) is chosen. Padding entries
are **ephemeral X25519 public keys** generated by
`crypto.getRandomValues`-seeded keypair generation; the corresponding
**private key is discarded before the pubkey is written to the
ciphertext**. No party — not the operator, not the CI, not anyone — can
decrypt the padding entries.

### Outsider Bound (Passive Web / Ciphertext Holder — A1, A2)

**Claim.** An adversary who does not hold any cohort recipient's
private key learns only the tier. They cannot distinguish real
recipient entries from padding entries.

**Why.** Each real recipient entry and each padding entry is a
uniformly-random X25519 public key. Distinguishing a real pubkey (which
has a corresponding private key held by a reader) from a padding pubkey
(whose private key was discarded) without access to either private key
reduces to the **Decisional Diffie-Hellman (DDH) problem on
Curve25519**, which is hard at ~2¹²⁸ classical work (RFC 7748).

Concretely, to "identify" any single entry as real, the adversary
must:

1. Guess a 32-byte candidate private key such that `X25519(candidate,
   basepoint) = observed_pubkey`. That is the discrete-logarithm
   problem on Curve25519 — ~2¹²⁸ work by Pollard's rho or equivalent.
2. Or, having guessed, they must also verify the private key decrypts
   the `age` recipient stanza (another AEAD integrity check). This
   doesn't reduce work; it just means guessing cannot be
   "verified-for-free" against the pubkey alone.

Even with specialised hardware, 2¹²⁸ is not reachable. The GPU-hours
cost to brute-force one X25519 key is approximately:

```
    2¹²⁸ / (≈10⁹ ops/sec on a top-tier GPU)
  = 3.4 × 10²⁸ GPU-seconds
  = 1.1 × 10²¹ GPU-years
```

— which is about 10¹¹ × the age of the universe. This holds regardless
of tier size; padding does not make brute-force easier or harder than
the underlying primitive.

**Therefore:** outsiders learn `count ∈ { 10, 30, 100, 300, 1000 }`,
nothing more.

### Insider Bound (Cohort Member — holds ≥ 1 private key)

**Claim.** A cohort member can identify **their own** recipient entry
and subtract it, learning `real_count ∈ { 0, …, tier − 1 }` (with the
tiering constraint implying `real_count > previous_tier`). They
**cannot** identify any *other* real recipient's entry without also
holding that recipient's private key.

**Why they can identify their own entry.** For each recipient entry in
the age header, the member can attempt `age`-decryption with their
own private key. The entry that decrypts successfully is their own.
This is how `age` works — and it's the right thing; an entry that could
not be decrypted by its intended recipient would be useless.

**Why they cannot identify other real entries.** For each other entry,
they have the same DDH hardness as an outsider. They have no reader's
private key but their own.

**Practical consequence.** For a cohort at tier=100 with 73 real
recipients:

- Outsider: knows `count = 100` (just the tier).
- Insider (cohort member): knows `count ∈ { 73, …, 99 }` is possible,
  but has a tighter bound if they also know, say, the cohort's
  approximate size from out-of-band channels ("the engineering team
  has ~70 people").
- Insider who knows the exact member list: still cannot *confirm* any
  specific other member is in the cohort (they might have been
  removed). The cryptographic bound does not reveal per-person
  membership.

**What insiders cannot do, ever.** Identify a *specific other reader's*
entry, even if they can guess at the full membership list. Linking
"Alice's entry is this specific X25519 pubkey" requires Alice's private
key; the insider has only their own. This is the core DDH argument.

### Tiering Honesty — What Padding Does NOT Buy You

The padding tier is a *coarse* observable. Operators should not read
into it:

- **Growth is visible at tier boundaries.** Crossing from 10 → 30 real
  readers causes the tier to jump. An observer watching successive
  rebuilds sees "small cohort" → "medium cohort." Tier transitions are
  load-bearing signalling events; if hiding cohort growth is important,
  provision a higher tier from the start.
- **Per-page recipient lists are identical across a cohort's pages.**
  All pages in a cohort share the same recipients (minus padding
  resamples). An adversary correlating entries across pages cannot
  learn more about *who* — they can confirm "these pages are from the
  same cohort," but that is also knowable from path-caps and envelope
  headers.
- **Tier choice is deterministic from real count.** If your ciphertexts
  are built reproducibly (REQ-3420), tier choice is observable, not
  private. Don't rely on it being a secret.

### Signing-Key Brute-Force Bound

Ed25519 vault-signing key: ~2¹²⁸ classical security against signature
forgery. An adversary who cannot obtain the private key cannot forge a
signature that the shim will accept. The shim bundle is
**SRI-pinned** (`integrity="sha384-…"`), so tampering with the embedded
pubkey in transit also fails — the browser refuses to execute a shim
whose SRI hash does not match.

The interesting attack here is not brute-forcing the key; it's
**stealing** it from the operator's machine, CI, or backup. That's
adversary A7 and is an operational-security problem, not a
cryptographic one. `zetl cap rotate-signing-key` is the recourse.

### Path-Cap Entropy (NFR-3401)

Default 64 bits. Minimum 48, maximum 128. Path-caps are stable across
rotations (they identify the URL, not the content key). Brute-force
URL enumeration against a 64-bit path-cap is infeasible for public
CDNs, but operators concerned about liveness enumeration should set
`path_cap_bits = 96` or higher.

## 6. Content-Authenticity Trust Model

Capability mode decouples **confidentiality** (who can read) from
**authenticity** (who wrote). Confidentiality rides on `age` recipient
encryption; authenticity rides on Ed25519 signing (REQ-3427, CON-3411,
ADR-3412). The two layers exist because they defend different attacks.

### Why Signing Is Necessary

`age` AEAD only confirms: *"someone who knew a cohort recipient key
encrypted this."* In capability mode, the "cohort recipient key" is
public — anyone with the recipients file can produce a valid `age`
ciphertext. This means a **CDN adversary (A3) can substitute
attacker-written content and have the shim render it**, because the
shim's decryption layer has no way to tell legitimate ciphertext from
attacker-produced ciphertext.

Ed25519 signing closes this gap. The shim refuses to render any
ciphertext whose signature does not verify against the operator's
vault-signing pubkey.

### Trust Root

The vault-signing **public key** is:

1. Generated by `zetl cap genkey` on the operator's machine.
2. Embedded into the shim bundle (`dist/assets/shim.js`) at build time.
3. Shipped to readers alongside every page.
4. Pinned via **Subresource Integrity** (SRI):

   ```html
   <script src="/assets/shim.js"
           integrity="sha384-<hash>"
           crossorigin="anonymous"></script>
   ```

The vault-signing **private key** (`ZETL_CAP_SIGNING_KEY`):

- NEVER crosses into browser code (NFR-3408).
- Is provided to the build at CI time via env var.
- Operator-stored (password manager, or keychain — see §14 OQ-1).

**Trust flow.**

```
  operator generates keypair (zetl cap genkey)
       │
       ├──► private key  ──► env var at CI ──► build signs ciphertexts
       │
       └──► public key   ──► embedded in shim.js (SRI-hashed)
                               │
                               ▼
                        browser fetches shim
                        SRI verifies shim integrity
                        shim verifies ciphertext signatures
                        ONLY then decrypts
```

A reader's trust in any page they render reduces to their trust that:

1. Their browser correctly enforces SRI (universal in modern browsers).
2. The shim bundle's embedded pubkey is the operator's real pubkey.
3. The operator has not lost the signing private key.

(1) is a browser-platform property. (2) reduces to "the first shim the
reader loaded was the operator's real shim," which is a TOFU assumption
for shim pubkey pinning (identical in structure to the TOFU on
passkey binding; same honesty caveats apply — see §4 on URL-harvest
residual).

### Signing-Key Rotation Hazards

`zetl cap rotate-signing-key` triggers:

1. Generate new Ed25519 keypair.
2. Rebuild every page with new signatures.
3. Rebuild the shim bundle with the new embedded pubkey (new SRI hash).
4. Deploy.
5. **Cache-invalidate `/assets/shim.js` at the CDN.**

Step 5 is load-bearing. If a reader's browser holds the **old** shim
while the CDN serves **new** signed pages, every page will be reported
as "signature verification failed" — because the old shim's embedded
pubkey cannot verify signatures produced by the new private key.

The honest version of the hazard: there is a **rotation transition
window** during which a subset of readers may be locked out, depending
on their browser's cache state. Operators should:

- Schedule rotation during low-traffic hours.
- Pre-invalidate `/assets/shim.js` before deploying new pages.
- Communicate to readers that a hard-refresh (Ctrl/Cmd-Shift-R) may
  be needed.
- Monitor `OBS-3413` (signature-failure counter) for a spike
  post-rotation.

A more elaborate dual-pubkey-slot scheme (new shim trusts both old and
new pubkey during transition) is identified in SPEC-034 §14 OQ-5 and
the Tier 2 review S2-05 as a v2 consideration. It is not in v1.

### Signing-Key Loss

If the operator loses `ZETL_CAP_SIGNING_KEY` without having rotated,
all previously-signed content remains readable (signature verification
still passes against the pinned pubkey), but **no new content can be
signed**. Re-issuing the signing key is equivalent to the rotation
procedure above.

An M-of-N Shamir-split recovery key is noted as a future option (SPEC-034
§14 OQ-2). v1 has no such recovery; operators should back up
`ZETL_CAP_SIGNING_KEY` alongside `ZETL_CAP_SECRET`.

## 7. Prior Art and Empirical Basis

Capability mode is not novel cryptography. It is a deliberate composition
of standard primitives. The table below — reproduced from SPEC-034 §1.4
with expanded citations — is the authoritative dependency map; the
citations should be consulted by reviewers, and the primitives treated
as unmodified.

| Prior art                                          | Contribution                                                                                               | Relationship here                                                                     |
| -------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------- |
| [`age` / typage (Valsorda)][age]                   | Modern AEAD; age v1 wire format; recipient-based encryption; browser runtime via typage.                   | Consumed unmodified. `age` v1 ciphertexts are readable by the reference impl.         |
| [WebAuthn Level 3 PRF extension][webauthn-prf]     | Hardware-backed deterministic pseudorandom output bound to a passkey.                                      | Direct dependency for both TOFU-wrap and hardened-mode identity derivation.           |
| [Filippo — "Encrypting Files with Passkeys and age"][filippo-passkeys] | PRF→X25519 composition; `age-encryption.org/fido2prf` standard salt prefix.                    | Extended with per-cohort salt disambiguator (REQ-3414).                               |
| [Ed25519 (RFC 8032)][rfc8032]                      | Modern signature primitive; deterministic; small keys and sigs.                                            | Used as the vault-signing key (REQ-3427).                                             |
| [X25519 (RFC 7748)][rfc7748]                       | Elliptic-curve Diffie-Hellman; DDH hardness → padding indistinguishability.                                | Underlies `age` recipient encryption and padding (REQ-3422).                          |
| [HKDF (RFC 5869)][rfc5869]                         | Extract-and-expand KDF.                                                                                     | Used for path-cap derivation and TOFU-wrap key derivation.                            |
| PrivateBin; Bitwarden Send; PageCrypt              | URL-fragment-as-decryption-key pattern in consumer-facing tools.                                           | Delegated-URL mode is in the same design family; we add TOFU, signing, honest disclosure. |
| SSH trust-on-first-use (pinning on first encounter) | The canonical TOFU pattern.                                                                                | Direct inspiration for passkey-binding on first decrypt.                              |
| [Pulse Security — "Sensitive data in URLs"][pulse] | Empirical analysis of URL-harvester surfaces (Safe Browsing, SmartScreen, unfurl bots).                    | Motivates both the delegated-URL residual disclosure and split-key mode (REQ-3430).   |
| [`ammonia` (Rust)][ammonia] / [DOMPurify (JS)][dompurify] | Battle-tested HTML sanitisers with OWASP-aligned defaults.                                           | `ammonia` is the build-side sanitiser (REQ-3421); DOMPurify is the reference shim-side config. |
| [Subresource Integrity (W3C SRI)][sri]             | Browser-verified script integrity against the hash declared in the tag.                                    | Required on shim loader (REQ-3421) to pin the vault-signing pubkey.                   |
| [CSP Level 3 — Trusted Types][trusted-types]       | Browser-enforced DOM-XSS mitigation via type-tagged sinks.                                                  | `require-trusted-types-for 'script'; trusted-types 'none'` (CON-3410).                |

[webauthn-prf]: https://www.w3.org/TR/webauthn-3/#prf-extension
[filippo-passkeys]: https://words.filippo.io/passkey-encryption/
[rfc8032]: https://www.rfc-editor.org/rfc/rfc8032
[rfc7748]: https://www.rfc-editor.org/rfc/rfc7748
[rfc5869]: https://www.rfc-editor.org/rfc/rfc5869
[pulse]: https://pulsesecurity.co.nz/advisories/sensitive-data-in-urls
[ammonia]: https://github.com/rust-ammonia/ammonia
[dompurify]: https://github.com/cure53/DOMPurify
[sri]: https://www.w3.org/TR/SRI/
[trusted-types]: https://www.w3.org/TR/trusted-types/

## 8. Acknowledged Residual Exposures

Reproduced from SPEC-034 §11.2 for operator convenience. The spec is
the normative source; if this list ever drifts from §11.2, trust the
spec.

- **Fragment leak during TOFU window.** `history.replaceState` is
  best-effort (§4 above).
- **Insider recipient-count inference.** Cohort member learns `count
  ∈ { 0, …, tier − 1 }`. Not hidden.
- **Operator-side multi-cohort linkage (delegated-URL).** Operator
  with `grants.toml` sees the mapping. Inherent.
- **Forward secrecy not provided.** Revoked readers retain past
  decryption. Use `zetl serve` if this matters.
- **URL forwarding.** Invite URL is a bearer capability; anyone with
  it during the TOFU window can bind.
- **Path-cap probing.** Liveness enumeration is observable. Bounded by
  cohort rotation + larger `path_cap_bits`.
- **CDN access logs.** Operator-configurable. Many CDNs log URLs +
  timestamps + source IPs by default.
- **Traffic-size analysis.** Plaintext page size leaks via ciphertext
  size. `age` does not pad plaintexts.
- **Authenticator loss-of-custody.** Standard WebAuthn; the spec adds
  no controls.
- **Timing side-channel in decrypt.** Page-size-dependent; not
  defended.
- **Link shorteners / preview bots.** REQ-3410 prints an operator
  warning on every `zetl cap invite`. Operator obligation.
- **Post-quantum harvest-now-decrypt-later.** Classical X25519 only.
  No v1 PQ path. Consider this before publishing very-long-shelf-life
  content.

## 9. Incident-Response Playbooks

Reproduced from SPEC-034 §11.3 for operator convenience. `zetl cap
emergency-shutdown` prints a live checklist at invocation time; the
table below is a reference.

| Incident                                 | Response                                                                                                                             |
| ---------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| Reader leaves                             | `zetl cap revoke <grant-id>` → rebuild → deploy. Latency ≤ NFR-3409 (rebuild + `max-age`, default ≤ 1 h).                             |
| Authenticator compromised                 | Rotate affected cohort (`zetl cap rotate --cohort <id>`); redistribute entry URLs.                                                   |
| Invite URL leaked pre-TOFU, within expiry | `zetl cap revoke <grant-id>` + re-invite on a fresh URL. Old URL becomes inert on next rebuild.                                      |
| Invite URL leaked post-expiry             | Already inert. No action required beyond monitoring for anomalous access patterns.                                                   |
| Signing key compromised                   | `zetl cap rotate-signing-key` → rebuild all pages → deploy → invalidate `/assets/shim.js` cache at CDN. Monitor OBS-3413 post-cutover. |
| `ZETL_CAP_SECRET` compromised             | `zetl cap genkey` → rotate ALL cohorts → rebuild → re-issue all URLs → readers re-TOFU per device. Effectively a full re-onboarding.  |
| Malicious PR landed                       | Revert on main → rebuild → `zetl cap audit-diff` across the exposure window → consider sanitiser allowlist tightening.               |
| Emergency shutdown                        | `zetl cap emergency-shutdown` → follow printed checklist (DNS removal, CDN purge, secret rotation, reader notification).             |

## 10. Cross-References

- `specs/SPEC-034.md` — normative source (§1, §11, §12 in particular).
- `specs/SPEC-020.md` — `zetl serve` runtime ACL (the forward-secrecy
  alternative).
- `docs/hook-security.md` — unrelated but analogous: pipeline-hook
  trust model for SPEC-032.
- `tools/sanitiser-config.toml` — normative HTML denylist (REQ-3421).
- `.hence/reviews/cap-tier1-crypto-2026-04-20.md` — Tier 1 crypto review.
- `.hence/reviews/cap-tier2-2026-04-20.md` — Tier 2 fresh-context review
  (whose S1/S2 findings should be consulted before trusting any
  quantitative claim in this document to production-grade).

---

*If any statement here conflicts with SPEC-034, the spec governs. File
a PR updating this document; do not depart from the spec.*
