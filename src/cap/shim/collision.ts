// REQ-3425 TOFU collision UI. When the shim detects a `#k=` or
// `#k1=` invite fragment arriving at a page whose cohortId already
// has a persisted IDB binding, the reader is prompted to resolve
// the collision rather than the TOFU flow silently overwriting or
// silently short-circuiting. Three outcomes, rendered in the
// wireframe pinned by SPEC-034 §REQ-3425:
//
//     ┌─────────────────────────────────────────────┐
//     │  ⚠  Existing wiki access detected           │
//     │  [ KEEP existing access (recommended) ]     │
//     │  [ Add new invite alongside ]               │
//     │  [ Replace (advanced — why? _________) ]    │
//     └─────────────────────────────────────────────┘
//
// Acceptance criteria (§REQ-3425):
//   * Default focus on KEEP
//   * `Replace` requires a 1-line free-text rationale, persisted
//     locally in a never-leaves-the-device audit log
//   * No silent overwrites
//
// Storage semantics in v1: at the IndexedDB layer the object store
// is keyed by cohortId, so `add` and `replace` both overwrite the
// single binding row. The audit log distinguishes operator intent
// — "I wanted to keep both" vs. "I deliberately retired the old
// one" — and a future task can promote the schema to support true
// multi-binding coexistence. `keep` is a pure no-op: the existing
// binding is preserved and nothing is audited (nothing was
// overwritten).
//
// This module exposes the decision surface + a DOM-based default
// prompt. The identity dispatcher (identity.ts) calls
// `resolveCollision` before running TOFU on a first-visit with an
// existing binding; the pipeline (pipeline.ts) threads the prompt
// callback through from the shim entry point (index.ts) which
// resolves the production default (DOM renderer).

import {
  appendAuditEntry,
  deleteBindingRecord,
  type CollisionAuditRecord,
  type IdbFactoryLike,
  type TofuBinding,
} from "./storage.ts";

/// The three outcomes the wireframe exposes. The storage layer in
/// v1 treats `add` and `replace` identically at the binding-row
/// level; the audit log is where the distinction lives. See module
/// header for the rationale.
export type CollisionChoice = "keep" | "add" | "replace";

/// Free-text rationale size cap. Anything longer is rejected by
/// `validateDecision` so the audit log stays bounded. The wireframe
/// advertises a single-line input so 200 chars comfortably fits
/// every sensible explanation.
export const MAX_RATIONALE_LEN = 200;

/// Read-only view the prompt callback receives. Origin + cohortId
/// let a DOM renderer show the "for cohort: <name>" line from the
/// wireframe; `existingBindingCreatedAt` lets a more elaborate
/// prompt surface "you first enrolled this device on <date>" as
/// extra context.
export interface CollisionContext {
  cohortId: string;
  origin: string;
  existingBindingCreatedAt: number;
}

export type CollisionDecision =
  | { choice: "keep" }
  | { choice: "add" }
  | { choice: "replace"; rationale: string };

/// Async callback the identity dispatcher invokes. Production
/// wiring uses `renderCollisionPrompt` (DOM); tests inject a stub
/// that returns the decision directly.
export type CollisionPrompt = (
  ctx: CollisionContext,
) => Promise<CollisionDecision>;

export class CollisionError extends Error {
  override readonly name = "CollisionError";
  constructor(
    readonly kind: "rationale-required" | "rationale-too-long",
    message: string,
  ) {
    super(message);
  }
}

/// Normalise + validate a decision. `replace` requires a non-empty
/// rationale ≤ `MAX_RATIONALE_LEN` chars; whitespace is trimmed
/// before length checks so a reader who accidentally typed a space
/// + Enter is rejected, but a reader whose spoken-phrase prompt
/// accumulated newlines around the payload is accepted.
export function validateDecision(decision: CollisionDecision): CollisionDecision {
  if (decision.choice === "replace") {
    const trimmed = decision.rationale.trim();
    if (trimmed.length === 0) {
      throw new CollisionError(
        "rationale-required",
        "Replace requires a one-line rationale (SPEC-034 REQ-3425 acceptance criterion)",
      );
    }
    if (trimmed.length > MAX_RATIONALE_LEN) {
      throw new CollisionError(
        "rationale-too-long",
        `rationale is ${trimmed.length} chars; cap is ${MAX_RATIONALE_LEN}`,
      );
    }
    return { choice: "replace", rationale: trimmed };
  }
  return decision;
}

export interface ResolveDeps {
  /// `Date.now` by default; tests inject a deterministic clock.
  now?: () => number;
  /// IDB factory — forwarded to the audit-log write and binding
  /// delete. Production leaves this undefined; the shim entry point
  /// resolves to the live `indexedDB`.
  idbFactory?: IdbFactoryLike | null;
}

