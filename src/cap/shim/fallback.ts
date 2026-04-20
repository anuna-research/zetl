// REQ-3412 / OBS-3412 — Graceful fallback for non-PRF browsers.
//
// When the reader's browser/authenticator combination does not expose
// the WebAuthn PRF extension, the shim cannot wrap priv_A into a
// passkey binding for subsequent visits (CON-3409 TOFU). Instead of
// failing closed, the pipeline operates in "fragment-required" mode:
//
//   * priv_A is read from the URL fragment on every visit
//   * no TOFU bind runs (idbFactory is forced to null into
//     `acquireIdentity`)
//   * `history.replaceState` is NOT called — the fragment stays in
//     the URL so the reader can bookmark / reload without losing
//     decryptability
//   * a non-intrusive banner is appended to the capability host,
//     linking to the reader-docs `#fallback-prf-unavailable` anchor
//   * `performance.mark("zetl:cap:fallback-prf-unavailable")` fires
//     so RUM-capable operators can alert on spikes (OBS-3412)
//
// Detection uses `PublicKeyCredential.getClientCapabilities()`
// (WebAuthn L3) when available and falls back to feature-probing
// `navigator.credentials.create` + `PublicKeyCredential`. The probe
// is asymmetric with `enroll.ts::detectPrfSupport()` only in its
// output shape — both take the same permissive branch on L2-only
// browsers, because a reliable authoritative check still needs a
// post-`create()` inspection of `getClientExtensionResults().prf`.

/// Performance-API mark name for OBS-3412. Kept as a const so
/// operator RUM dashboards pivot on a stable string.
export const FALLBACK_PERF_MARK = "zetl:cap:fallback-prf-unavailable";

/// Section anchor in `/reader.html` (mirrored in
/// `docs/reader-troubleshooting.md`) that `[data-zetl-fallback]`
/// banner deep-links to.
export const FALLBACK_DOC_ANCHOR = "fallback-prf-unavailable";

export type PrfReason =
  | "no-navigator"
  | "no-public-key-credential"
  | "caps-prf-false"
  | "no-webauthn-create";

export interface PrfAvailability {
  available: boolean;
  /// Machine-readable tag for the reason PRF is unavailable. Included
  /// as the `performance.mark` detail so RUM dashboards can pivot
  /// across distinct cohorts (Safari-sans-PRF vs. private-browsing
  /// stubs vs. entirely-missing WebAuthn) without bolting a label on
  /// every mark name.
  reason?: PrfReason;
}

/// Probe whether WebAuthn PRF is advertised on this runtime. Never
/// throws — any probe error is classified as "unavailable" so the
/// fallback path is taken rather than surfacing a hard error to the
/// reader. Safe to call inside the pipeline before any identity
/// acquisition; the probe does not touch `credentials.get/create`.
export async function detectPrfAvailability(): Promise<PrfAvailability> {
  if (typeof navigator === "undefined") {
    return { available: false, reason: "no-navigator" };
  }
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const pk: any = (globalThis as any).PublicKeyCredential;
  if (!pk) {
    return { available: false, reason: "no-public-key-credential" };
  }
  if (typeof pk.getClientCapabilities === "function") {
    try {
      const caps = await pk.getClientCapabilities();
      if (caps && typeof caps === "object" && "prf" in caps) {
        return caps.prf
          ? { available: true }
          : { available: false, reason: "caps-prf-false" };
      }
      // `getClientCapabilities` present but returned an unexpected
      // shape — fall through to the feature-probe branch rather
      // than declaring unavailable on a runtime that probably does
      // support PRF.
    } catch {
      // L3 method present but threw — same fallthrough.
    }
  }
  const hasCreate = !!(
    navigator.credentials && "create" in navigator.credentials
  );
  if (!hasCreate) {
    return { available: false, reason: "no-webauthn-create" };
  }
  // L3 capability query absent + `navigator.credentials.create`
  // present → optimistically assume PRF is available. This matches
  // `enroll.ts::detectPrfSupport()` so a reader who bypasses the
  // enrol page and lands on a delegated-URL invite sees the same
  // branch taken. A downstream PRF miss (e.g. Safari on a browser
  // that has `create()` but no `prf` extension result) still lets
  // TOFU's inner `performTofu()` catch it via `TofuError("no-prf")`.
  return { available: true };
}

/// Emit OBS-3412 performance mark. Best-effort: if `performance.mark`
/// isn't present, or the runtime rejects the optional `detail` arg,
/// the call is silently downgraded. Never throws — callers treat
/// observability as advisory.
export function emitFallbackMark(reason?: PrfReason): void {
  if (typeof performance === "undefined" || typeof performance.mark !== "function") {
    return;
  }
  try {
    if (reason) {
      performance.mark(FALLBACK_PERF_MARK, { detail: reason });
      return;
    }
    performance.mark(FALLBACK_PERF_MARK);
  } catch {
    try {
      performance.mark(FALLBACK_PERF_MARK);
    } catch {
      // No perf API at all — OBS-3412 is best-effort.
    }
  }
}

/// Append the non-intrusive banner as the first child of the
/// capability host, above the decrypted content. The banner is
/// plain text + one anchor — no styling hook beyond the
/// `[data-zetl-fallback]` attribute, so operators can CSS it
/// however they want from the enclosing shell. Idempotent: a second
/// call replaces the existing banner rather than stacking copies.
/// Returns the banner element for downstream assertions.
export function renderFallbackNotice(host: Element, reason?: PrfReason): Element {
  const doc = host.ownerDocument;
  if (doc === null) {
    throw new Error(
      "renderFallbackNotice: host element has no ownerDocument — cannot create banner",
    );
  }
  // Only deduplicate banners that are *direct* children of the host —
  // a decrypted page that happens to include a `[data-zetl-fallback]`
  // attribute in its own content (unlikely, but sanitiser allows any
  // data-* attr) must not be stripped. Walking children keeps the
  // check selector-free so happy-dom's partial `:scope` support
  // doesn't trip it up in the unit suite.
  for (const child of Array.from(host.children)) {
    if (child.hasAttribute("data-zetl-fallback")) child.remove();
  }

  const banner = doc.createElement("aside");
  banner.setAttribute("data-zetl-fallback", "");
  if (reason) banner.setAttribute("data-zetl-fallback-reason", reason);
  // `role="status"` lets assistive tech announce the degraded mode
  // without stealing focus — matches the "non-intrusive" wording in
  // REQ-3412.
  banner.setAttribute("role", "status");

  const summary = doc.createElement("p");
  summary.setAttribute("data-zetl-fallback-summary", "");
  summary.textContent = FALLBACK_COPY;
  banner.appendChild(summary);

  const help = doc.createElement("p");
  help.setAttribute("data-zetl-fallback-help", "");
  const a = doc.createElement("a");
  a.setAttribute("href", `/reader.html#${FALLBACK_DOC_ANCHOR}`);
  a.setAttribute("rel", "noopener noreferrer");
  a.textContent = "Why am I seeing this?";
  help.appendChild(a);
  banner.appendChild(help);

  host.insertBefore(banner, host.firstChild);
  return banner;
}

/// Exposed for the test suite's byte-stable copy assertion. If this
/// string changes the reader-docs section must update in lockstep
/// so the help link lands on the matching copy.
export const FALLBACK_COPY =
  "Running in fragment-required mode — your browser does not advertise WebAuthn PRF, so this page must be reopened from the full invite URL on every visit.";
