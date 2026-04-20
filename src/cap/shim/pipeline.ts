// CON-3408 pipeline state machine. The security property is that each
// phase may only advance in the documented order:
//
//   Init → LockAcquired → SwPurged → EnvelopeFetched → SignatureVerified
//     → IdentityAcquired → Decrypted → Sanitised → Scrubbed → Rendered
//
// A failure at any phase transitions to `Errored` (terminal) and MUST
// NOT touch later phases. In particular: signature-verification failure
// MUST NOT call `credentials.get/create` (REQ-3427 acceptance), and a
// missing-identity error MUST NOT attempt decryption.

import { parseEnvelope, type ParsedEnvelope } from "./envelope.ts";
import { verifyEd25519 } from "./signature.ts";
import {
  acquireIdentity,
  IdentityError,
  type SplitKeyPrompt,
  type SplitKeySecondFactor,
} from "./identity.ts";
import { ageDecrypt } from "./decrypt.ts";
import { sanitiseHtml } from "./sanitise.ts";
import { renderInto, rewriteWikiHrefs, scrubFragment } from "./render.ts";
import { errorKindFromException, renderError, type ErrorKind } from "./errors.ts";
import {
  detectPrfAvailability,
  emitFallbackMark,
  renderFallbackNotice,
  type PrfAvailability,
} from "./fallback.ts";
import type { IdbFactoryLike } from "./storage.ts";
import type { TofuDeps } from "./tofu.ts";

export const LOCK_NAME = "zetl-capability-shim";

export const Phase = {
  Init: 0,
  LockAcquired: 1,
  SwPurged: 2,
  EnvelopeFetched: 3,
  SignatureVerified: 4,
  IdentityAcquired: 5,
  Decrypted: 6,
  Sanitised: 7,
  Scrubbed: 8,
  Rendered: 9,
  Errored: 99,
} as const;
export type Phase = (typeof Phase)[keyof typeof Phase];

/// Captured transition log — exposed so tests can assert "pipeline
/// stopped at SignatureVerified, never reached IdentityAcquired".
export interface PipelineTrace {
  phases: Phase[];
  errorKind?: ErrorKind;
  errorDetail?: string;
  /// Set when REQ-3412 fragment-required fallback activated: PRF
  /// unavailable → TOFU skipped + fragment scrub skipped + banner
  /// rendered. Unset on the happy path so tests can assert the
  /// default experience is untouched.
  fallbackActive?: boolean;
  /// The probe's reason tag — present when `fallbackActive` is
  /// true. Lets tests assert the exact branch (missing credential
  /// API vs. caps.prf=false vs. no-create).
  fallbackReason?: PrfAvailability["reason"];
}

export interface PipelineDeps {
  vaultSigningPubkey: Uint8Array;
  /// Fetcher for the envelope bytes. Injected so tests don't need a
  /// real network.
  fetchEnvelope: () => Promise<Uint8Array>;
  /// Service-worker purge. Injected so tests can stub it.
  purgeServiceWorkers: () => Promise<void>;
  /// Lock acquisition. Injected so tests can run without navigator.locks
  /// and so the TOFU/unwrap tasks can wrap the critical section in a
  /// `BroadcastChannel` fallback per REQ-3429.
  withLock: <T>(body: () => Promise<T>) => Promise<T>;
  locationHash: string;
  /// Origin the shim binds TOFU records to. Defaults to
  /// `location.origin` when omitted; tests override to decouple
  /// from happy-dom defaults.
  origin?: string;
  /// Dep overrides threaded into the TOFU flow. Production leaves
  /// these undefined and the tofu module resolves the browser
  /// built-ins itself; tests inject fakes for
  /// `navigator.credentials.create`, `crypto.subtle`, RNG, and
  /// clock.
  tofuDeps?: Partial<TofuDeps>;
  /// Override the IDB factory (fake-indexeddb in the test suite).
  idbFactory?: IdbFactoryLike | null;
  /// PRF-availability probe. Defaults to
  /// `fallback.ts::detectPrfAvailability`. Tests inject a
  /// deterministic stub so happy-dom doesn't need a real
  /// `PublicKeyCredential` surface.
  probePrf?: () => Promise<PrfAvailability>;
  /// Override for REQ-3412 OBS-3412 mark emission. Defaults to the
  /// live `performance.mark` path; tests pass a capturing spy.
  emitFallbackMark?: (reason?: PrfAvailability["reason"]) => void;
  /// REQ-3430 split-key: callback that prompts the reader for
  /// `half2`. Required when a `#k1=<half1>` fragment is present;
  /// omitted when the build wasn't compiled with split-key support
  /// (in which case such a fragment surfaces an
  /// `IdentityError("mode-not-supported")`).
  promptHalf2?: SplitKeyPrompt;
  /// `[access.split_key] second_factor` — injected at build time
  /// from the operator's config so the shim knows which prompt
  /// widget to render.
  splitKeySecondFactor?: SplitKeySecondFactor;
}