export interface ResolveOutcome {
  /// The decision the reader made, post-validation.
  decision: CollisionDecision;
  /// `true` when the caller should proceed with the TOFU wrap
  /// (`add`/`replace`), `false` when it should skip (`keep`). The
  /// caller uses this to decide whether to invoke `performTofu`.
  shouldWrap: boolean;
}

/// Run the collision resolution. Writes the audit entry, clears
/// the stale binding on `add`/`replace`, and returns whether the
/// caller should proceed with the TOFU wrap. `keep` short-circuits
/// with no IDB mutation and no audit entry.
export async function resolveCollision(
  existing: TofuBinding,
  prompt: CollisionPrompt,
  deps: ResolveDeps = {},
): Promise<ResolveOutcome> {
  const ctx: CollisionContext = {
    cohortId: existing.cohortId,
    origin: existing.origin,
    existingBindingCreatedAt: existing.createdAt,
  };
  const raw = await prompt(ctx);
  const decision = validateDecision(raw);

  if (decision.choice === "keep") {
    return { decision, shouldWrap: false };
  }

  const now = deps.now ?? (() => Date.now());
  const entry: CollisionAuditRecord = {
    at: now(),
    origin: existing.origin,
    cohortId: existing.cohortId,
    choice: decision.choice,
    existingBindingCreatedAt: existing.createdAt,
  };
  if (decision.choice === "replace") {
    entry.rationale = decision.rationale;
  }
  await appendAuditEntry(entry, deps.idbFactory);
  // Clear the stale row so the downstream `performTofu` call skips
  // its idempotency short-circuit and writes a fresh binding.
  await deleteBindingRecord(existing.cohortId, deps.idbFactory);
  return { decision, shouldWrap: true };
}

// ── DOM default prompt ────────────────────────────────────────────────

/// Host selector the default renderer mounts under. Matches the
/// same capability-mode host the decrypted content later renders
/// into, so screen readers see a single landmark region per page.
const HOST_SELECTOR = "main[data-zetl-capability]";

/// Rendered copy — exported so the test suite can assert byte-stable
/// matches and the operator-facing docs can mirror the wording.
export const COLLISION_TITLE = "Existing wiki access detected";
export const COLLISION_BODY_INTRO =
  "You already have access to this wiki on this device via a previous invite.";
export const COLLISION_BODY_COHORT_PREFIX =
  "This new invite URL (for cohort: ";
export const COLLISION_BODY_COHORT_SUFFIX =
  ") would either add to or replace your current access.";
export const COLLISION_DEFAULT_NOTE = "Default: KEEP your current access.";
export const COLLISION_BUTTON_KEEP = "KEEP existing access (recommended)";
export const COLLISION_BUTTON_ADD = "Add new invite alongside";
export const COLLISION_BUTTON_REPLACE = "Replace";
export const COLLISION_RATIONALE_LABEL =
  "Rationale (required for Replace — 1 line)";
export const COLLISION_FOOTER =
  "If you didn't expect this message, close the tab and contact your wiki operator.";
export const COLLISION_RATIONALE_MISSING_HINT =
  "Please give a one-line reason before choosing Replace.";
export const COLLISION_RATIONALE_TOO_LONG_HINT =
  `Please shorten the rationale to ${MAX_RATIONALE_LEN} characters or fewer.`;

