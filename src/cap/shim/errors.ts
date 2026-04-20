// Error surface — user-visible diagnostics for the capability-mode
// host. Error copy is pinned by REQ-3427 ("This page's signature did not
// verify…") so byte-stable tests can assert the exact wording.

export type ErrorKind =
  | "signature-failed"
  | "need-invite"
  | "identity-unavailable"
  | "decrypt-failed"
  | "envelope-malformed"
  | "sw-purge-failed"
  | "lock-unavailable"
  | "host-missing"
  | "internal";

const COPY: Record<ErrorKind, string> = {
  "signature-failed":
    "This page's signature did not verify — possible tampering; contact your wiki operator",
  "need-invite":
    "This page is only readable from a fresh invite URL on this device. Ask your wiki operator for a new invite URL.",
  "identity-unavailable":
    "Could not recover the reading identity for this cohort on this device. Ask your wiki operator to re-invite you.",
  "decrypt-failed":
    "Could not decrypt this page — the invite may have been revoked or rotated. Ask your wiki operator for a new invite.",
  "envelope-malformed":
    "This page's envelope is malformed and cannot be read. Reload; if the problem persists, contact your wiki operator.",
  "sw-purge-failed":
    "Could not clear stale service workers on this origin — close all tabs for this site and try again.",
  "lock-unavailable":
    "Your browser does not support the concurrency lock this shim requires (navigator.locks). Use a recent Chromium- or Firefox-based browser.",
  "host-missing":
    "Capability-mode mount point <main data-zetl-capability> is missing from the HTML shell.",
  "internal":
    "An internal error occurred while rendering this page. Reload; if the problem persists, contact your wiki operator.",
};

/// Render a user-visible error page and stamp a `data-zetl-error=<kind>`
/// attribute the Playwright suite keys off for assertions.
export function renderError(kind: ErrorKind, detail?: string): void {
  const body = document.body ?? document.documentElement;
  // Clear any existing content so a half-rendered page never coexists
  // with the error notice.
  body.innerHTML = "";
  const host =
    document.querySelector("main[data-zetl-capability]") ??
    document.createElement("main");
  host.setAttribute("data-zetl-capability", "");
  host.setAttribute("data-zetl-error", kind);
  host.textContent = "";

  const p = document.createElement("p");
  p.setAttribute("data-zetl-error-summary", "");
  p.textContent = COPY[kind];
  host.appendChild(p);

  if (detail !== undefined && detail.length > 0) {
    const small = document.createElement("p");
    small.setAttribute("data-zetl-error-detail", "");
    small.style.opacity = "0.7";
    small.textContent = detail;
    host.appendChild(small);
  }

  if (!host.isConnected) body.appendChild(host);
}

export function errorKindFromException(err: unknown): ErrorKind {
  if (err === null || typeof err !== "object") return "internal";
  const name = (err as { name?: string }).name;
  const kind = (err as { kind?: string }).kind;
  if (name === "EnvelopeParseError") return "envelope-malformed";
  if (name === "IdentityError") {
    return kind === "need-invite" ? "need-invite" : "identity-unavailable";
  }
  if (name === "DecryptError") return "decrypt-failed";
  return "internal";
}
