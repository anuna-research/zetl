---
title: "SPEC-034 v0.4.0 — Tier 1 Cross-Model Adversarial Review (browser shim)"
date: 2026-04-21
reviewer: "Claude Opus 4.7 (1M context) — fresh session, no prior shim-iteration exposure"
scope: "src/cap/shim/*.ts at HEAD of branch hence/cap-tier1-review-shim-v1"
review-type: "adversarial code review — state-machine correctness (first-use vs subsequent-use vs fallback dispatch), signature-verify-before-decrypt ordering, ServiceWorker hygiene, navigator.locks correctness, absence of key-material persistence outside IndexedDB, CSP+SRI enforcement"
companion-reviews: ".hence/reviews/cap-tier1-crypto-2026-04-20.md, .hence/reviews/cap-tier2-2026-04-20.md"
---

# Tier 1 Adversarial Shim Review — Browser TypeScript

## Reviewer Context

- Model: `claude-opus-4-7[1m]`, fresh session.
- Artefacts reviewed at HEAD of `hence/cap-tier1-review-shim-v1`:
  - `src/cap/shim/index.ts` (102 L) — entry + bundle-embedded pubkey wiring
  - `src/cap/shim/pipeline.ts` (250 L) — phase state-machine, lock, SW purge, dispatcher
  - `src/cap/shim/signature.ts` (55 L) — Ed25519 verify (`@noble/ed25519` v2)
  - `src/cap/shim/envelope.ts` (206 L) — CON-3404 envelope parser
  - `src/cap/shim/identity.ts` (349 L) — first-use / subsequent-use / fallback dispatch
  - `src/cap/shim/tofu.ts` (306 L) — first-use TOFU wrap
  - `src/cap/shim/unwrap.ts` (260 L) — subsequent-use PRF unwrap
  - `src/cap/shim/decrypt.ts` (142 L) — age v1 via `age-encryption` + bech32 helper
  - `src/cap/shim/sanitise.ts` (101 L) — DOM allowlist scrub (defence-in-depth)
  - `src/cap/shim/render.ts` (64 L) — fragment scrub, wiki href rewrite, innerHTML inject
  - `src/cap/shim/fallback.ts` (175 L) — REQ-3412 PRF probe + banner
  - `src/cap/shim/storage.ts` (252 L) — IndexedDB persistence
  - `src/cap/shim/session_policy.ts` (196 L) — REQ-3417 `[access.session]` cache
  - `src/cap/shim/prf_salt.ts` (46 L) — REQ-3414 salt
  - `src/cap/shim/errors.ts` (115 L) — error page
  - `src/cap/shim/enroll.ts` (721 L) — hardened-mode self-enrol (only skimmed for CSP)
  - `src/cap/shim/build.mjs` — esbuild bundler + SRI emission
  - Adjacent Rust surfaces: `src/cap/html_shell.rs`, `src/cap/deploy_headers.rs`
  - Unit tests under `src/cap/shim/test/` (happy-dom; no Chrome/CSP enforcement)
- Pinned crate/npm versions: `@noble/ed25519`, `@noble/hashes`, `age-encryption` (typage), `fake-indexeddb`, `happy-dom`.
- Companion reviews: Tier 1 crypto at `.hence/reviews/cap-tier1-crypto-2026-04-20.md`; Tier 2 spec at `.hence/reviews/cap-tier2-2026-04-20.md`. This review does not re-litigate spec-level findings except where the shim code pins the flagged behaviour.

## Severity key

- **S1** — blocker; the shim is broken against its own declared invariants, or a claimed mitigation does not actually run. Must be fixed before merge.
- **S2** — material gap; a mitigation is partial, a stated invariant is silently conditional on default config, or a dispatcher path admits a downgrade/confusion attack. Should be fixed before merge; may be parallelised.
- **S3** — clarity, hardening, or regression-defence hygiene. Worth fixing; does not block merge.

Priority areas per the task brief: **(a)** state-machine correctness (first-use vs subsequent-use vs fallback dispatch); **(b)** signature-verify-before-decrypt ordering; **(c)** ServiceWorker hygiene; **(d)** `navigator.locks` correctness; **(e)** absence of key-material persistence outside IndexedDB; **(f)** CSP + SRI enforcement.

## Summary

**Two S1 findings, four S2 findings, ten S3 findings.**

