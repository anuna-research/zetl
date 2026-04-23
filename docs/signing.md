# Vault Signing Key — Operator Reference

This document explains what the **vault-signing key** is in capability
mode, why it exists, how to rotate it, and what to do if it is
compromised or lost. It is the canonical long-form reference for the
operator workflow introduced in v0.4.0 as the resolution to BUG-004
(the CDN-substitution attack identified in the v0.3.0 review).

The normative source is `specs/SPEC-034.md` — in particular REQ-3427
(content signing), ADR-3412 (rationale), CON-3411 (protocol),
REQ-3431 / §11.3 (emergency shutdown), and §11.4 (trust tier).
`docs/capability-security.md` contains the wider security model; this
document zooms in on the key-management procedures alone. If any
statement here conflicts with the spec, the spec governs — file a PR
against this file, do not depart from the spec.

## 1. What the Vault-Signing Key Is

The vault-signing key is an **Ed25519 keypair** held by the operator
of a capability-mode wiki (RFC 8032; used in strict `verify_strict`
form in both build and shim, so only canonical-R/S signatures are
accepted).

- The **private** half (`ztl_CAP_SIGNING_KEY`) signs every encrypted
  page as it is emitted at build time.
- The **public** half (`[vault].signing_pubkey` in `recipients.toml`)
  is embedded into the shim JS bundle at build time and shipped to
  every reader inside `dist/assets/shim.js`.

Both halves are generated once by `ztl cap genkey` — alongside
`ztl_CAP_SECRET` — and displayed on stdout exactly once with storage
instructions. The private half SHALL NOT be committed, logged, or
persisted; it enters the build process only via the
`ztl_CAP_SIGNING_KEY` environment variable (SPEC-034 NFR-3408).

At build time, for each page `p`:

```
ciphertext_p = age_encrypt(p.html, cohort_recipients ∪ padding)
signature_p  = Ed25519.sign(vault_signing_priv_key, ciphertext_p)
envelope_p   = ztl-Schema: v4
               ztl-Cohort-Id: …
               ztl-Cohort-Mode: delegated-url | hardened
               ztl-Slug: …
               ztl-Build-Epoch: …
               ztl-Signature: <base64url-unpadded-64-bytes>
               <blank line>
               <age ciphertext bytes>
```

At read time, the shim refuses to decrypt, derive identity, or prompt
for a WebAuthn credential until the Ed25519 signature has verified
against the embedded pubkey. See CON-3411 for the exact step order and
`docs/capability-security.md` §6 for the trust flow diagram.

## 2. Why It Matters — CDN Substitution

In capability mode, the content is encrypted with `age` against the
cohort's *recipient* public keys. `age` provides **AEAD over the
ciphertext**, not **author authenticity**: anyone who holds a cohort
recipient key can produce a ciphertext that `age` will accept. For
delegated-URL mode, the per-grant X25519 pubkey is bound to the
**URL** — which traverses the same CDN path as the content itself.

That means a **compromised CDN (attacker A3)** — or a mirror, a
malicious edge worker, an MITM on TLS, or anyone who controls a
fraction of the serving path — could substitute ciphertext that was
encrypted under the legitimate cohort recipient list. Without a
signature, the shim has no way to tell operator-produced ciphertext
from attacker-produced ciphertext. The attacker does not need the
reader's private key; they only need to replace `/c/<slug>.html` at
the CDN edge with a ciphertext they authored against the public
recipient list.

Ed25519 content signing closes that gap. The attacker does not hold
`ztl_CAP_SIGNING_KEY`, so they cannot produce a signature that the
shim will accept. The shim refuses to render, and the reader sees:

> This page's signature did not verify — possible tampering; contact
> your wiki operator

This is REQ-3427 / ADR-3412 / CON-3411 in prose. The v0.3.0 review
flagged the gap as **BUG-004**; v0.4.0 ships the fix.

### The Trust Root Is SRI + TOFU on Shim

The shim bundle itself is pinned via **Subresource Integrity**:

```html
<script src="/assets/shim.js"
        integrity="sha384-<hash>"
        crossorigin="anonymous"></script>
```

