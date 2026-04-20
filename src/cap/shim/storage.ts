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

export const DB_NAME = "zetl-cap-shim";
export const DB_VERSION = 1;
export const STORE_BINDINGS = "bindings";

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

/// Open `zetl-cap-shim` DB, upgrading the schema in place to
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