Both S1s are **the shim contradicts its own declared CSP**. The `Content-Security-Policy` header emitted by `src/cap/deploy_headers.rs::CAP_CSP` (and echoed verbatim into the HTML shell's `<meta http-equiv>`) includes two directives that, if actually enforced by the browser, will break the shim's basic operation:

1. `connect-src 'none'` blocks `fetch()` even for same-origin same-document URLs. The shim's `defaultFetchEnvelope` calls `fetch(location.pathname)` to retrieve the envelope. In Chromium with the meta CSP honoured, this fetch is rejected and the pipeline cannot advance past `Phase.SwPurged`.
2. `require-trusted-types-for 'script'` + `trusted-types 'none'` requires a `TrustedHTML` for every `innerHTML` assignment and simultaneously disallows creating any Trusted Types policy. The shim performs three `innerHTML` assignments (`render.ts:62`, `errors.ts:61`, plus an implicit empty-string wipe) with plain strings. Under Chromium Trusted Types enforcement these assignments throw `TypeError`. No policy is installed by the shim, and with `trusted-types 'none'` none could be installed even if someone tried.

Neither contradiction has been caught by tests: the shim is only exercised under happy-dom (which enforces neither CSP nor Trusted Types), the Rust integration test at `tests/cap_csp_sri_integration.rs` only asserts the CSP *string*, and the Playwright suite at `tests/nfr/` has no capability-shim coverage. A live deployment to Chrome/Edge would break at first load; Firefox/Safari would silently pass both until they catch up to Chromium's CSP-L3 enforcement.

The S2 findings cluster around (i) `sessionStorage` plaintext persistence of `priv_A` under the `per-session` / `per-minute` session policies — a defensible operator opt-in for UX, but one that silently voids the "no key material outside IndexedDB" invariant the Tier 1 brief asks me to check; (ii) a `webauthn-prf` cohort still honouring a URL-fragment `#k=` (the v1 deliberate-looser contract pinned by `unwrap.test.ts:468`) — a downgrade-to-fragment vector; (iii) the `cohort_mode` envelope header driving dispatch while remaining unsigned (Tier 2 S1-01 surfaces at code level here as a shim-dispatch confusion); (iv) a ServiceWorker-purge race where `unregister()` returns before the active SW stops controlling the page, so the subsequent `fetch()` may still be intercepted.

The S3 findings are hardening: no envelope size cap; lock held across the full pipeline including render; `priv_A` never best-effort-zeroed; the phases array is a log rather than a state machine; a materially wrong comment in `deploy_headers.rs` that may have seeded the CSP S1 (§S3-07); and a handful of minor diagnostic-leak and scoping tidies.

The pipeline's **signature-verify-before-decrypt ordering is correctly enforced by control flow** (`pipeline.ts:114-122` throws before reaching `IdentityAcquired`) and well-pinned by `test/signature-verify.test.ts` negative cases (a), (b), (c). The cryptographic construction itself (@noble/ed25519 v2 `verifyAsync` with injected sync-SHA-512, `age-encryption` via bech32-encoded raw scalar, AES-256-GCM + AAD for the TOFU wrap, REQ-3414 per-cohort PRF salt) matches the Rust-side derivation byte-for-byte where it needs to. I have nothing to flag on the signature-verify or decrypt cryptography at the shim layer — my quarrel is with the delivery environment (§S1-01, §S1-02) and the dispatcher (§S2-02, §S2-03, §S2-04).

---

## S1 Findings

### S1-01 — `connect-src 'none'` blocks the shim's envelope fetch (CSP vs. `defaultFetchEnvelope`)

**Affected:** `src/cap/shim/pipeline.ts:215-228` (`defaultFetchEnvelope`), `src/cap/deploy_headers.rs:86-97` (`CAP_CSP`), `src/cap/html_shell.rs::render_shell` (meta CSP fallback).

**Claim from the shim.** Pipeline `Phase.EnvelopeFetched` is reached by calling
```ts
const resp = await fetch(location.pathname, {
  credentials: "omit",
  redirect: "error",
  cache: "default",
});
```
on every page load (first-use *and* subsequent-use).

**Claim from deploy_headers.rs.** The `CAP_CSP` directive string — emitted as both an HTTP response header on `/c/*` and an inline `<meta http-equiv="Content-Security-Policy">` on the shell — contains `connect-src 'none'` with this justifying comment (`deploy_headers.rs:85-87`):

> `- `connect-src 'none'`        — no `fetch`/XHR to anywhere. The shim fetches its envelope from the page URL itself (same-origin same-document, which browsers do not gate on `connect-src`).`

**Problem — the comment is materially wrong.** CSP L3 `connect-src` governs every JavaScript-initiated network request: `fetch()`, `XMLHttpRequest`, WebSocket, EventSource, `navigator.sendBeacon()`, and `Navigator.ping`. There is **no exception** for same-origin requests, same-document requests, or requests whose URL string happens to equal `location.pathname`. Per the Fetch Standard's CSP integration step, the request is run through Content Security Policy regardless of origin; `connect-src 'none'` rejects it with a `net::ERR_BLOCKED_BY_CSP` (Chromium) / equivalent Firefox console error. The current page's own document was loaded via the initial navigation (governed by `navigate-to` / `default-src`), but a subsequent `fetch(location.pathname)` is a fresh network request subject to `connect-src`.

Concrete consequence: on any browser that enforces CSP-L3 `connect-src` (Chrome 56+, Firefox 60+, Safari 16+ with their respective quirks) the shim will:
1. Parse the shell HTML, install the `<meta>` CSP if the HTTP header is absent.
2. Execute `runPipeline` → acquire lock → purge SWs → call `fetch(location.pathname)`.
3. Browser rejects the fetch with a CSP violation. `resp` is never obtained; `defaultFetchEnvelope` rejects with a `TypeError`.
4. Pipeline falls to the error branch (`pipeline.ts:181-188`) with `errorKind = "internal"`.
5. User sees `"An internal error occurred while rendering this page. Reload…"` with a detail line that mentions CSP. Reloading does not help.

**Why this has not been caught.**
- `src/cap/shim/test/*.test.ts` runs under `node:test` + happy-dom. happy-dom does not enforce CSP. All tests inject `fetchEnvelope: async () => fx.envelopeBytes` — the real `defaultFetchEnvelope` is never exercised in CI.
- `tests/cap_csp_sri_integration.rs` is a Rust end-to-end assertion on the *text* of the CSP string and the `_zetl/capability-shell.html` bytes. It never spawns a browser.
- `tests/nfr/` (Playwright) has no capability-shim coverage — `grep -r capability|shim.js tests/nfr/tests` returns nothing.

**Why this is S1, not S2.** A cryptographic claim that fails in the field is S1 by definition; a usability claim that fails in the field is S1 when the usability claim is **"the shim renders any page at all under the declared CSP"**. The declared CSP and the shim's delivery path are stapled together as the centrepiece of REQ-3421 / CON-3410 (BUG-006 resolution). The shim cannot ship to a Chromium reader as-is.

**Recommendation.** Pick one of:
1. **Relax `connect-src` to `'self'`.** The shim only ever fetches same-origin URLs. `connect-src 'self'` still blocks exfiltration to third-party origins (the stated A3/A6 threats) without breaking envelope retrieval. This is the path of least resistance; CON-3410 prose can keep its threat-model framing.
2. **Replace `fetch()` with a `<link rel="preload">` + DOM read of the pre-fetched response.** This is bureaucratic for little gain; option (1) is strictly better.
3. **Deliver the ciphertext inline in the shell HTML.** Biggest refactor; also shifts trust boundaries. Not recommended for a shim-only fix.

Delete the `deploy_headers.rs:86-87` comment outright — it is false and will mislead future implementers. Option (1) is the intended fix; the comment should become *"`connect-src 'self'` — envelope fetch is same-origin; third-party exfil still blocked by default-src"*.

---

### S1-02 — Trusted Types enforcement bricks every `innerHTML` sink in the shim

**Affected:** `src/cap/shim/render.ts:55-64` (`renderInto`), `src/cap/shim/errors.ts:57-101` (`renderError` — `body.innerHTML = ""` + `host.innerHTML = ""`), indirectly `test/signature-verify.test.ts:61` (test fixture also assigns `innerHTML` but tests don't enforce CSP). The companion CSP directives: `src/cap/deploy_headers.rs:94-97` (`CAP_CSP`).

**Claim from the CSP.**
```
require-trusted-types-for 'script'; trusted-types 'none';
```
Combined meaning per CSP-L3 §8.3 and the Trusted Types spec (<https://www.w3.org/TR/trusted-types/>):
- `require-trusted-types-for 'script'` — every DOM sink listed in §5 ("script sinks") must receive a `TrustedHTML` / `TrustedScript` / `TrustedScriptURL`, not a plain string.
- `trusted-types 'none'` — no policies may be created via `trustedTypes.createPolicy()`. Calling `createPolicy` throws a `TypeError: Policy with name ... disallowed`.

With both directives present, there is **no legal way to satisfy an `innerHTML` sink**: you need a `TrustedHTML`, but no policy can be created to manufacture one.

**The shim's actual sinks.** Three string-to-`innerHTML` assignments:
1. `render.ts:62` — `host.innerHTML = sanitisedHtml;` (success path, after sanitiser)
2. `errors.ts:61` — `body.innerHTML = "";` (wipe before rendering the error page)
3. `errors.ts:67` — `host.textContent = "";` (safe; `textContent` is not a Trusted Types sink)

The plus-shaped DOM construction in `errors.ts:68-100` uses `createElement` + `setAttribute` + `textContent`, which are fine. The critical path is `errors.ts:61` + `render.ts:62`.

Under a Chromium browser that honours the meta CSP, the first `innerHTML =` throws `TypeError: Failed to set 'innerHTML' on 'Element': This document requires 'TrustedHTML' assignment.` The exception propagates out of `renderInto`, `pipeline.ts:181-188` catches it with `errorKind = "internal"`, and then `renderError()` immediately does `body.innerHTML = ""` — which *also* throws. The page is left with whatever half-rendered state it had before the pipeline ran (the blank shell HTML plus any console noise), and the reader has no visible diagnostic.

**Why this has not been caught.** Same as S1-01: happy-dom does not implement Trusted Types enforcement, and no Playwright suite loads the shim under an enforced CSP. The `require-trusted-types-for`/`trusted-types` directives have existed in the CSP string since `src/cap/deploy_headers.rs:97` was first written, but they were never exercised.

**Interaction with S1-01.** The two S1s compound: even if `connect-src` is relaxed to `'self'` (fixing S1-01), the Trusted Types directive still bricks the render path. Both must be addressed.

**Recommendation.** Pick one of:
1. **Allow a single named policy and install it.** Change `trusted-types 'none'` → `trusted-types zetl-shim` (or any single name), and install a policy in `index.ts` before `runPipeline`:
   ```ts
   // eslint-disable-next-line @typescript-eslint/no-explicit-any
   const tt = (globalThis as any).trustedTypes;
   const policy = tt && typeof tt.createPolicy === "function"
     ? tt.createPolicy("zetl-shim", {
         createHTML: (s: string) => s,  // trusted: post-sanitiser only
       })
     : null;
   // Then in render.ts:
   host.innerHTML = policy ? policy.createHTML(sanitisedHtml) : sanitisedHtml;
   ```
   The policy's `createHTML` is an identity function because the string has already passed through `sanitise.ts`. The Trusted Types boundary documents *who* is permitted to mint HTML (namely the shim's sanitiser-audited path), which is the actual security property.
2. **Drop `require-trusted-types-for 'script'` entirely** and rely on sanitiser + sink audit. Weaker defence-in-depth; only pick this if (1) proves contentious.
3. **Eliminate all `innerHTML` assignments** by constructing the DOM through `DOMParser().parseFromString` + `Node.appendChild`. Removes the sink entirely. Largest refactor.

Option (1) is what the CSP prose at `deploy_headers.rs:92-93` ("the shim installs a named policy before injection") clearly *intended* — the code simply never did. The fix is a ~15-line change in `index.ts` plus the CSP directive tweak.

**Regression gate.** Add a Playwright test (`tests/nfr/tests/cap-shim-csp.spec.ts`) that loads a seeded envelope under the full CSP headers and asserts the capability host is populated. Without it this reviewer has no confidence that either S1 is being tested against the real enforcement behaviour.

---

## S2 Findings

### S2-01 — ServiceWorker purge races against its own `fetch()` call

**Affected:** `src/cap/shim/pipeline.ts:102-105` (purge), `pipeline.ts:207-213` (`defaultPurgeServiceWorkers`), `pipeline.ts:215-228` (`defaultFetchEnvelope`).

**Problem.** The flow inside the lock is:
```ts
await deps.purgeServiceWorkers();           // line 102
trace.phases.push(Phase.SwPurged);
const envelopeBytes = await deps.fetchEnvelope();  // line 105
```
`defaultPurgeServiceWorkers` does:
```ts
const regs = await navigator.serviceWorker.getRegistrations();
await Promise.all(regs.map((r) => r.unregister()));
```
`ServiceWorkerRegistration.unregister()` resolves when the registration is *marked* for removal, but the active SW continues to control any page/client that was already under its scope until every controlled client is either reloaded or navigates away. The page currently executing the shim is one such controlled client.

Consequence: a SW that had previously installed a `fetch` handler on `location.pathname` will still intercept the envelope fetch the shim makes on line 105. The ciphertext returned to the shim could be the SW's cached copy, a modified copy the SW chose to serve, or — in a pathological case — a SW-synthesised response the SW constructed from an attacker-controlled payload.

**Why the mitigation is partial.**
- SPEC-034 REQ-3428 requires `Clear-Site-Data: "cache", "storage", "executionContexts"` on `/enroll.html` and `/logout` (see `deploy_headers.rs:65`). `"executionContexts"` terminates the controlling SW. But `/c/*` does NOT receive `Clear-Site-Data`, so the SW purge on ciphertext pages is the only gate.
- `navigator.serviceWorker.controller` is not consulted. If `navigator.serviceWorker.controller !== null` at pipeline start, the current page is still under SW control and a fresh navigation is required to detach.
- The signature-verify check *does* run over the bytes returned by `fetch()`, so a SW that substitutes attacker-written ciphertext still fails Ed25519 verification (REQ-3427 still holds). Where this bites is the UX and error-reporting: a mismatched-sig diagnostic is emitted on a page the operator believed had been freshly fetched; a DoS where the SW replays stale ciphertext can produce `decrypt-failed` / `need-invite` spurious diagnostics the reader cannot distinguish from genuine revocation.

**Why this is S2, not S1.** The signature-verify step downstream still gates rendering attacker-supplied content. The hygiene gap is about freshness, UX confidence, and consistency with the stated "purge then fetch" invariant — not a plaintext-disclosure vector.

**Recommendation.** Before the fetch, check `navigator.serviceWorker.controller`:
```ts
export async function defaultPurgeServiceWorkers(): Promise<void> {
  if (typeof navigator === "undefined" || !("serviceWorker" in navigator)) return;
  const regs = await navigator.serviceWorker.getRegistrations();
  await Promise.all(regs.map((r) => r.unregister()));
  if (navigator.serviceWorker.controller !== null) {
    // Legacy SW still controls this client; force a hard reload so the
    // unregister takes effect before we fetch the envelope.
    location.reload();
    // Reload is asynchronous; the rest of the pipeline will not run.
    // Return a never-resolving promise to halt the caller cleanly.
    await new Promise(() => {});
  }
}
```
Alternative: call `fetch(location.pathname, { cache: "no-store", ...})` — browsers bypass the SW for `cache: "no-store"` in most implementations, though this behaviour is under-specified. Belt-and-braces is to combine the `controller`-check with `cache: "reload"` (more widely honoured than `"no-store"` for SW bypass).

---

### S2-02 — `per-session` / `per-minute` policies store `priv_A` as plaintext in `sessionStorage` (contradicts the "no key material outside IndexedDB" invariant)

**Affected:** `src/cap/shim/session_policy.ts:119-145` (`storeCachedPrivA`), `session_policy.ts:180-186` (`base64UrlEncode`), documented at `session_policy.ts:18-22`.

**Claim from the Tier-1 task brief.** One of the explicit review criteria is *"absence of key-material persistence outside IndexedDB"*.

**Observed behaviour.** When `sessionPolicy` is set to `"per-session"` or `"per-minute"` (REQ-3417 `[access.session]`), `storeCachedPrivA` persists the raw 32-byte `priv_A` — base64url-encoded, NO AEAD wrap — to `sessionStorage` under the key `zetl:cap:uv:<origin>:<cohortId>` (`session_policy.ts:27`). The comment block at lines 18-22 is honest that this is an opt-in trade-off, but the Tier-1 brief asks for an **absence**, not a conditional.

**Attack model this enables.**
- **Sanitiser-bypass XSS.** The allowlist-based sanitiser in `sanitise.ts` is strict, but any bypass (novel mXSS vector, sanitiser bug, content with a tag the allowlist forgets about in a future spec) runs JavaScript in the same origin as the shim. Same-origin code can read `sessionStorage.getItem("zetl:cap:uv:<origin>:<cohortId>")`, base64url-decode, and recover `priv_A`. The TOFU-wrapped IDB record is not readable (no PRF prompt available to an arbitrary script) — but the sessionStorage copy needs no ceremony.
- **Browser extensions with storage access.** Many extensions request `<all_urls>` storage permissions. sessionStorage is explicitly enumerated as one of the surfaces `chrome.storage` / extension APIs can read.
- **Tab-crash memory disclosure.** sessionStorage is memory-mapped on most browsers; a tab-crash dump (auto-submitted by some crash-reporting stacks) may include the stored key.

The IDB wrap is explicitly designed so a raw DB dump does not yield `priv_A` — the PRF ceremony is the unlock. The `per-session` cache structurally defeats that property.

**Why this is S2, not S1.** The default policy is `per-page` (`session_policy.ts:24`) — a fresh install does not enable the feature. The policy is also clearly operator-documented as a UX/security trade. But it is a material contradiction of the brief's invariant and worth pinning at code-level so future `[access.session]` expansions don't extend the surface.

**Recommendation (menu; pick at operator-policy level).**
1. **Move the cached key to IndexedDB with the same PRF wrap** as the durable binding. Fast path: derive a session-scoped K_cache via HKDF(prf_output, info="zetl/session-cache/v1"), AES-GCM-wrap priv_A, store in a second IDB store keyed by (origin, cohortId). On the cache-hit path the shim still has to `credentials.get()` once per tab session — that's the bit the operator wanted to avoid, so this option is UX-equivalent to `per-page` and defeats the purpose.
2. **Keep sessionStorage but wrap under a per-tab-ephemeral AES-GCM key held in a JS closure** (closure lives as long as the page; tab navigate-away clears it). Defeats XSS because the key is unreachable; defeats extension reads because the ciphertext alone is useless. Adds ~30 lines to `session_policy.ts`. This is the pragmatic fix.
3. **Rename the feature honestly.** If (1) and (2) are out of scope, at minimum mark the stored entry `"priv_A": <b64>` with a `"warning": "plaintext-priv-A"` and surface it in `zetl cap audit` so an operator who enables `per-session` sees the trade in their own audit trail.

Recommend option (2). Document the change in `docs/capability-security.md` alongside the existing `[access.session]` section.

---

### S2-03 — `webauthn-prf` cohort still honours a URL `#k=` fragment (downgrade / fragment-injection path)

**Affected:** `src/cap/shim/identity.ts:99-127` (`acquireIdentity`), pinned by `src/cap/shim/test/unwrap.test.ts:468-496`.

**Observed behaviour.** `acquireIdentity` checks the fragment first, regardless of cohort mode:
```ts
const fromFragment = readFragmentKey(ctx.locationHash);
if (fromFragment !== null) {
  // First-visit path: try to persist a passkey-wrapped copy...
  await maybeBindFragment(ctx, fromFragment);
  return fromFragment;
}
const unwrapped = await tryUnwrapBinding(ctx);
```
The `cohortMode` branch inside `throw IdentityError("need-invite")` exists only for the diagnostic copy, not as a dispatch gate. A reader in a `webauthn-prf` cohort who visits `https://wiki.example/c/<path-cap>/<slug>.html#k=<attacker-priv_A>` will:
1. Decode the attacker's scalar from the fragment.
2. Attempt TOFU wrap of the *attacker's* scalar (silently skipped if binding already exists — `tofu.ts:123-126`).
3. Return the attacker's scalar to `runPipeline`.
4. Age-decrypt the ciphertext under the attacker's scalar. Age emits `"no identity matched"`, surfacing `errorKind = "decrypt-failed"`.

The test at `unwrap.test.ts:468` explicitly pins this behaviour:
> "In v1 we accept this looser behaviour; a stricter hardened-mode guard is a follow-up task. This test pins the current contract rather than the ideal."

**Attack variants.**
1. **Downgrade.** A phished reader clicks an attacker-crafted link for a *legitimate* cohort, with an attacker-chosen fragment. Decryption fails with `"Could not decrypt this page — the invite may have been revoked or rotated. Ask your wiki operator for a new invite."` — a perfect phish setup: reader asks operator for a new invite over some attacker-influenced channel.
2. **Fragment-persistence exfiltration.** On `webauthn-prf` + fragment-present, the shim runs `maybeBindFragment` which *persists* the attacker's scalar to IDB if no binding exists. Subsequent visits without the fragment unwrap the attacker's key. A reader who got phished once and has not yet enrolled legitimately is bound to the attacker's identity on that device for that cohort. Recovery requires `forgetBinding()` + re-enrol.
3. **Cohort-mode flip → fragment attack** (compounds with S2-04 below). A CDN that flips `Zetl-Cohort-Mode` from `webauthn-prf` → `delegated-url` on a *hardened* cohort ciphertext makes the shim look for `#k=` where none is expected; flipping the other direction means the shim still parses and honours `#k=` on a page the operator believed was hardened-only. Either direction admits this S2-03 path.

**Why this is S2.** The test pins "the current contract" explicitly, so this is a documented follow-up. But it is material enough to require a named gate before the shim merges to main — either the follow-up task or an explicit `safe_mode`-style gate. Per `task-cap-safe-mode` idiom, a `SafeMode::Fragment` refusal with a clear diagnostic would close the path.

**Recommendation.** In `acquireIdentity`, add at the top:
```ts
if (ctx.cohortMode === "webauthn-prf") {
  const fromFragment = readFragmentKey(ctx.locationHash);
  if (fromFragment !== null) {
    throw new IdentityError(
      "mode-not-supported",
      "hardened (webauthn-prf) cohorts do not accept URL-fragment invites — \
       this link appears to be misrouted",
    );
  }
  // Fall through to unwrap branch only.
}
```
This removes the fragment path from hardened-mode entirely. The `IdentityError` kind `"mode-not-supported"` already exists (`identity.ts:43-54`); wire it through `errors.ts::errorKindFromException` to a clear "ask operator for a fresh enrolment" copy.

Update `unwrap.test.ts:468` to assert the *refusal*, not the looser behaviour.

---

### S2-04 — Envelope `cohort_mode` + `cohort_id` drive shim dispatch but are not covered by the Ed25519 signature

**Affected:** `src/cap/shim/envelope.ts:49-187` (parser), `src/cap/shim/pipeline.ts:140-152` (dispatch read of `envelope.header.cohortMode` / `.cohortId`), upstream CON-3404 (signed range excludes headers).

This is the code-side echo of Tier 2 S1-01 ("Envelope headers are unsigned but drive shim dispatch"). I include it here because the shim's behaviour pins specific attacker paths that the spec-level discussion does not enumerate, and because fixing it purely at the shim level (without revising CON-3404) is possible but misses the root cause.

**Observed behaviour at the shim layer.** `parseEnvelope` pulls `cohortId`, `cohortMode`, `slug`, `buildEpoch`, `signature` out of the UTF-8 header block (`envelope.ts:77-162`). Only the `signature` is fed into `verifyEd25519(pubkey, ciphertext, signature)` — and only `ciphertext` is the signed message. `cohortId` and `cohortMode` flow directly into `acquireIdentity` (`pipeline.ts:140-151`) without any post-verify consistency check.

**Concrete dispatcher-level attacks** (the ones that actually run, given the verification layer passes):
1. **Cohort-id mis-attribution.** CDN re-serves a valid `engineering` ciphertext under `Zetl-Cohort-Id: ops`. The shim reads the stored TOFU binding for `"ops"` (if any) and tries to unwrap with the wrong cohort's key. Decrypt fails. OBS-3412 / observability counters get mis-bucketed (the `ops`-cohort miss rate spikes).
2. **Mode flip `delegated-url → webauthn-prf`.** The shim in `acquireIdentity` ignores the fragment only for the `"need-invite"` diagnostic copy, not for dispatch — so fragment-present + mode flipped produces the same behaviour as S2-03 variant 3. The ciphertext still signature-verifies.
3. **Mode flip `webauthn-prf → delegated-url`.** Shim expects a fragment. Hardened-cohort reader has none. `need-invite` diagnostic fires. UX → social-engineering scaffold (see Tier 2 S1-01).

**Why the signature verify does not save us.** `verifyEd25519` is called with `envelope.ciphertext` as the message. Any header byte — including the bytes that name the cohort and drive the mode branch — can be rewritten without invalidating the signature. The tests at `signature-verify.test.ts` do not assert header immutability because CON-3404 explicitly carves the headers out.

**Why this is S2, not S1.** Every attack variant ultimately lands in a diagnostic state, not plaintext disclosure — the age recipient-list check in the ciphertext is the ultimate gate on who can decrypt. But the "blocks CDN substitution" claim (§11.1 matrix) is stronger than what the code actually delivers, and the `mode-flip` path is a cheap phishing primitive.

**Recommendation.** Two options, not mutually exclusive:
1. **Spec-side fix (preferred, per Tier 2 S1-01).** Cover the envelope headers in the signed byte range via a canonical serialisation. The shim parses the headers before verify (needed for dispatch), then verifies signature over `canonical(headers) || ciphertext`.
2. **Shim-side defence-in-depth until (1) lands.** After signature verify, cross-check `envelope.header.cohortId` against a separately-delivered pinning manifest. If the page URL is `/c/<path-cap>/<slug>.html` and the cohort id is a function of `<path-cap>`, the shim can derive the expected cohort id from the URL path and compare.

The latter is a ~30-line addition to `pipeline.ts`. Do option (2) **now** while option (1) is in-flight, to avoid leaving the dispatcher confusion open across the `v0.4.0 → v0.5.0` gap.

---

## S3 Findings

### S3-01 — `defaultFetchEnvelope` has no size cap (DoS / OOM)

**Affected:** `pipeline.ts:215-228`.

`const buf = await resp.arrayBuffer();` reads unconditionally. A hostile CDN (or any attacker-in-the-middle that has broken TLS) can serve an unbounded response. The browser's own memory cap is the only gate; there's no `Content-Length` sanity check and no bounded-read path.

Realistic envelopes are small (a few KB of header + a few MB of ciphertext at most). Add:
```ts
const CL = parseInt(resp.headers.get("content-length") ?? "0", 10);
if (CL > MAX_ENVELOPE_BYTES) {
  throw new Error(`envelope is ${CL} bytes; refusing to load > ${MAX_ENVELOPE_BYTES}`);
}
```
with `MAX_ENVELOPE_BYTES` set to e.g. 32 MiB. Or use a `ReadableStream` reader with a running byte counter.

### S3-02 — `navigator.locks` wraps the full pipeline including render

**Affected:** `pipeline.ts:99-180`.

The lock is held across sanitise + render + scrubFragment + rewriteWikiHrefs. None of those operations need cross-tab serialisation — they mutate the current document only. The actual critical section is:
- SW purge + envelope fetch (avoid racing against a re-registering SW)
- TOFU wrap / unwrap (serial IDB write)

Two tabs of the same origin for different cohorts end up serialising through unrelated work. Recommendation: release the lock once `Phase.IdentityAcquired` is recorded. Or narrow the lock to `tryUnwrapBinding` / `maybeBindFragment` specifically.

### S3-03 — `priv_A` / `K_wrap` / `prfOutput` never best-effort zeroed

**Affected:** `tofu.ts:189-204`, `unwrap.ts:165-179`, `identity.ts:101-127`, `decrypt.ts:21-62`, `session_policy.ts:132` (plaintext encode).

JavaScript's GC makes reliable zeroisation impossible, but best-effort `.fill(0)` on the Uint8Array right before it falls out of scope is a known hygiene practice (e.g., `@noble/hashes` `clean()` helper). The Rust side at `src/cap/sign.rs` has a module-level discipline (`ZeroizeOnDrop` wrappers); the shim has none. Add:
- `prfOutput.fill(0)` after HKDF inputs done
- `kWrap.fill(0)` after `importKey` returns
- `privA.fill(0)` after `ageDecrypt` returns (the scalar is no longer needed)

Worst case these are no-ops against a determined attacker (GC may have already copied); best case they reduce residence time of key material in resident heap. Parity with Rust-side discipline, and the comment block in `sign.rs:40-46` suggests the project already values this.

### S3-04 — Pipeline phases are a push-only log, not a state machine

**Affected:** `pipeline.ts:30-43, 96-180`.

`Phase.Init..Errored` is enumerated as an object, but nothing *checks* that the current phase is a legal predecessor of the next one. The signature-verify-before-decrypt ordering is enforced by `throw` + try-catch, not by a state guard. If a future refactor introduces an early return that skips the throw, the phases array will silently record an impossible transition.

Two small improvements:
1. Wrap phase transitions in a helper `advance(trace, next)` that asserts the top of `trace.phases` is a permitted predecessor.
2. Add a `pipeline.test.ts::rejects-out-of-order-phase` that constructs a trace with `[Init, Decrypted]` and asserts the helper throws.

Neither changes observable behaviour — they just turn a discipline into a check.

### S3-05 — `deploy_headers.rs:86-87` comment asserts a browser-gating exception that does not exist

**Affected:** `src/cap/deploy_headers.rs:85-87`.

The comment

> "The shim fetches its envelope from the page URL itself (same-origin same-document, which browsers do not gate on `connect-src`)."

is false (see S1-01). It almost certainly seeded the shim's assumption that `connect-src 'none'` would work. Removing the comment (and fixing the directive) closes the loop. I flag this separately because a stray wrong comment in a file that serves as the single source of truth for the CSP is a hazard independent of the S1 fix — a future reviewer who only reads the Rust side will mis-understand the intent.

### S3-06 — `history.replaceState` scrub is best-effort and documented as such, but there's no fallback for UA sync

**Affected:** `render.ts:10-18`, §11.2 leak-hygiene list.

`scrubFragment` only touches the current document's URL. Browser sync, pinned tabs, extensions, link previewers, and URL-bar autocomplete histories all captured the pre-scrub URL. The spec (§11.2) acknowledges this. No S3 recommendation on the code itself; flagged here as a reminder that every `REQ-3406`-style guarantee must read "on this device only, after the first render".

### S3-07 — `defaultWithLock` has no timeout; stuck SW or runaway sibling tab can deadlock the pipeline

**Affected:** `pipeline.ts:234-243`.

`navigator.locks.request(LOCK_NAME, { mode: "exclusive" }, body)` with no `AbortSignal` waits indefinitely. A crashed-but-not-GC'd sibling page (rare but possible) can sit on the lock. Recommendation: pass `signal: AbortSignal.timeout(5_000)` so the shim surfaces a `lock-unavailable` diagnostic rather than a blank page.

### S3-08 — `LOCK_NAME` is a single string for all cohorts on the origin

**Affected:** `pipeline.ts:28`.

`"zetl-capability-shim"` is one lock for every cohort. Two tabs on two cohorts of the same wiki serialise even though their IDB stores are keyed by distinct `cohortId`. Minor UX; include cohortId in the lock name:
```ts
const lockName = `zetl-cap-${envelope.header.cohortId}`;
```
— but note this requires moving the lock acquisition to *after* envelope parse, which in turn requires a second re-entry to guard the SW-purge critical section. Probably not worth the complexity; tag as an optional polish.

### S3-09 — Error-page `detail` line echoes underlying message verbatim

**Affected:** `errors.ts:74-79`.

`small.textContent = detail;` echoes the full `err.message` string to the reader. For:
- `decrypt-failed`, detail contains the age library's error string ("no identity matched" vs "malformed header"). An attacker can distinguish "revoked by operator" (no identity matched, post-rotation) from "grant-missing-on-device" (no binding) from `"ciphertext tampered"` (malformed header). This is useful reconnaissance.
- `tofu-failed`, detail may include the PRF/AES error — usually benign, but a `no-prf` error message includes the string "PRF output was X bytes" with X from the authenticator.

Recommendation: classify detail strings into a fixed enum before display, and only render the enum tag. The full message can still go to `console.error` for operator diagnostics.

### S3-10 — `.innerHTML = ""` in `errors.ts:61` is a TT sink even under S1-02 fix

**Affected:** `errors.ts:57-101`.

Once S1-02 is fixed with a named policy, `errors.ts` needs to use the same policy (or switch to DOM construction). Right now `body.innerHTML = "";` plus `host.textContent = "";` is inconsistent — `body` cleanup via `innerHTML = ""` could be rewritten as:
```ts
while (body.firstChild) body.removeChild(body.firstChild);
```
which is Trusted-Types-safe regardless. The cost is ~2 lines. Do this as part of the S1-02 fix and the error-path works under any CSP variant (`'none'` or a named policy).

---

## Areas examined and found clean

- **Ed25519 verify path** (`signature.ts`). `@noble/ed25519` v2 `verifyAsync` with SHA-512 injected via `@noble/hashes` — RFC 8032 strict verification (non-canonical R/S rejected). Input-length checks at 32 (pubkey) / 64 (signature) bytes. Error → `false`, not throw. Matches Rust-side `ed25519-dalek::verify_strict` semantics.
- **Envelope parser** (`envelope.ts`). UTF-8 strict decode with `{ fatal: true }`; per-header bounds; schema-mismatch before any signature handling. Base64url decoder handles stray padding but never emits it. Unknown headers tolerated (forward-compat).
- **TOFU wrap AAD discipline** (`tofu.ts::performTofu`). AES-256-GCM with AAD = `utf8(origin || "/" || cohortId)`; IV from injected RNG; K_wrap via HKDF-SHA256(prf_output, info="zetl/tofu-wrap/v1", 32). Byte-identical derivation across wrap + unwrap (`tofu.test.ts::HKDF-derives K_wrap`).
- **PRF salt** (`prf_salt.ts::computePrfSalt`). SHA-256(PRF_SALT_PREFIX || origin || "/" || cohortId) — exact mirror of Rust-side `cap::enrolment::compute_prf_salt`; TEST-3414 cross-checks byte equality. No length-prefixing between `origin` and `cohortId`, but a strict cohortId charset check is enforced on the Rust side (see Tier-2 S3-x re length-prefixing — code is consistent with spec; any strengthening is a spec change).
- **Subsequent-visit unwrap** (`unwrap.ts::performUnwrap`). Re-derives AAD from stored `binding.origin`/`binding.cohortId` rather than trusting the persisted AAD field — defence-in-depth against IDB record tampering (comment at lines 181-186). Good pattern; keep it.
- **Fragment-required fallback** (`fallback.ts` + `pipeline.ts:131-150`). PRF probe runs *before* `acquireIdentity` at a single point. No `credentials.get/create` call on the probe path. `idbFactory: null` force-disables TOFU in fallback. Tested end-to-end in `fallback.test.ts`.
- **Sanitiser allowlist** (`sanitise.ts`). `DOMParser().parseFromString` + allowlist walk. Denied schemes regex (`javascript:`, `data:` except for `img`, `vbscript:`, `file:`, `about:`). `on*` attribute strip. Per-tag attribute whitelist. Conservative; matches the Rust `ammonia` pass. The `DOMParser()` route does not itself hit a Trusted Types sink.
- **age decrypt** (`decrypt.ts`). Raw 32-byte scalar → bech32 AGE-SECRET-KEY-1 → typage `Decrypter.addIdentity` → `decrypt(ciphertext, "uint8array")`. No identity persists across calls. Error classification into `not-a-recipient` / `malformed-ciphertext` / `identity-encode` is sensible.
- **SRI build path** (`build.mjs`). esbuild IIFE, minified, deterministic given inputs. Bundle → `createHash("sha384").update(bundleBytes).digest("base64")` → written to `shim.sri`. Rust side reads the sibling file and renders into the shell. Dev placeholder pubkey (`"A".repeat(43)`) is swapped at build time; manifest flags `placeholderPubkey: true` when in dev mode so a dev-mode bundle cannot be mistaken for a production one.
- **Envelope signature byte-range** (shim ↔ Rust parity). Shim verifies over `envelope.ciphertext` (bytes after the `\n\n` separator). Rust `sign.rs::build_envelope` signs the same byte range. Envelope round-trip test at `signature-verify.test.ts` pinning positive + 3 negatives.
- **`acquireIdentity` fragment parser** (`identity.ts::readFragmentKey`). Strict length + charset check before base64url-decoding; malformed fragments surface as `IdentityError("fragment-*")` rather than silently falling through to an IDB lookup.

## Test-harness observations (not findings, but context)

- `src/cap/shim/test/*` runs under `node:test` + happy-dom. happy-dom does not enforce CSP, does not implement Trusted Types, and injects `fetchEnvelope` so `defaultFetchEnvelope` is untested.
- `tests/nfr/tests/*.spec.ts` has no capability-shim coverage; the Playwright fixtures exist only for graph / render-latency NFR gates.
- The Rust-side `tests/cap_csp_sri_integration.rs` asserts the CSP *string* byte-for-byte but never loads the shim under a real browser with that CSP applied.
- Consequence: both S1 findings above were not caught because *no test anywhere in the tree exercises the shim under an enforced CSP*. I strongly recommend adding a Playwright spec `tests/nfr/tests/cap-shim-happy-path.spec.ts` that (a) serves a seeded envelope with the exact HTTP+meta CSP, (b) loads the shim, (c) asserts the capability host populates. This single test would catch both S1s and is a hard regression gate for any future CSP directive drift.

---

## Verdict

The signature-verify-before-decrypt ordering and the first-use / subsequent-use / fallback *logic* are well-implemented, well-tested under their injected-fake harness, and cryptographically sound at the shim layer. The state machine transitions in the intended order, the tests pin that invariant tightly, and the REQ-3427 negative cases all reach `Errored` without ever calling `credentials.get/create` or `ageDecrypt`.

The shim as it stands **cannot be merged to main** because of the two S1 CSP contradictions — they are single-function fixes, but without them the shim does not work on Chromium at all under its own declared policy. Both are straightforward:

1. `connect-src 'none'` → `connect-src 'self'` in `deploy_headers.rs::CAP_CSP` + delete the false comment at lines 85-87.
2. `trusted-types 'none'` → `trusted-types zetl-shim` + install a named identity-createHTML policy in `index.ts` before `runPipeline` + wire the policy through `render.ts::renderInto` and `errors.ts::renderError`.

Once both S1s land (with a Playwright regression gate), S2-01/S2-03/S2-04 are the remaining material issues for shim-main merge. S2-02 should be tracked as a follow-up once `[access.session]` hardening is scoped; it does not strictly block the shim itself, only the feature built on top of it.

The S3s are improvement tickets that can be parallelised with other v0.5.x work.

— reviewer