export async function runPipeline(deps: PipelineDeps): Promise<PipelineTrace> {
  const trace: PipelineTrace = { phases: [Phase.Init] };
  try {
    return await deps.withLock(async () => {
      trace.phases.push(Phase.LockAcquired);

      await deps.purgeServiceWorkers();
      trace.phases.push(Phase.SwPurged);

      const envelopeBytes = await deps.fetchEnvelope();
      const envelope: ParsedEnvelope = parseEnvelope(envelopeBytes);
      trace.phases.push(Phase.EnvelopeFetched);

      const sigOk = await verifyEd25519(
        deps.vaultSigningPubkey,
        envelope.ciphertext,
        envelope.signature,
      );
      if (!sigOk) {
        // Terminal. CON-3408 forbids any further cryptographic work,
        // including `credentials.get/create` — which is why we throw
        // BEFORE calling `acquireIdentity`.
        throw new SignatureVerifyError(
          "Ed25519 signature did not verify against the embedded vault-signing pubkey",
        );
      }
      trace.phases.push(Phase.SignatureVerified);

      // REQ-3412 probe — runs before acquireIdentity so the
      // decision to skip TOFU (idbFactory=null) is taken once, at
      // a single point, rather than relying on downstream APIs
      // happening to be absent. The probe does not call
      // credentials.get/create — it only inspects capabilities
      // and feature-probes the surface — so it's safe to run
      // before any cryptographic work beyond signature verify.
      const probe = deps.probePrf ?? detectPrfAvailability;
      const prfProbe = await probe();
      const fallbackActive = !prfProbe.available;
      trace.fallbackActive = fallbackActive;
      trace.fallbackReason = prfProbe.reason;
      if (fallbackActive) {
        (deps.emitFallbackMark ?? emitFallbackMark)(prfProbe.reason);
      }

      const priv = await acquireIdentity({
        cohortId: envelope.header.cohortId,
        cohortMode: envelope.header.cohortMode,
        locationHash: deps.locationHash,
        origin: deps.origin,
        tofuDeps: deps.tofuDeps,
        // Fragment-required fallback: force TOFU off even when
        // the other surfaces exist (e.g. a browser that has
        // `navigator.credentials.create` and `crypto.subtle` but
        // does not implement the PRF extension).
        idbFactory: fallbackActive ? null : deps.idbFactory,
        promptHalf2: deps.promptHalf2,
        splitKeySecondFactor: deps.splitKeySecondFactor,
      });
      trace.phases.push(Phase.IdentityAcquired);

      const plaintextBytes = await ageDecrypt(envelope.ciphertext, priv);
      const plaintext = new TextDecoder("utf-8").decode(plaintextBytes);
      trace.phases.push(Phase.Decrypted);

      const sanitised = sanitiseHtml(plaintext);
      trace.phases.push(Phase.Sanitised);

      // REQ-3412: in fallback mode the fragment stays in the URL so
      // the reader can bookmark / reload without losing decrypt
      // capability. Wikilink rewrite still runs — build-time intra-
      // wiki hrefs are fragmentless so this is effectively a no-op,
      // and keeping it on preserves the §11.2 leak-hygiene property
      // for any content that somehow did embed one.
      if (!fallbackActive) {
        scrubFragment();
      }
      rewriteWikiHrefs();
      trace.phases.push(Phase.Scrubbed);

      const host = renderInto(sanitised);
      if (fallbackActive) {
        renderFallbackNotice(host, prfProbe.reason);
      }
      trace.phases.push(Phase.Rendered);

      return trace;
    });
  } catch (err) {
    trace.phases.push(Phase.Errored);
    const kind = errorKindFrom(err);
    trace.errorKind = kind;
    trace.errorDetail = err instanceof Error ? err.message : String(err);
    renderError(kind, trace.errorDetail);
    return trace;
  }
}

export class SignatureVerifyError extends Error {
  override readonly name = "SignatureVerifyError";
}

function errorKindFrom(err: unknown): ErrorKind {
  if (err instanceof SignatureVerifyError) return "signature-failed";
  if (err instanceof IdentityError) {
    if (err.kind === "need-invite") return "need-invite";
    if (err.kind === "tofu-failed") return "tofu-failed";
    if (err.kind === "split-key-cancelled") return "need-invite";
    if (err.kind === "split-key-invalid-half2") return "need-invite";
    return "identity-unavailable";
  }
  return errorKindFromException(err);
}

// ── Default dep implementations (browser) ──────────────────────────────

export async function defaultPurgeServiceWorkers(): Promise<void> {
  if (typeof navigator === "undefined" || !("serviceWorker" in navigator)) {
    return;
  }
  const regs = await navigator.serviceWorker.getRegistrations();
  await Promise.all(regs.map((r) => r.unregister()));
}

export async function defaultFetchEnvelope(): Promise<Uint8Array> {
  const resp = await fetch(location.pathname, {
    credentials: "omit",
    redirect: "error",
    cache: "default",
  });
  if (!resp.ok) {
    throw new Error(
      `envelope fetch for ${location.pathname} returned HTTP ${resp.status} ${resp.statusText}`,
    );
  }
  const buf = await resp.arrayBuffer();
  return new Uint8Array(buf);
}

/// `navigator.locks` wrapper that throws a structured error when the API
/// is unavailable. REQ-3429 allows a BroadcastChannel fallback at
/// operator-configurable discretion; v1 core takes the "refuse" branch
/// so the downstream TOFU task can decide policy.
export function defaultWithLock<T>(body: () => Promise<T>): Promise<T> {
  if (typeof navigator === "undefined" || !("locks" in navigator)) {
    return Promise.reject(new LockUnavailableError());
  }
  return navigator.locks.request(
    LOCK_NAME,
    { mode: "exclusive" },
    body,
  ) as Promise<T>;
}

export class LockUnavailableError extends Error {
  override readonly name = "LockUnavailableError";
  constructor() {
    super("navigator.locks is unavailable on this browser");
  }
}
