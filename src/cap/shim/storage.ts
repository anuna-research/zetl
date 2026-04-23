// IndexedDB persistence layer for TOFU-wrapped priv_A records
// (CON-3409). The TOFU branch writes one record per (origin,
// cohort_id); the subsequent-visit unwrap branch (companion task
// `task-cap-shim-unwrap`) reads the record, replays the passkey via
// `navigator.credentials.get({ prf: { eval: { first: prfSalt } } })`
// and AES-GCM-decrypts the stored ciphertext back to the 32-byte
// scalar.
//
// The object store is keyed by cohort id so collisions between
// distinct cohorts never overwrite each other. Origin is stored
// alongside for defence-in-depth (an IDB database is already origin-
// scoped, but carrying the origin in the record itself lets the
// AES-GCM AAD check catch a mismatch if the browser's origin-keying
// is ever bypassed).

export const DB_NAME = "ztl-cap-shim";
/// v2 adds `STORE_AUDIT` for REQ-3425 collision-UI audit entries.
/// Existing readers with a v1 DB upgrade in place — `STORE_BINDINGS`
/// rows are preserved; only the new store is created.
export const DB_VERSION = 2;
export const STORE_BINDINGS = "bindings";
/// REQ-3425: local audit log of TOFU-collision resolutions. Entries
/// are append-only and never leave the device; the object store is
/// auto-incremented so bursty collisions with identical `at`
/// timestamps don't overwrite each other.
export const STORE_AUDIT = "collision-audit";

/// Persisted record shape. All `Uint8Array` fields are stored as
/// structured-cloned bytes; IDB preserves the exact contents.
export interface TofuBinding {
  origin: string;
  cohortId: string;
  credentialId: Uint8Array;
  prfSalt: Uint8Array;
  iv: Uint8Array;
  aad: Uint8Array;
  ciphertext: Uint8Array;
  createdAt: number;
}

export class StorageError extends Error {
  override readonly name = "StorageError";
  constructor(
    readonly kind:
      | "unavailable"
      | "open-failed"
      | "transaction-failed"
      | "malformed-record",
    message: string,
  ) {
    super(message);
  }
}

/// Minimal IDBFactory surface the shim uses. Injected so tests can
/// supply a fake-indexeddb instance without touching the real
/// browser API.
export type IdbFactoryLike = IDBFactory;

/// Resolve an IDBFactory — the real `indexedDB` global in the
/// browser, or whatever the caller injects. Returns `null` when the
/// runtime has no IDB at all (very old browsers, certain embed
/// contexts). The TOFU branch treats a missing IDB as a fatal
/// storage error; the unwrap branch treats it as "no binding".
export function defaultFactory(): IdbFactoryLike | null {
  if (typeof indexedDB !== "undefined") return indexedDB;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const g = globalThis as any;
  if (g.indexedDB) return g.indexedDB as IdbFactoryLike;
  return null;
}

/// Open `ztl-cap-shim` DB, upgrading the schema in place to
/// version [`DB_VERSION`]. Subsequent schema bumps append new
/// upgrade branches; the v1 branch is stable.
export async function openDb(
  factory: IdbFactoryLike | null = defaultFactory(),
): Promise<IDBDatabase> {
  if (!factory) {
    throw new StorageError(
      "unavailable",
      "IndexedDB is unavailable in this runtime; the TOFU branch cannot persist a binding",
    );
  }
  return await new Promise<IDBDatabase>((resolve, reject) => {
    const req = factory.open(DB_NAME, DB_VERSION);
    req.onupgradeneeded = () => {
      const db = req.result;
      if (!db.objectStoreNames.contains(STORE_BINDINGS)) {
        db.createObjectStore(STORE_BINDINGS, { keyPath: "cohortId" });
      }
      if (!db.objectStoreNames.contains(STORE_AUDIT)) {
        db.createObjectStore(STORE_AUDIT, { autoIncrement: true });
      }
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () =>
      reject(
        new StorageError(
          "open-failed",
          `IDBFactory.open(${DB_NAME}, ${DB_VERSION}) failed: ${req.error?.message ?? "unknown"}`,
        ),
      );
    req.onblocked = () =>
      reject(
        new StorageError(
          "open-failed",
          `IDBFactory.open(${DB_NAME}, ${DB_VERSION}) blocked — close other tabs holding an older schema`,
        ),
      );
  });
}

/// Read the persisted binding for `cohortId`. Returns `null` when
/// no binding exists. Does NOT decrypt — the unwrap branch owns the
/// passkey challenge + AES-GCM decrypt side.
export async function readBindingRecord(
  cohortId: string,
  factory: IdbFactoryLike | null = defaultFactory(),
): Promise<TofuBinding | null> {
  if (!factory) return null;
  const db = await openDb(factory);
  try {
    return await new Promise<TofuBinding | null>((resolve, reject) => {
      const tx = db.transaction(STORE_BINDINGS, "readonly");
      const store = tx.objectStore(STORE_BINDINGS);
      const req = store.get(cohortId);
      req.onsuccess = () => {
        const got = req.result;
        if (got === undefined) {
          resolve(null);
          return;
        }
        try {
          resolve(validateRecord(got));
        } catch (err) {
          reject(err);
        }
      };
      req.onerror = () =>
        reject(
          new StorageError(
            "transaction-failed",
            `IDB read(${STORE_BINDINGS}, ${cohortId}) failed: ${req.error?.message ?? "unknown"}`,
          ),
        );
    });
  } finally {
    db.close();
  }
}

/// Write or overwrite the persisted binding. The object store is
/// keyed by `cohortId`, so a second call with the same cohort id
/// replaces the record — policy decisions about when that is
/// allowed (e.g. TOFU collision UI per REQ-3425) live in the
/// caller.
export async function writeBindingRecord(
  binding: TofuBinding,
  factory: IdbFactoryLike | null = defaultFactory(),
): Promise<void> {
  const db = await openDb(factory);
  try {
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(STORE_BINDINGS, "readwrite");
      tx.oncomplete = () => resolve();
      tx.onerror = () =>
        reject(
          new StorageError(
            "transaction-failed",
            `IDB write(${STORE_BINDINGS}) tx failed: ${tx.error?.message ?? "unknown"}`,
          ),
        );
      tx.onabort = () =>
        reject(
          new StorageError(
            "transaction-failed",
            `IDB write(${STORE_BINDINGS}) tx aborted: ${tx.error?.message ?? "unknown"}`,
          ),
        );
      tx.objectStore(STORE_BINDINGS).put(binding);
    });
  } finally {
    db.close();
  }
}