/// Mount the REQ-3425 wireframe into the capability host and
/// resolve with the reader's decision. Default focus lands on the
/// KEEP button (acceptance criterion #1). A reader who picks
/// Replace without entering a rationale is re-prompted in-place;
/// the promise does not resolve until a valid decision is made —
/// there is no "cancel" affordance (the wireframe instructs the
/// reader to close the tab instead).
export function renderCollisionPrompt(
  ctx: CollisionContext,
  doc: Document = document,
): Promise<CollisionDecision> {
  return new Promise<CollisionDecision>((resolve) => {
    const host =
      doc.querySelector(HOST_SELECTOR) ??
      doc.body ??
      doc.documentElement;
    if (host === null) {
      throw new Error(
        `renderCollisionPrompt: no host element (${HOST_SELECTOR} / body / documentElement) available`,
      );
    }

    // Clear any prior content so a decrypted page never coexists
    // with the prompt — collision UI must precede decrypt (the
    // storage write we block on can change which priv_A the
    // subsequent-visit unwrap recovers).
    while (host.firstChild) host.removeChild(host.firstChild);

    const panel = doc.createElement("section");
    panel.setAttribute("data-zetl-collision", "");
    panel.setAttribute("role", "alertdialog");
    panel.setAttribute("aria-labelledby", "zetl-collision-title");
    panel.setAttribute("aria-describedby", "zetl-collision-body");
    host.appendChild(panel);

    const title = doc.createElement("h1");
    title.id = "zetl-collision-title";
    title.setAttribute("data-zetl-collision-title", "");
    title.textContent = `\u26a0\ufe0f  ${COLLISION_TITLE}`;
    panel.appendChild(title);

    const body = doc.createElement("div");
    body.id = "zetl-collision-body";
    body.setAttribute("data-zetl-collision-body", "");
    const p1 = doc.createElement("p");
    p1.textContent = COLLISION_BODY_INTRO;
    body.appendChild(p1);
    const p2 = doc.createElement("p");
    p2.textContent =
      COLLISION_BODY_COHORT_PREFIX +
      ctx.cohortId +
      COLLISION_BODY_COHORT_SUFFIX;
    body.appendChild(p2);
    const p3 = doc.createElement("p");
    p3.setAttribute("data-zetl-collision-default", "");
    p3.textContent = COLLISION_DEFAULT_NOTE;
    body.appendChild(p3);
    panel.appendChild(body);

    const err = doc.createElement("p");
    err.setAttribute("data-zetl-collision-error", "");
    err.setAttribute("role", "alert");
    err.setAttribute("aria-live", "polite");
    err.hidden = true;
    panel.appendChild(err);

    const showError = (msg: string) => {
      err.hidden = false;
      err.textContent = msg;
    };
    const clearError = () => {
      err.hidden = true;
      err.textContent = "";
    };

    const keepBtn = doc.createElement("button");
    keepBtn.type = "button";
    keepBtn.setAttribute("data-zetl-collision-choice", "keep");
    keepBtn.textContent = COLLISION_BUTTON_KEEP;
    panel.appendChild(keepBtn);

    const addBtn = doc.createElement("button");
    addBtn.type = "button";
    addBtn.setAttribute("data-zetl-collision-choice", "add");
    addBtn.textContent = COLLISION_BUTTON_ADD;
    panel.appendChild(addBtn);

    const replaceWrap = doc.createElement("div");
    replaceWrap.setAttribute("data-zetl-collision-replace", "");
    const label = doc.createElement("label");
    label.setAttribute("for", "zetl-collision-rationale");
    label.textContent = COLLISION_RATIONALE_LABEL;
    replaceWrap.appendChild(label);
    const rationale = doc.createElement("input");
    rationale.id = "zetl-collision-rationale";
    rationale.setAttribute("type", "text");
    rationale.setAttribute("data-zetl-collision-rationale", "");
    rationale.maxLength = MAX_RATIONALE_LEN;
    replaceWrap.appendChild(rationale);
    const replaceBtn = doc.createElement("button");
    replaceBtn.type = "button";
    replaceBtn.setAttribute("data-zetl-collision-choice", "replace");
    replaceBtn.textContent = COLLISION_BUTTON_REPLACE;
    replaceWrap.appendChild(replaceBtn);
    panel.appendChild(replaceWrap);

    const footer = doc.createElement("p");
    footer.setAttribute("data-zetl-collision-footer", "");
    footer.textContent = COLLISION_FOOTER;
    panel.appendChild(footer);

    const detach = () => {
      keepBtn.removeEventListener("click", onKeep);
      addBtn.removeEventListener("click", onAdd);
      replaceBtn.removeEventListener("click", onReplace);
    };

    const onKeep = () => {
      detach();
      resolve({ choice: "keep" });
    };
    const onAdd = () => {
      detach();
      resolve({ choice: "add" });
    };
    const onReplace = () => {
      const raw = rationale.value;
      const trimmed = raw.trim();
      if (trimmed.length === 0) {
        showError(COLLISION_RATIONALE_MISSING_HINT);
        rationale.focus();
        return;
      }
      if (trimmed.length > MAX_RATIONALE_LEN) {
        showError(COLLISION_RATIONALE_TOO_LONG_HINT);
        rationale.focus();
        return;
      }
      clearError();
      detach();
      resolve({ choice: "replace", rationale: trimmed });
    };

    keepBtn.addEventListener("click", onKeep);
    addBtn.addEventListener("click", onAdd);
    replaceBtn.addEventListener("click", onReplace);

    // REQ-3425 acceptance criterion #1: default focus lands on KEEP.
    // `autofocus` alone is unreliable post-dynamic-mount; an explicit
    // `.focus()` after appendChild is the durable fix.
    try {
      keepBtn.focus();
    } catch {
      // happy-dom focus can throw on detached roots in some
      // fixtures — swallow so the promise still waits for a click.
    }
  });
}