Since the operator's pubkey is a **string literal** in the emitted
shim IIFE (embedded by esbuild's `define` substitution at build time),
the SHA-384 SRI hash covers every byte of the pubkey. Tampering with
the embedded pubkey in transit invalidates the SRI hash, and the
browser refuses to execute the shim at all.

The residual trust assumption is **TOFU on the shim bundle**: the
first time a reader loaded `/assets/shim.js`, they received the
operator's real shim. This mirrors the TOFU assumption on the
delegated-URL passkey binding; both are unavoidable for a pure
static-host architecture. See `docs/capability-security.md` §6.

## 3. Rotation Procedure — `ztl cap rotate-signing-key`

Rotation applies in two scenarios:

- **Compromise** (suspected or confirmed leak of
  `ztl_CAP_SIGNING_KEY` — e.g. CI secret exposed, operator machine
  compromised, backup stolen — attacker A6 / A7).
- **Routine key hygiene** on a schedule of the operator's choosing.

The rotation lifecycle is a **coordinated four-step sequence**. Each
step is separately failable; do not conflate them.

### Step-by-Step

```bash
# 1. Generate a new Ed25519 keypair and update
#    recipients.toml[vault].signing_pubkey in place.
ztl cap rotate-signing-key > new-signing-key.txt

# 2. Store the new ztl_CAP_SIGNING_KEY in your password manager.
#    The line emitted on stdout has the form:
#      export ztl_CAP_SIGNING_KEY='<base64-standard-32-bytes>'
#    Do NOT commit new-signing-key.txt; copy the value across and
#    shred the file.
source new-signing-key.txt   # if you trust your shell's history handling
shred -u new-signing-key.txt

# 3. Rebuild the vault with the new key exported so every
#    /c/*.html is re-signed and a fresh shim bundle is emitted
#    with the new embedded pubkey (and new SRI hash).
ztl build --capability

# 4. Deploy the rebuilt dist AND cache-invalidate the shim at the CDN.
#    Order matters: invalidate the shim FIRST, or run both in the
#    same atomic deploy, so readers cannot fetch new ciphertext
#    under the OLD shim.
deploy.sh dist/                # or your equivalent
cdn-purge /assets/shim.js
cdn-purge '/c/*'               # for good measure; optional since
                                # the new SRI hash forces a re-fetch
                                # of the shim but not of pages
```

`ztl cap rotate-signing-key` itself performs **only steps 1 and 2**.
It will not rebuild the vault or touch the CDN — those live in `ztl
build` and your deploy pipeline, respectively, and combining them
would entangle secret emission with long-running encryption.

### The Shim Cache Invalidation Is Load-Bearing

If the CDN serves **new** signed pages while a reader's browser still
holds the **old** shim, every signature check fails. The old shim's
embedded pubkey cannot verify signatures produced by the new private
key, and the reader sees `signature-failed` on every page.

Mitigations (in order of preference):

1. **Atomic deploy.** Point readers at the new pages only after the
   CDN has flushed `/assets/shim.js`.
2. **Pre-invalidate.** Issue the CDN purge for `/assets/shim.js`
   before pushing the new `dist/`. Practically, for most CDNs, this
   means purging the shim, waiting for the TTL, then flipping the
   content.
3. **Low-traffic window.** Schedule rotations during a period when
   reader-side cache churn is tolerable.
4. **Reader-side self-service.** Readers can hard-refresh
   (Ctrl/Cmd-Shift-R) to force a shim re-fetch. Document this in
   your announcement.
5. **Observability.** Watch `OBS-3413` (signature-failure counter)
   post-cutover for a spike. A sustained spike indicates that a
   non-trivial fraction of readers still hold the old shim.

A dual-pubkey-slot scheme (shim trusts both old and new pubkey during
a transition window) is identified in SPEC-034 §14 OQ-5 and Tier 2
review S2-05 as a v2 consideration. **It is not in v1** — treat
rotation as a short-lived, planned disruption, not a hot-swap.

### What `ztl cap rotate-signing-key` Prints

The command emits a banner + export line on **stdout** and a guidance
line on **stderr**. Capture stdout if you want to save the key; the
stderr line is a reminder — it will not appear in a redirected file.

Stdout (exactly once):

```
# ztl cap rotate-signing-key — new Ed25519 vault-signing key (SPEC-034 REQ-3427)
#
# Store the new signing-key in your password manager BEFORE rebuilding.
# This key is printed to this terminal exactly once; ztl does not
# persist or log it.
#
# recipients.toml[vault].signing_pubkey has been updated in-place:
#   ed25519:<base64url-pubkey>
#
export ztl_CAP_SIGNING_KEY='<base64-standard-private-scalar>'
```

Stderr:

```
[ztl cap rotate-signing-key] new public key written to <vault>/recipients.toml.
Next: (1) rebuild the vault with the new `ztl_CAP_SIGNING_KEY` exported so
every page is re-signed; (2) deploy the rebuilt dist + new shim bundle;
(3) cache-invalidate `/assets/shim.js` (and any versioned shim URL) at the CDN
so readers with a cached OLD shim pick up the new embedded pubkey.
```

`recipients.toml` is modified in place; commit the change alongside
the rebuild so the vault-signing pubkey travels in version control
next to the content it authenticates.

### Signing-Key Loss (No Backup)

If `ztl_CAP_SIGNING_KEY` is lost without the operator having rotated
first, previously-signed content remains readable indefinitely — the
shim continues to verify against the pinned pubkey for any ciphertext
the old key signed. However, **no new content can be signed**. The
recovery path is identical to rotation: generate a new key
(`ztl cap rotate-signing-key` still works — it does not need the old
private key to run), rebuild, deploy, invalidate the shim.

An M-of-N Shamir-split recovery key is noted as a future option in
SPEC-034 §14 OQ-2. v1 has no such facility; operators should back up
`ztl_CAP_SIGNING_KEY` alongside `ztl_CAP_SECRET` using the same
secret-management hygiene they apply to any long-lived private key.

## 4. Emergency Shutdown — `ztl cap emergency-shutdown`

Emergency shutdown is the operator procedure for taking a
capability-mode deployment **offline at the host level**. Use it when
rotation is insufficient — typical triggers:

- `ztl_CAP_SECRET` and `ztl_CAP_SIGNING_KEY` both compromised.
- Unknown scope of compromise; operator needs to halt service while
  investigating.
- Legal, contractual, or safety requirement to suspend access on a
  fixed deadline.

**`ztl cap emergency-shutdown` is a documentation-generation
command.** It prints a printable operator runbook and exits 0. It does
NOT:

- modify any files on disk,
- purge any CDN,
- rotate any keys,
- make any network calls,
- nor does it ship any cryptographic kill-switch.

**The spec has no kill-switch by design.** A reader who has already
decrypted and cached a page can retain it indefinitely; capability
mode does not claim forward secrecy (see SPEC-034 NFR-3414). Shutdown
is a *host-level* action — DNS, CDN, secrets, announcements — and
must be performed by a human operator following the checklist.

### The Checklist

Running `ztl cap emergency-shutdown` inside a vault directory prints
five numbered sections, each titled and separated by blank lines. The
checklist is deterministic; piping it into a text editor or mailing
it to the incident bridge is safe.

```
ztl cap emergency-shutdown
===========================
vault:  <vault-basename>
deploy: <not configured — substitute your hostname>

This command does NOT modify any files, purge any CDN, or rotate any
key material. It prints the operator actions required to take the
wiki offline at the host level (SPEC-034 REQ-3431, §11.3). Work
through the steps in order; none of them are reversible.

Step 1 — Remove or redirect DNS
Step 2 — CDN: purge /c/* objects
Step 3 — Rotate ztl_CAP_SECRET + ztl_CAP_SIGNING_KEY
Step 4 — Announce to readers
Step 5 — Re-enrolment (when service resumes)
```

The body of each step enumerates concrete actions; the full output is
the one to follow during an actual incident (it surfaces live vault
context — cohort ids, on-disk signing pubkey — that a generic copy of
this document cannot).

### Step Semantics

**Step 1 — DNS.** Point the deployment's DNS record at a holding page
or delete it entirely; wait for TTL propagation before declaring the
site offline. DNS change is the **fastest** control you have;
everything downstream assumes the origin is no longer resolving for
readers.

**Step 2 — CDN purge.** Instruct the CDN to purge or delete every
object under `/c/*` so cached encrypted pages can no longer be served.
This also invalidates `/assets/shim.js`. Keep purge receipts — if a
reader surfaces a cached page later from a mirror, you want evidence
that the origin was flushed.

**Step 3 — Rotate secrets.** Run `ztl cap genkey` to produce fresh
`ztl_CAP_SECRET` and `ztl_CAP_SIGNING_KEY`. Store them in the
password manager; the old values are now considered compromised. Any
subsequent rebuild SHALL use the new secrets. (This is a superset of
`ztl cap rotate-signing-key` — emergency shutdown rotates both the
content-encryption secret AND the signing key, since the incident
scope is typically broader than a single key class.)

**Step 4 — Announce.** Notify every affected cohort. If
`recipients.toml` is present and parseable, the checklist enumerates
each cohort id (and `name`, if set) inline so the operator has a
ready-made distribution list. If the file is missing or malformed,
the checklist falls back to a generic "announce through every reader
channel you have on record" and the step completes without vault
context — an operator running this in an incident does not need a
parser error on the critical path.

**Step 5 — Re-enrolment.** When (and if) service resumes, issue fresh
`ztl cap invite` grants for returning readers. Do NOT reuse any old
invite URL, grant id, or cohort salt. The new deployment starts from
a clean TOFU state: readers re-bind passkeys per device against the
new vault-signing pubkey, the new cohort salts, and (for hardened
mode) the new per-cohort PRF inputs.

### Structured Output

The same checklist is available in JSON (`--json` or `-f json`) for
scripts, incident-response tooling, and agents that parse structured
output. The wire shape is:

```json
{
  "command": "ztl cap emergency-shutdown",
  "spec": "SPEC-034 REQ-3431",
  "automated": false,
  "vault_name": "…",
  "deploy_target": "…" | null,
  "signing_pubkey": "ed25519:…" | null,
  "cohorts": [ { "id": "…", "name": "…" | null }, … ],
  "steps": [ { "n": 1..5, "title": "…", "summary": "…" }, … ]
}
```

Field names and step titles are a committed wire contract. Output
format is chosen **explicitly** — the command does not auto-promote
to JSON on a non-TTY pipe, because piping the checklist into `less` /
`cat` / `mail` is a first-class use case and silently swapping in
JSON would be the wrong default.

## 5. Threat Model Recap

This table summarises the signing-specific attackers (reproduced from
SPEC-034 §11.1 for convenience; the spec is normative):

| Attacker | Capability                                           | Mitigation                                                                 |
| -------- | ---------------------------------------------------- | -------------------------------------------------------------------------- |
| **A3**   | CDN-compromised; substitutes served ciphertext       | **Blocked** by Ed25519 signature verification in shim (REQ-3427).          |
| **A6**   | CI-compromised; reads `ztl_CAP_SIGNING_KEY` env var | Rotate via `ztl cap rotate-signing-key`; rebuild; cache-invalidate shim.  |
| **A7**   | Signing-key compromiser (backup, machine, or CI)     | Same as A6. Operational-security problem, not cryptographic.               |

A3 is the attacker that motivated the v0.4.0 signing layer. A7 is
newly named in v0.4.0 to make the threat model honest about the fact
that the signing key is now a single-point-of-compromise for
**authorship authenticity** (it does NOT read content — readers'
passkeys still gate decryption).

Signing-key brute-force is ~2¹²⁸ classical; the interesting attack is
**theft**, not cryptanalysis. Protect the key with the same hygiene
you apply to `ztl_CAP_SECRET`.

## 6. Cross-References

- `specs/SPEC-034.md` — normative spec. §4 REQ-3427 / REQ-3431 for
  requirements; §6 CON-3411 for the protocol; §7 ADR-3412 for the
  design rationale; §11.1–11.3 for the threat model, residuals, and
  incident-response playbooks.
- `docs/capability-security.md` — the operator-facing security model
  overall; §6 "Content-Authenticity Trust Model" covers the signing
  layer in the wider context; §9 cross-references every incident
  response (this document is the long-form expansion of the two
  signing-relevant rows).
- `src/cap/sign.rs` — build-side signing implementation.
- `src/cap/shim/signature.ts` — shim-side Ed25519 verification.
- `src/cap/emergency_shutdown.rs` — pure-core checklist renderer.
- `src/main.rs` — effectful shells (`cmd_cap_rotate_signing_key`,
  `cmd_cap_emergency_shutdown`).
- `.hence/reviews/cap-tier1-crypto-2026-04-20.md` — Tier 1 crypto
  review (covers REQ-3427 and the signing layer).
- `.hence/reviews/cap-tier2-2026-04-20.md` — Tier 2 fresh-context
  review (S2-05 notes the dual-pubkey-slot deferral).

---

*If any statement in this document conflicts with SPEC-034, the spec
governs.*