/// Delete the binding row for a single cohort. Used by the REQ-3425
/// collision resolver on the `add` / `replace` paths so the
/// subsequent `performTofu` call skips its idempotency short-circuit
/// and writes a fresh row.
export async function deleteBindingRecord(
  cohortId: string,
  factory: IdbFactoryLike | null = defaultFactory(),
): Promise<void> {
  if (!factory) return;
  const db = await openDb(factory);
  try {
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(STORE_BINDINGS, "readwrite");
      tx.oncomplete = () => resolve();
      tx.onerror = () =>
        reject(
          new StorageError(
            "transaction-failed",
            `IDB delete(${STORE_BINDINGS}, ${cohortId}) tx failed: ${tx.error?.message ?? "unknown"}`,
          ),
        );
      tx.onabort = () =>
        reject(
          new StorageError(
            "transaction-failed",
            `IDB delete(${STORE_BINDINGS}, ${cohortId}) tx aborted: ${tx.error?.message ?? "unknown"}`,
          ),
        );
      tx.objectStore(STORE_BINDINGS).delete(cohortId);
    });
  } finally {
    db.close();
  }
}

/// Persisted shape of a REQ-3425 audit-log entry. Mirrors the TS
/// interface in `collision.ts`; kept here so the storage layer can
/// validate on read without creating a circular import.
export interface CollisionAuditRecord {
  at: number;
  origin: string;
  cohortId: string;
  choice: "keep" | "add" | "replace";
  rationale?: string;
  existingBindingCreatedAt: number;
}

/// Append an audit-log entry. The object store auto-increments so
/// two simultaneous collisions with identical `at` timestamps both
/// persist.
export async function appendAuditEntry(
  entry: CollisionAuditRecord,
  factory: IdbFactoryLike | null = defaultFactory(),
): Promise<void> {
  if (!factory) {
    throw new StorageError(
      "unavailable",
      "IndexedDB is unavailable in this runtime; the REQ-3425 collision audit log cannot persist",
    );
  }
  const db = await openDb(factory);
  try {
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(STORE_AUDIT, "readwrite");
      tx.oncomplete = () => resolve();
      tx.onerror = () =>
        reject(
          new StorageError(
            "transaction-failed",
            `IDB write(${STORE_AUDIT}) tx failed: ${tx.error?.message ?? "unknown"}`,
          ),
        );
      tx.onabort = () =>
        reject(
          new StorageError(
            "transaction-failed",
            `IDB write(${STORE_AUDIT}) tx aborted: ${tx.error?.message ?? "unknown"}`,
          ),
        );
      tx.objectStore(STORE_AUDIT).add(entry);
    });
  } finally {
    db.close();
  }
}

