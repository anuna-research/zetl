//! Roster + role gate (SPEC-047 CON-477, REQ-480/505, IMPL-047 T10).
//!
//! **UNREVIEWED AUTH-CORE** — see the banner in [`super`]. Pending Q1/Q2/Q7/Q11.
//!
//! The roster is the per-vault membership projection: member DID → {role,
//! key_epoch}. Roles are `owner | member`, immutable in v1 (REQ-505); the
//! vault-genesis member is the `owner`. The **role gate** answers "may this
//! author change membership?" — `owner` only. This composes the MLS group
//! (T9): the Owner is permanently leaf 0, so the roster's owner is the group
//! creator, and the MLS layer already refuses non-Owner commits at the crypto
//! level (T9 `process_commit`). This module is the *policy projection* the
//! daemon and higher layers read (who is on the roster, in what role, at what
//! epoch), plus the explicit authorization predicate.

use openmls::prelude::MlsGroup;
use std::collections::BTreeMap;

/// A member's role in the vault (REQ-505, immutable in v1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    /// Sole membership authority — may commit admissions and removals.
    Owner,
    /// A member; reads/writes the vault but cannot change membership.
    Member,
}

/// One roster entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterEntry {
    pub did: String,
    pub role: Role,
    /// MLS group epoch at which this projection was taken (`key_epoch`).
    pub key_epoch: u64,
}

/// The membership roster (CON-477), projected from the vault's MLS group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roster {
    /// DID → entry, deterministic order.
    entries: BTreeMap<String, RosterEntry>,
    owner_did: String,
}

impl Roster {
    /// Project the roster from the MLS group. `owner_did` is the DID that
    /// created the group (leaf 0) — every other member is `member`.
    pub fn from_group(group: &MlsGroup, owner_did: &str) -> Roster {
        let key_epoch = group.epoch().as_u64();
        let mut entries = BTreeMap::new();
        for m in group.members() {
            // Decode the DID from the leaf credential grammar (did ‖ endpoint);
            // skip any leaf whose credential does not parse (fail closed).
            let Ok((did, _endpoint)) =
                crate::p2p::group::decode_credential(m.credential.serialized_content())
            else {
                continue;
            };
            let role = if did == owner_did {
                Role::Owner
            } else {
                Role::Member
            };
            entries.insert(
                did.clone(),
                RosterEntry {
                    did,
                    role,
                    key_epoch,
                },
            );
        }
        Roster {
            entries,
            owner_did: owner_did.to_string(),
        }
    }

    pub fn owner_did(&self) -> &str {
        &self.owner_did
    }

    pub fn role_of(&self, did: &str) -> Option<Role> {
        self.entries.get(did).map(|e| e.role)
    }

    pub fn contains(&self, did: &str) -> bool {
        self.entries.contains_key(did)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> impl Iterator<Item = &RosterEntry> {
        self.entries.values()
    }

    /// The role gate (REQ-505): may `did` author a **membership** change
    /// (admission / member-level removal)? Only the `owner`. A device managing
    /// its *own* DID's verification methods is a separate, member-delegated
    /// permission (REQ-498) — not gated here.
    pub fn may_change_membership(&self, did: &str) -> bool {
        self.role_of(did) == Some(Role::Owner)
    }
}

#[cfg(test)]
mod tests {
    use super::super::group::{
        add_member, build_key_package, create_group, key_package_from_bytes, provider,
        GroupIdentity,
    };
    use super::*;

    fn two_member_group() -> (MlsGroup, String, String) {
        let op = provider();
        let bp = provider();
        let owner = GroupIdentity::new(&[1u8; 32]).unwrap();
        let bob = GroupIdentity::new(&[2u8; 32]).unwrap();
        let mut group = create_group(&op, &owner, b"vault").unwrap();
        let (_kb, kp_bytes) = build_key_package(&bp, &bob).unwrap();
        let kp = key_package_from_bytes(&op, &kp_bytes, &bob.endpoint_id).unwrap();
        add_member(&op, &mut group, &owner, kp).unwrap();
        (group, owner.did, bob.did)
    }

    // REQ-505: the roster projects owner + member roles; the genesis member is
    // the owner, admitted members are `member`.
    #[test]
    fn roster_projects_owner_and_member_roles() {
        let (group, owner_did, bob_did) = two_member_group();
        let roster = Roster::from_group(&group, &owner_did);

        assert_eq!(roster.len(), 2);
        assert_eq!(roster.role_of(&owner_did), Some(Role::Owner));
        assert_eq!(roster.role_of(&bob_did), Some(Role::Member));
        assert_eq!(roster.role_of("did:crdt:stranger"), None);
    }

    // REQ-505 role gate: only the owner may change membership; a member and a
    // non-member may not.
    #[test]
    fn only_owner_may_change_membership() {
        let (group, owner_did, bob_did) = two_member_group();
        let roster = Roster::from_group(&group, &owner_did);

        assert!(roster.may_change_membership(&owner_did), "owner may");
        assert!(!roster.may_change_membership(&bob_did), "member may not");
        assert!(
            !roster.may_change_membership("did:crdt:stranger"),
            "non-member may not"
        );
    }
}
