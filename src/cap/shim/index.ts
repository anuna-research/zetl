// Capability-mode browser shim — CON-3408 entry point.
//
// The bundled build (esbuild) replaces `__VAULT_SIGNING_PUBKEY_B64URL__`
// with the operator's Ed25519 vault-signing public key encoded as
// base64url. The replacement is string-literal, so the SHA-384 SRI hash
// over the emitted `shim.js` covers the pubkey: a CDN that tampers with
// the pubkey invalidates SRI (REQ-3427).

import {
  renderCollisionPrompt,
  type CollisionPrompt,
} from "./collision.ts";
import { base64UrlDecode } from "./envelope.ts";
import { renderError } from "./errors.ts";
import type { SplitKeyPrompt, SplitKeySecondFactor } from "./identity.ts";
import {
  defaultFetchEnvelope,
  defaultPurgeServiceWorkers,
  defaultWithLock,
  LockUnavailableError,
  runPipeline,
  type PipelineDeps,
  type PipelineTrace,
} from "./pipeline.ts";
import { clearAllBindings } from "./storage.ts";

// `declare const` + esbuild `define` swaps this for the literal b64url
// pubkey string at build time. Keeping it a plain const means the
// emitted bundle has a single exact byte range to audit.
declare const __VAULT_SIGNING_PUBKEY_B64URL__: string;
/// REQ-3430 opt-in. `esbuild --define` swaps this for the literal
/// value of `[access.split_key] second_factor` at build time when
/// `enabled = true`, and for `""` otherwise. Empty-string means "no
/// split-key support in this bundle" — a reader arriving on a
/// `#k1=` URL will see a `mode-not-supported` diagnostic.
declare const __SPLIT_KEY_SECOND_FACTOR__: string;

export interface ShimOptions {
  /// Override dependency injection points — used by the Playwright
  /// pipeline-order tests.
  deps?: Partial<PipelineDeps>;
}

export async function renderCurrentPage(
  opts: ShimOptions = {},
): Promise<PipelineTrace> {
  let vaultSigningPubkey: Uint8Array;
  try {
    vaultSigningPubkey = base64UrlDecode(__VAULT_SIGNING_PUBKEY_B64URL__);
  } catch (err) {
    renderError(
      "internal",
      `embedded vault-signing pubkey is not valid base64url: ${(err as Error).message}`,
    );
    return { phases: [], errorKind: "internal", errorDetail: (err as Error).message };
  }
  if (vaultSigningPubkey.length !== 32) {
    renderError(
      "internal",
      `embedded vault-signing pubkey is ${vaultSigningPubkey.length} bytes; expected 32`,
    );
    return { phases: [], errorKind: "internal" };
  }

  const splitKeyFactor = resolveSplitKeyFactor();
  const deps: PipelineDeps = {
    vaultSigningPubkey,
    fetchEnvelope: opts.deps?.fetchEnvelope ?? defaultFetchEnvelope,
    purgeServiceWorkers:
      opts.deps?.purgeServiceWorkers ?? defaultPurgeServiceWorkers,
    withLock: opts.deps?.withLock ?? defaultWithLock,
    locationHash: opts.deps?.locationHash ?? location.hash,
    splitKeySecondFactor:
      opts.deps?.splitKeySecondFactor ?? splitKeyFactor ?? undefined,
    promptHalf2:
      opts.deps?.promptHalf2
      ?? (splitKeyFactor !== null ? defaultPromptHalf2 : undefined),
    promptCollision: opts.deps?.promptCollision ?? defaultPromptCollision,
  };

  try {
    return await runPipeline(deps);
  } catch (err) {
    if (err instanceof LockUnavailableError) {
      renderError("lock-unavailable", err.message);
      return { phases: [], errorKind: "lock-unavailable", errorDetail: err.message };
    }
    throw err;
  }
}

/// Mirror of CON-3408's `forgetBinding()` surface. Deletes every
/// TOFU-wrapped priv_A record persisted by the shim on this origin.
/// After calling this the reader needs a fresh invite URL to
/// re-bind their device.
export async function forgetBinding(): Promise<void> {
  await clearAllBindings();
}

/// Read the esbuild-substituted split-key factor literal. Returns
/// `null` when the operator has not opted in (`__SPLIT_KEY_..._$$ = ""`)
/// OR when the literal was not replaced (undefined at runtime in
/// non-bundled test harnesses). A non-empty string must be one of
/// `"spoken-phrase"` / `"qr"` — anything else is a build misconfig
/// and falls back to null with a console warning so the reader
/// doesn't get a broken prompt.
function resolveSplitKeyFactor(): SplitKeySecondFactor | null {
  const raw =
    typeof __SPLIT_KEY_SECOND_FACTOR__ !== "undefined"
      ? __SPLIT_KEY_SECOND_FACTOR__
      : "";
  if (raw === "") return null;
  if (raw === "spoken-phrase" || raw === "qr") return raw;
  if (typeof console !== "undefined" && typeof console.warn === "function") {
    console.warn(
      `zetl: unknown [access.split_key] second_factor ${JSON.stringify(raw)}; disabling split-key support for this bundle`,
    );
  }
  return null;
}

/// REQ-3425 default collision prompt — mounts the DOM wireframe
/// under the capability-mode host and resolves with the reader's
/// choice. Tests inject a stub `promptCollision` via `opts.deps`
/// so they never hit this branch.
const defaultPromptCollision: CollisionPrompt = (ctx) =>
  renderCollisionPrompt(ctx);

/// Default half2 collector. For `spoken-phrase` we pop a blocking
/// `window.prompt`. `qr` is a TODO — the camera scanner UI is a
/// follow-up task; v1 surfaces a "QR transport not yet implemented"
/// diagnostic so the reader learns to ask the operator for a
/// spoken-phrase invite instead of silently hanging. Tests inject a
/// custom `promptHalf2` so they never hit this branch.
const defaultPromptHalf2: SplitKeyPrompt = async (factor) => {
  if (factor === "spoken-phrase") {
    if (typeof window === "undefined" || typeof window.prompt !== "function") {
      throw new Error(
        "spoken-phrase factor requires a browser with window.prompt",
      );
    }
    const entered = window.prompt(
      "Enter the second-factor phrase you received on a separate channel (REQ-3430):",
    );
    if (entered === null) {
      throw new Error("reader cancelled the second-factor prompt");
    }
    return entered;
  }
  throw new Error(
    "qr second-factor transport is not yet wired in this shim build; ask your wiki operator for a spoken-phrase invite",
  );
};

declare global {
  interface Window {
    __zetlCapShim?: {
      renderCurrentPage: typeof renderCurrentPage;
      forgetBinding: typeof forgetBinding;
    };
  }
}

if (typeof window !== "undefined") {
  window.__zetlCapShim = { renderCurrentPage, forgetBinding };
  const start = () => {
    void renderCurrentPage();
  };
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", start, { once: true });
  } else {
    start();
  }
}