/// Read every audit-log entry in insertion order. Exposed for the
/// reader-facing `forgetBinding` UX + local CLI diagnostics; not
/// wired into the hot path.
export async function readAuditLog(
  factory: IdbFactoryLike | null = defaultFactory(),
): Promise<CollisionAuditRecord[]> {
  if (!factory) return [];
  const db = await openDb(factory);
  try {
    return await new Promise<CollisionAuditRecord[]>((resolve, reject) => {
      const tx = db.transaction(STORE_AUDIT, "readonly");
      const store = tx.objectStore(STORE_AUDIT);
      const req = store.getAll();
      req.onsuccess = () => {
        const raw = (req.result ?? []) as unknown[];
        try {
          resolve(raw.map(validateAuditRecord));
        } catch (err) {
          reject(err);
        }
      };
      req.onerror = () =>
        reject(
          new StorageError(
            "transaction-failed",
            `IDB read(${STORE_AUDIT}) failed: ${req.error?.message ?? "unknown"}`,
          ),
        );
    });
  } finally {
    db.close();
  }
}

/// Delete every persisted binding. Used by [`forgetBinding`] in the
/// shim entry point. Implemented via `deleteDatabase` so the entire
/// object-store schema resets — on the next TOFU we start from a
/// fresh upgrade.
export async function clearAllBindings(
  factory: IdbFactoryLike | null = defaultFactory(),
): Promise<void> {
  if (!factory) return;
  await new Promise<void>((resolve, reject) => {
    const req = factory.deleteDatabase(DB_NAME);
    req.onsuccess = () => resolve();
    req.onerror = () =>
      reject(
        new StorageError(
          "transaction-failed",
          `IDB deleteDatabase(${DB_NAME}) failed: ${req.error?.message ?? "unknown"}`,
        ),
      );
    req.onblocked = () =>
      reject(
        new StorageError(
          "transaction-failed",
          `IDB deleteDatabase(${DB_NAME}) blocked — close other tabs on this origin`,
        ),
      );
  });
}

function validateRecord(raw: unknown): TofuBinding {
  if (!raw || typeof raw !== "object") {
    throw new StorageError(
      "malformed-record",
      "IDB binding record is not an object",
    );
  }
  const r = raw as Record<string, unknown>;
  const mustString = (key: string): string => {
    const v = r[key];
    if (typeof v !== "string") {
      throw new StorageError(
        "malformed-record",
        `IDB binding record field ${JSON.stringify(key)} is not a string`,
      );
    }
    return v;
  };
  const mustBytes = (key: string): Uint8Array => {
    const v = r[key];
    if (v instanceof Uint8Array) return v;
    if (v instanceof ArrayBuffer) return new Uint8Array(v);
    throw new StorageError(
      "malformed-record",
      `IDB binding record field ${JSON.stringify(key)} is not Uint8Array/ArrayBuffer`,
    );
  };
  const mustNumber = (key: string): number => {
    const v = r[key];
    if (typeof v !== "number" || !Number.isFinite(v)) {
      throw new StorageError(
        "malformed-record",
        `IDB binding record field ${JSON.stringify(key)} is not a finite number`,
      );
    }
    return v;
  };
  return {
    origin: mustString("origin"),
    cohortId: mustString("cohortId"),
    credentialId: mustBytes("credentialId"),
    prfSalt: mustBytes("prfSalt"),
    iv: mustBytes("iv"),
    aad: mustBytes("aad"),
    ciphertext: mustBytes("ciphertext"),
    createdAt: mustNumber("createdAt"),
  };
}

function validateAuditRecord(raw: unknown): CollisionAuditRecord {
  if (!raw || typeof raw !== "object") {
    throw new StorageError(
      "malformed-record",
      "IDB collision-audit record is not an object",
    );
  }
  const r = raw as Record<string, unknown>;
  const at = r["at"];
  if (typeof at !== "number" || !Number.isFinite(at)) {
    throw new StorageError("malformed-record", "audit entry .at is not a finite number");
  }
  const origin = r["origin"];
  if (typeof origin !== "string") {
    throw new StorageError("malformed-record", "audit entry .origin is not a string");
  }
  const cohortId = r["cohortId"];
  if (typeof cohortId !== "string") {
    throw new StorageError("malformed-record", "audit entry .cohortId is not a string");
  }
  const choice = r["choice"];
  if (choice !== "keep" && choice !== "add" && choice !== "replace") {
    throw new StorageError(
      "malformed-record",
      `audit entry .choice is ${JSON.stringify(choice)}; expected "keep"|"add"|"replace"`,
    );
  }
  const createdAt = r["existingBindingCreatedAt"];
  if (typeof createdAt !== "number" || !Number.isFinite(createdAt)) {
    throw new StorageError(
      "malformed-record",
      "audit entry .existingBindingCreatedAt is not a finite number",
    );
  }
  const out: CollisionAuditRecord = {
    at,
    origin,
    cohortId,
    choice,
    existingBindingCreatedAt: createdAt,
  };
  const rationale = r["rationale"];
  if (rationale !== undefined) {
    if (typeof rationale !== "string") {
      throw new StorageError(
        "malformed-record",
        "audit entry .rationale is not a string",
      );
    }
    out.rationale = rationale;
  }
  return out;
}
