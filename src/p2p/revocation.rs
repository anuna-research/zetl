//! Revocation + durable epoch rotation (SPEC-047 REQ-481/498/506, IMPL-047 T13).
//!
//! **UNREVIEWED AUTH-CORE** — see the banner in [`super`]. Pending Q1/Q2/Q7/Q11.
//!
//! Revocation itself is the Owner's MLS Remove commit (T9 `remove_member`,
//! which advances the group epoch so the removed leaf can no longer derive the
//! key). This module adds the **durability** REQ-506 requires: the rotation is
//! recorded durably *before* the rotating daemon advances its own epoch, and an
//! interrupted rotation is completed **exactly once** on recovery — so a crash
//! mid-rotation never strands surviving peers behind the new epoch. Modelled on
//! `../elephant-3000`'s transactional outbox (its SPEC-004 REQ-306 /
//! `recover_outbox`): write the commit, fsync, *then* merge the local epoch;
//! recovery republishes the recorded bytes and completes at most once, guarded
//! by the recorded pre-rotation epoch.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A pending rotation, written durably before the Owner advances its epoch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RotationOutbox {
    /// The removal (Remove-commit) bytes the surviving peers must receive.
    #[serde(with = "b64")]
    pub commit: Vec<u8>,
    /// The group epoch *before* this rotation. Recovery uses it to complete the
    /// rotation at most once: if the live epoch already advanced past it, the
    /// merge is done and only the publish must be finished.
    pub pre_epoch: u64,
}

fn outbox_path(vault_runtime_dir: &Path) -> PathBuf {
    vault_runtime_dir.join("rotation-outbox.json")
}

/// Record a pending rotation atomically (tmp + fsync + rename), 0600. MUST be
/// called *before* the caller merges its own MLS epoch (REQ-506 ordering).
pub fn write_outbox(vault_runtime_dir: &Path, outbox: &RotationOutbox) -> Result<()> {
    use std::io::Write as _;
    std::fs::create_dir_all(vault_runtime_dir)
        .with_context(|| format!("create {}", vault_runtime_dir.display()))?;
    let path = outbox_path(vault_runtime_dir);
    let bytes = serde_json::to_vec_pretty(outbox).context("serialise rotation outbox")?;
    let tmp = path.with_extension("json.tmp");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o600);
    }
    let mut f = opts
        .open(&tmp)
        .with_context(|| format!("open {}", tmp.display()))?;
    f.write_all(&bytes)?;
    f.sync_all()?; // durable before the epoch advances
    std::fs::rename(&tmp, &path)?;
    // The rename's directory entry must also be durable, or a power loss can
    // lose the outbox despite the file sync above.
    crate::crdt::fsync_dir(vault_runtime_dir)?;
    Ok(())
}

/// The complete REQ-506 removal sequence in one call, so no caller can get
/// the ordering wrong: create the (unmerged) Remove commit, record it durably
/// in the rotation outbox, and only then advance the local epoch. A crash at
/// any point leaves either no rotation (before the outbox write) or a
/// recorded rotation that [`recovery_action`] completes exactly once.
pub fn remove_member_durable(
    vault_runtime_dir: &Path,
    provider: &openmls_rust_crypto::OpenMlsRustCrypto,
    group: &mut openmls::prelude::MlsGroup,
    owner: &super::group::GroupIdentity,
    did: &str,
) -> Result<Vec<u8>> {
    let pre_epoch = group.epoch().as_u64();
    let commit = super::group::remove_member(provider, group, owner, did)?;
    write_outbox(
        vault_runtime_dir,
        &RotationOutbox {
            commit: commit.clone(),
            pre_epoch,
        },
    )?;
    super::group::merge_removal(provider, group)?;
    Ok(commit)
}

/// Read a pending rotation, if a crash left one behind.
pub fn read_outbox(vault_runtime_dir: &Path) -> Result<Option<RotationOutbox>> {
    match std::fs::read(outbox_path(vault_runtime_dir)) {
        Ok(bytes) => Ok(Some(
            serde_json::from_slice(&bytes).context("parse rotation outbox")?,
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).context("read rotation outbox"),
    }
}

/// Clear the outbox once the rotation is fully published (idempotent).
pub fn clear_outbox(vault_runtime_dir: &Path) -> Result<()> {
    match std::fs::remove_file(outbox_path(vault_runtime_dir)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).context("clear rotation outbox"),
    }
}

/// Whether a rotation still needs completing (crash recovery, REQ-506). Given
/// the live group epoch, decide what recovery must do:
/// - no outbox → nothing pending;
/// - outbox present, live epoch already advanced past `pre_epoch` → the merge
///   happened; only re-publish the recorded commit, then clear;
/// - outbox present, live epoch still at `pre_epoch` → the crash landed before
///   the merge; re-apply (merge) *and* publish, then clear.
#[derive(Debug, PartialEq, Eq)]
pub enum Recovery {
    Nothing,
    RepublishOnly,
    ReapplyAndRepublish,
}

/// Pure recovery decision (REQ-506) — a function of the outbox and live epoch,
/// so it is unit-testable without a live MLS group.
pub fn recovery_action(outbox: Option<&RotationOutbox>, live_epoch: u64) -> Recovery {
    match outbox {
        None => Recovery::Nothing,
        Some(o) if live_epoch > o.pre_epoch => Recovery::RepublishOnly,
        Some(_) => Recovery::ReapplyAndRepublish,
    }
}

mod b64 {
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&B64.encode(bytes))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        B64.decode(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // REQ-506: the outbox persists a pending rotation atomically and reloads it.
    #[test]
    fn outbox_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let ob = RotationOutbox {
            commit: vec![1, 2, 3, 4],
            pre_epoch: 7,
        };
        assert!(read_outbox(tmp.path()).unwrap().is_none());
        write_outbox(tmp.path(), &ob).unwrap();
        assert_eq!(read_outbox(tmp.path()).unwrap(), Some(ob));
        clear_outbox(tmp.path()).unwrap();
        assert!(read_outbox(tmp.path()).unwrap().is_none());
    }

    // REQ-506 recovery decisions: complete an interrupted rotation exactly once,
    // distinguishing "crash before merge" from "crash after merge".
    #[test]
    fn recovery_completes_exactly_once() {
        let ob = RotationOutbox {
            commit: vec![9],
            pre_epoch: 3,
        };
        // No outbox → nothing to do.
        assert_eq!(recovery_action(None, 5), Recovery::Nothing);
        // Crash before the epoch merged (live still at pre_epoch) → reapply.
        assert_eq!(recovery_action(Some(&ob), 3), Recovery::ReapplyAndRepublish);
        // Crash after the merge (live advanced past pre_epoch) → republish only,
        // never re-rotate.
        assert_eq!(recovery_action(Some(&ob), 4), Recovery::RepublishOnly);
    }
}
