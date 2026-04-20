// Error surface — user-visible diagnostics for the capability-mode
// host. Error copy is pinned by REQ-3427 ("This page's signature did not
// verify…") so byte-stable tests can assert the exact wording.

export type ErrorKind =
  | "signature-failed"
  | "need-invite"
  | "identity-unavailable"
  | "tofu-failed"
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
  "tofu-failed":
    "Could not bind this device to the cohort passkey — TOFU registration failed. Reload to retry; if the problem persists, your browser or authenticator may not support the WebAuthn PRF extension.",
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

/// Base URL of the reader-facing onboarding + troubleshooting page.
/// `renderError` appends `#err-<kind>` to deep-link to the matching
/// section. Operators can override the base at bundle time via the
/// esbuild `define` substitution, in case they host the doc under a
/// different path than the default `dist/reader.html` location. The
/// default is a root-relative path so the link resolves against the
/// same origin the shim is running on.
export const READER_DOC_BASE: string = "/reader.html";

/// Build the deep-link URL pointing into `reader.html` for a given
/// error kind. Exported so tests can assert the exact href.
export function helpHrefFor(kind: ErrorKind): string {
  return `${READER_DOC_BASE}#err-${kind}`;
}

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

  // Deep-link to the matching section of the reader-facing
  // troubleshooting doc. The summary element above is kept textContent-
  // only so REQ-3427's byte-stable copy assertion stays green; this
  // link is a separate sibling the Playwright suite can query by the
  // `data-zetl-error-help` attribute without affecting the summary.
  const help = document.createElement("p");
  help.setAttribute("data-zetl-error-help", "");
  const a = document.createElement("a");
  a.setAttribute("href", helpHrefFor(kind));
  // Belt-and-braces: the enclosing shell already sets a document-level
  // `no-referrer` policy (html_shell::REFERRER_META), but readers who
  // open this link in another tab via middle-click benefit from the
  // explicit rel here too.
  a.setAttribute("rel", "noopener noreferrer");
  a.textContent = "Troubleshooting";
  help.appendChild(a);
  host.appendChild(help);

  if (!host.isConnected) body.appendChild(host);
}

export function errorKindFromException(err: unknown): ErrorKind {
  if (err === null || typeof err !== "object") return "internal";
  const name = (err as { name?: string }).name;
  const kind = (err as { kind?: string }).kind;
  if (name === "EnvelopeParseError") return "envelope-malformed";
  if (name === "IdentityError") {
    if (kind === "need-invite") return "need-invite";
    if (kind === "tofu-failed") return "tofu-failed";
    return "identity-unavailable";
  }
  if (name === "DecryptError") return "decrypt-failed";
  return "internal";
}
