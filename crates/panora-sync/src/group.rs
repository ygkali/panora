// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! The sync group: which devices belong to it (the roster) and the
//! symmetric key they share (ADR 0005).
//!
//! Every membership change is a new *roster*: the full member list, an
//! epoch one higher than its parent's, the parent's hash, and the signature
//! of a device that was a member of the parent. All members are equal; any
//! of them can add or remove devices. Removing a device always introduces
//! a fresh group key (a new *key epoch*), so the removed device cannot read
//! what is shared afterwards. The roster carries only a keyed check value
//! of the key, never the key itself; the key travels to members over an
//! authenticated channel (the pairing welcome, or SYNC-04's pinned TLS) and
//! is accepted only if it matches that check.
//!
//! Two members can change the roster at the same time, which forks it: two
//! different branches growing from the same parent. Branches are compared
//! as a whole ([`crate::group::fork_winner`]): one that removes a signer of
//! the other wins, so a removed, possibly stolen device cannot undo its
//! removal by signing a competing roster on an older parent; when each
//! removes a signer of the other, a device keeps the branch it has and
//! never undoes a removal it has seen; otherwise a value the signers cannot
//! grind decides. The losing side's own changes are handed back
//! ([`crate::group::RosterUpdate::Replaced`]) so its device can reapply
//! them on top of the winner.

use crate::bytes::{b64, b64_secret, b64_vec, put};
use crate::error::{Error, Result};
use crate::identity::{DeviceIdentity, PublicIdentity, SIGNATURE_LEN};
use panora_core::storage::{Cipher, MasterKey};
use panora_core::sync::SyncRecord;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Most devices a group may hold.
pub const MAX_MEMBERS: usize = 16;
/// Longest device name, in characters.
pub const MAX_NAME_CHARS: usize = 64;
/// Longest device id, in bytes.
pub const MAX_DEVICE_ID_LEN: usize = 64;
/// Length of a group id.
pub const GROUP_ID_LEN: usize = 16;
/// Length of the group key.
pub const GROUP_KEY_LEN: usize = 32;
/// Rosters kept (the current one included), enough to judge a fork a few
/// changes deep. A new device receives all of them.
pub const KEPT_ROSTERS: usize = 8;
/// Highest roster epoch. Far beyond any real group, and low enough that
/// `epoch + 1` never overflows, whatever a peer sends.
pub const MAX_EPOCH: u64 = u32::MAX as u64;

/// A group's random identifier.
pub type GroupId = [u8; GROUP_ID_LEN];

/// Check a device id: what `panod` stamps on history entries (32 hex
/// digits today). 1–64 ASCII letters, digits or `-`.
pub fn validate_device_id(id: &str) -> Result<()> {
    let ok = !id.is_empty()
        && id.len() <= MAX_DEVICE_ID_LEN
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if ok {
        Ok(())
    } else {
        Err(Error::Device(
            "device id must be 1-64 ASCII letters, digits or '-'",
        ))
    }
}

/// Check a device name. It is shown to the user in the device list and in
/// pairing prompts, so it must not be empty, padded, overlong, or contain
/// control or text-direction override characters (which could make one
/// name render as another).
pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.trim() != name {
        return Err(Error::Device(
            "device name must not be empty or start or end with spaces",
        ));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(Error::Device("device name is longer than 64 characters"));
    }
    if name.chars().any(|c| c.is_control() || is_format(c)) {
        return Err(Error::Device(
            "device name contains control, format or line-separator characters",
        ));
    }
    Ok(())
}

/// Characters that change how the text around them is shown without
/// being visible themselves (Unicode general categories Cf, Zl and Zp):
/// direction overrides, zero-width spaces and joiners, soft hyphens, line
/// and paragraph separators, tags. A name holding one could pass for
/// another device's, or add a fake line to a pairing prompt.
fn is_format(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{0600}'..='\u{0605}'
            | '\u{061C}'
            | '\u{06DD}'
            | '\u{070F}'
            | '\u{0890}'..='\u{0891}'
            | '\u{08E2}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{2028}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{110BD}'
            | '\u{110CD}'
            | '\u{13430}'..='\u{1343F}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0001}'
            | '\u{E0020}'..='\u{E007F}'
    )
}

/// One device in the roster.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    /// The device's long-term public key.
    pub identity: PublicIdentity,
    /// The id its `panod` stamps on history entries.
    pub device_id: String,
    /// User-visible name (the host name by default).
    pub name: String,
    /// Unix time the device was added, per the adding device's clock.
    pub added_at: i64,
}

/// A signed membership list; see the module documentation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Roster {
    /// The group this roster belongs to.
    #[serde(with = "b64")]
    pub group_id: GroupId,
    /// 0 for the group's first roster, then parent + 1.
    pub epoch: u64,
    /// [`Roster::hash`] of the parent; zeros for epoch 0.
    #[serde(with = "b64")]
    pub prev: [u8; 32],
    /// Unix time of the change, per the signer's clock. Informational.
    pub created_at: i64,
    /// Epoch of the roster that introduced the current group key.
    pub key_epoch: u64,
    /// [`GroupKey::check`] of the current group key.
    #[serde(with = "b64")]
    pub key_check: [u8; 32],
    /// Members, sorted by identity key.
    pub members: Vec<Member>,
    /// The member (of the parent roster) that made this change.
    pub signer: PublicIdentity,
    /// Ed25519 signature by `signer` over everything above.
    #[serde(with = "b64")]
    pub signature: [u8; SIGNATURE_LEN],
}

impl Roster {
    /// Canonical encoding of every field but the signature.
    fn body(&self) -> Vec<u8> {
        let mut out = b"panora-roster/1".to_vec();
        put(&mut out, &self.group_id);
        put(&mut out, &self.epoch.to_be_bytes());
        put(&mut out, &self.prev);
        put(&mut out, &self.created_at.to_be_bytes());
        put(&mut out, &self.key_epoch.to_be_bytes());
        put(&mut out, &self.key_check);
        put(&mut out, self.signer.as_bytes());
        put(&mut out, &(self.members.len() as u64).to_be_bytes());
        for m in &self.members {
            put(&mut out, m.identity.as_bytes());
            put(&mut out, m.device_id.as_bytes());
            put(&mut out, m.name.as_bytes());
            put(&mut out, &m.added_at.to_be_bytes());
        }
        out
    }

    /// Hash of the signed content; what a child's `prev` refers to.
    pub fn hash(&self) -> [u8; 32] {
        blake3::derive_key("panora roster hash v1", &self.body())
    }

    fn signed(mut self, identity: &DeviceIdentity) -> Self {
        self.signer = identity.public();
        self.signature = identity.sign(&self.body());
        self
    }

    /// The entry for `identity`, if it is a member.
    pub fn member(&self, identity: &PublicIdentity) -> Option<&Member> {
        self.members
            .binary_search_by(|m| m.identity.cmp(identity))
            .ok()
            .map(|i| &self.members[i])
    }

    /// Check the roster on its own: shape, limits and signature. Whether
    /// the signer was allowed to make it is [`check_successor`]'s job.
    pub fn verify(&self) -> Result<()> {
        if self.members.is_empty() || self.members.len() > MAX_MEMBERS {
            return Err(Error::Roster("a roster holds between 1 and 16 devices"));
        }
        if !self
            .members
            .windows(2)
            .all(|w| w[0].identity < w[1].identity)
        {
            return Err(Error::Roster(
                "members are not sorted or a device is listed twice",
            ));
        }
        let mut ids: Vec<&str> = self.members.iter().map(|m| m.device_id.as_str()).collect();
        ids.sort_unstable();
        if ids.windows(2).any(|w| w[0] == w[1]) {
            return Err(Error::Roster("two members share a device id"));
        }
        for m in &self.members {
            validate_device_id(&m.device_id)?;
            validate_name(&m.name)?;
        }
        if self.epoch > MAX_EPOCH {
            return Err(Error::Roster("roster epoch is out of range"));
        }
        if self.key_epoch > self.epoch {
            return Err(Error::Roster("key epoch is ahead of the roster epoch"));
        }
        if self.epoch == 0
            && (self.prev != [0; 32] || self.key_epoch != 0 || self.member(&self.signer).is_none())
        {
            return Err(Error::Roster(
                "a group's first roster must be made by one of its members",
            ));
        }
        self.signer.verify(&self.body(), &self.signature)
    }

    fn members_without(&self, other: &Roster) -> impl Iterator<Item = &Member> + '_ {
        let other: Vec<PublicIdentity> = other.members.iter().map(|m| m.identity).collect();
        self.members
            .iter()
            .filter(move |m| other.binary_search(&m.identity).is_err())
    }
}

/// Is `next` a legitimate change of `parent`? `next` must already have
/// passed [`Roster::verify`].
pub fn check_successor(parent: &Roster, next: &Roster) -> Result<()> {
    if next.group_id != parent.group_id {
        return Err(Error::Roster("roster belongs to another group"));
    }
    if parent.epoch.checked_add(1) != Some(next.epoch) || next.prev != parent.hash() {
        return Err(Error::Roster("roster does not follow its parent"));
    }
    if parent.member(&next.signer).is_none() {
        return Err(Error::Roster(
            "roster was signed by a device that is not a member",
        ));
    }
    for m in &parent.members {
        if let Some(n) = next.member(&m.identity) {
            if n != m {
                return Err(Error::Roster("a member's details changed"));
            }
        }
    }
    let removes = parent.members_without(next).next().is_some();
    let adds = next.members_without(parent).next().is_some();
    let rotates = next.key_epoch == next.epoch;
    if rotates {
        if next.key_check == parent.key_check {
            return Err(Error::Roster("a new key epoch must bring a new key"));
        }
    } else if next.key_epoch != parent.key_epoch || next.key_check != parent.key_check {
        return Err(Error::Roster(
            "the group key changed without a new key epoch",
        ));
    }
    if removes && !rotates {
        return Err(Error::Roster(
            "removing a device must introduce a new group key",
        ));
    }
    if !removes && !adds && !rotates {
        return Err(Error::Roster("roster changes nothing"));
    }
    Ok(())
}

/// Identities a branch removes: members of `parent`, or of an earlier
/// roster of the branch, that a later roster of the branch no longer lists.
fn removed_along(parent: &Roster, branch: &[Roster]) -> Vec<PublicIdentity> {
    let mut removed = Vec::new();
    let mut before = parent;
    for r in branch {
        removed.extend(before.members_without(r).map(|m| m.identity));
        before = r;
    }
    removed
}

/// Of two different branches growing from `parent`, does `incoming` beat
/// `held`, the one this device has? Whole branches are compared, not
/// single rosters: a device removed on one branch can still sign a
/// competing roster on an older parent, where it was a member.
///
/// 1. A branch that removes a signer of the other wins; a removal cannot
///    be undone by a roster its target signed.
/// 2. If each removes a signer of the other (two devices removing each
///    other, or a removed device fighting back), `held` wins: a device
///    never undoes a removal it has seen. Devices that saw different sides
///    first can stay split; ADR 0005 documents it.
/// 3. A branch that removes a device the other still lists beats one that
///    removes nobody, so an unrelated change made by a device that was
///    offline cannot bring a removed device back.
/// 4. Otherwise a value no signer can grind decides: a hash of the parent
///    and the first roster's signer, then the roster hash. Every device
///    computes the same answer. When both branches removed someone, the
///    loser's removals come back through [`RosterUpdate::Replaced`] so
///    the device redoes them at once.
pub fn fork_winner(parent: &Roster, incoming: &[Roster], held: &[Roster]) -> bool {
    let removes_signer = |a: &[Roster], b: &[Roster]| {
        let removed = removed_along(parent, a);
        b.iter().any(|r| removed.contains(&r.signer))
    };
    let removes_listed = |a: &[Roster], b: &[Roster]| {
        let removed = removed_along(parent, a);
        b.last()
            .is_some_and(|end| removed.iter().any(|id| end.member(id).is_some()))
    };
    match (
        removes_signer(incoming, held),
        removes_signer(held, incoming),
    ) {
        (true, false) => return true,
        (false, true) | (true, true) => return false,
        (false, false) => {}
    }
    match (
        removes_listed(incoming, held),
        removes_listed(held, incoming),
    ) {
        (true, false) => return true,
        (false, true) => return false,
        _ => {}
    }
    let rank = |r: &Roster| {
        let mut input = parent.hash().to_vec();
        input.extend_from_slice(r.signer.as_bytes());
        (
            blake3::derive_key("panora roster fork rank v1", &input),
            r.hash(),
        )
    };
    match (incoming.first(), held.first()) {
        (Some(a), Some(b)) => rank(a) < rank(b),
        _ => false,
    }
}

/// What to redo after `lost` lost a fork, judged by where that branch
/// ended (re-adding a device someone removed later on it, or re-removing
/// one someone added back, would undo their change):
///
/// - this device's own additions: only it knows it meant them, and several
///   devices re-adding the same device would only fork again;
/// - *every* removal, whoever made it: a removal that lost only a
///   tie-break must not leave the removed device in the group until its
///   author comes back online. Removals of this device itself are left to
///   the others.
fn changes_to_reapply(parent: &Roster, lost: &[Roster], me: &PublicIdentity) -> Vec<Change> {
    let Some(end) = lost.last() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut before = parent;
    for r in lost {
        if r.signer == *me {
            out.extend(
                r.members_without(before)
                    .filter(|m| end.member(&m.identity).is_some())
                    .cloned()
                    .map(Change::Added),
            );
        }
        out.extend(
            before
                .members_without(r)
                .filter(|m| m.identity != *me && end.member(&m.identity).is_none())
                .map(|m| Change::Removed(m.identity)),
        );
        before = r;
    }
    out
}

/// The shared symmetric key of one key epoch. Zeroized on drop.
#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct GroupKey {
    /// The key epoch it belongs to.
    pub epoch: u64,
    #[serde(with = "b64_secret")]
    key: [u8; GROUP_KEY_LEN],
}

impl GroupKey {
    fn generate(epoch: u64) -> Self {
        let mut key = [0u8; GROUP_KEY_LEN];
        rand::rngs::OsRng.fill_bytes(&mut key);
        Self { epoch, key }
    }

    /// A keyed BLAKE3 value the roster publishes so a received key can be
    /// checked; reveals nothing about the key.
    pub fn check(&self, group_id: &GroupId) -> [u8; 32] {
        let mut input = b"panora group key check v1".to_vec();
        put(&mut input, group_id);
        put(&mut input, &self.epoch.to_be_bytes());
        *blake3::keyed_hash(&self.key, &input).as_bytes()
    }

    fn cipher(&self) -> Cipher {
        Cipher::new(&MasterKey::from_bytes(self.key))
    }
}

impl std::fmt::Debug for GroupKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GroupKey(epoch {}, [REDACTED])", self.epoch)
    }
}

/// What applying a roster from a peer did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RosterUpdate {
    /// Already had all of it.
    Unchanged,
    /// It continued the current roster; the group moved forward.
    Advanced,
    /// It competed with the branch this device holds and lost; ignored.
    ForkLost,
    /// It competed with the branch this device holds and won: that branch
    /// was dropped. `reapply` lists what to redo on top of the winner
    /// (this device's own additions, and every removal) for
    /// [`GroupState::reapply`]; the caller should do it straight away.
    Replaced {
        /// Changes from the losing branch to redo.
        reapply: Vec<Change>,
    },
    /// This device was removed from the group; its keys are wiped and the
    /// caller should discard the group.
    RemovedThisDevice,
}

/// One membership change, as handed back after a lost fork.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// A device was added.
    Added(Member),
    /// A device was removed.
    Removed(PublicIdentity),
}

/// A device list row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// The roster entry.
    pub member: Member,
    /// [`PublicIdentity::fingerprint`] of its key.
    pub fingerprint: String,
    /// Whether it is this device.
    pub this_device: bool,
}

/// Data sealed under the group key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sealed {
    /// Key epoch of the key used.
    pub key_epoch: u64,
    /// First bytes of that key's check value, to tell forked keys apart.
    #[serde(with = "b64")]
    pub key_id: [u8; 8],
    /// `panora_core` AEAD envelope (XChaCha20-Poly1305).
    #[serde(with = "b64_vec")]
    pub ciphertext: Vec<u8>,
}

/// This device's view of its group. Persisted inside the encrypted
/// [`crate::SyncState`]; loading re-verifies every roster.
#[derive(Clone, Serialize, Deserialize)]
#[serde(try_from = "GroupStateRepr", into = "GroupStateRepr")]
pub struct GroupState {
    me: PublicIdentity,
    rosters: Vec<Roster>,
    keys: Vec<GroupKey>,
}

#[derive(Serialize, Deserialize)]
struct GroupStateRepr {
    me: PublicIdentity,
    rosters: Vec<Roster>,
    keys: Vec<GroupKey>,
}

impl TryFrom<GroupStateRepr> for GroupState {
    type Error = Error;

    fn try_from(repr: GroupStateRepr) -> Result<Self> {
        let state = GroupState {
            me: repr.me,
            rosters: repr.rosters,
            keys: repr.keys,
        };
        state.validate()?;
        Ok(state)
    }
}

impl From<GroupState> for GroupStateRepr {
    fn from(state: GroupState) -> Self {
        GroupStateRepr {
            me: state.me,
            rosters: state.rosters,
            keys: state.keys,
        }
    }
}

impl std::fmt::Debug for GroupState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroupState")
            .field("me", &self.me)
            .field("epoch", &self.current().epoch)
            .field("members", &self.current().members.len())
            .field("keys", &self.keys)
            .finish()
    }
}

impl GroupState {
    /// Start a new group with this device as its only member.
    pub fn create(
        identity: &DeviceIdentity,
        device_id: &str,
        name: &str,
        now: i64,
    ) -> Result<Self> {
        validate_device_id(device_id)?;
        validate_name(name)?;
        let mut group_id = [0u8; GROUP_ID_LEN];
        rand::rngs::OsRng.fill_bytes(&mut group_id);
        let key = GroupKey::generate(0);
        let roster = Roster {
            group_id,
            epoch: 0,
            prev: [0; 32],
            created_at: now,
            key_epoch: 0,
            key_check: key.check(&group_id),
            members: vec![Member {
                identity: identity.public(),
                device_id: device_id.to_string(),
                name: name.to_string(),
                added_at: now,
            }],
            signer: identity.public(),
            signature: [0; SIGNATURE_LEN],
        }
        .signed(identity);
        Ok(Self {
            me: identity.public(),
            rosters: vec![roster],
            keys: vec![key],
        })
    }

    /// The state a newly paired device starts from: the rosters the
    /// inviter sent (oldest first) and the current key. The last roster
    /// must list this device.
    pub(crate) fn joined(me: PublicIdentity, rosters: Vec<Roster>, key: GroupKey) -> Result<Self> {
        let state = Self {
            me,
            rosters,
            keys: vec![key],
        };
        state.validate()?;
        if !state.is_member() {
            return Err(Error::Roster(
                "the welcome roster does not list this device",
            ));
        }
        if state.current_key().is_none() {
            return Err(Error::Roster("the welcome key does not match the roster"));
        }
        Ok(state)
    }

    fn validate(&self) -> Result<()> {
        let first = self
            .rosters
            .first()
            .ok_or(Error::Roster("group has no roster"))?;
        if self.rosters.len() > KEPT_ROSTERS {
            return Err(Error::Roster("too many rosters kept"));
        }
        first.verify()?;
        for pair in self.rosters.windows(2) {
            pair[1].verify()?;
            check_successor(&pair[0], &pair[1])?;
        }
        let gid = first.group_id;
        for key in &self.keys {
            let check = key.check(&gid);
            if !self
                .rosters
                .iter()
                .any(|r| r.key_epoch == key.epoch && r.key_check == check)
            {
                return Err(Error::Roster("a stored key matches no roster"));
            }
        }
        Ok(())
    }

    /// The group's id.
    pub fn group_id(&self) -> GroupId {
        self.current().group_id
    }

    /// This device's identity.
    pub fn me(&self) -> PublicIdentity {
        self.me
    }

    /// The current roster.
    pub fn current(&self) -> &Roster {
        self.rosters.last().expect("a group always has a roster")
    }

    /// The rosters an inviter hands a new device: all it keeps, so the
    /// newcomer can judge a fork as deep as any other member can.
    pub(crate) fn welcome_chain(&self) -> Vec<Roster> {
        self.rosters.clone()
    }

    /// Whether this device is in the current roster. False after removal,
    /// and also while a device that was added on a losing fork waits to be
    /// added again.
    pub fn is_member(&self) -> bool {
        self.current().member(&self.me).is_some()
    }

    /// Whether `identity` is in the current roster.
    pub fn is_peer(&self, identity: &PublicIdentity) -> bool {
        *identity != self.me && self.current().member(identity).is_some()
    }

    /// The device list, in roster order.
    pub fn devices(&self) -> Vec<DeviceInfo> {
        self.current()
            .members
            .iter()
            .map(|m| DeviceInfo {
                member: m.clone(),
                fingerprint: m.identity.fingerprint(),
                this_device: m.identity == self.me,
            })
            .collect()
    }

    /// The key of the current roster, if this device holds it (after a
    /// rotation made elsewhere it arrives separately, [`Self::accept_key`]).
    pub fn current_key(&self) -> Option<&GroupKey> {
        let r = self.current();
        self.keys
            .iter()
            .find(|k| k.epoch == r.key_epoch && k.check(&r.group_id) == r.key_check)
    }

    /// The current key, for handing to `peer` over a channel authenticated
    /// as that peer. Refused unless `peer` is a current member.
    pub fn key_for_peer(&self, peer: &PublicIdentity) -> Result<GroupKey> {
        if !self.is_peer(peer) {
            return Err(Error::Roster("that device is not a member of the group"));
        }
        self.current_key().cloned().ok_or(Error::Roster(
            "this device does not hold the current key yet",
        ))
    }

    /// Store a key received from a member. It must match the check value
    /// of a roster this device holds. Returns whether it was new.
    pub fn accept_key(&mut self, key: GroupKey) -> Result<bool> {
        if !self.is_member() {
            return Err(Error::Roster("this device is not a member of the group"));
        }
        let check = key.check(&self.group_id());
        if !self
            .rosters
            .iter()
            .any(|r| r.key_epoch == key.epoch && r.key_check == check)
        {
            return Err(Error::Roster("key does not match any known roster"));
        }
        if self
            .keys
            .iter()
            .any(|k| k.epoch == key.epoch && k.check(&self.group_id()) == check)
        {
            return Ok(false);
        }
        self.keys.push(key);
        self.prune();
        Ok(true)
    }

    fn change(&mut self, signer: &DeviceIdentity, now: i64) -> Result<Roster> {
        if signer.public() != self.me || !self.is_member() {
            return Err(Error::Roster("only a member can change the group"));
        }
        let cur = self.current();
        let mut next = cur.clone();
        next.epoch = cur.epoch + 1;
        next.prev = cur.hash();
        next.created_at = now;
        Ok(next)
    }

    fn push(&mut self, roster: Roster) {
        self.rosters.push(roster);
        let excess = self.rosters.len().saturating_sub(KEPT_ROSTERS);
        self.rosters.drain(..excess);
        self.prune();
    }

    /// Drop keys no kept roster refers to.
    fn prune(&mut self) {
        let gid = self.group_id();
        let rosters = &self.rosters;
        self.keys.retain(|k| {
            let check = k.check(&gid);
            rosters
                .iter()
                .any(|r| r.key_epoch == k.epoch && r.key_check == check)
        });
    }

    /// Add a device. Returns the new roster, to send to the other members.
    pub fn add_member(
        &mut self,
        signer: &DeviceIdentity,
        member: Member,
        now: i64,
    ) -> Result<Roster> {
        validate_device_id(&member.device_id)?;
        validate_name(&member.name)?;
        let mut next = self.change(signer, now)?;
        if next.member(&member.identity).is_some() {
            return Err(Error::Device("this device is already in the group"));
        }
        if next.members.iter().any(|m| m.device_id == member.device_id) {
            return Err(Error::Device(
                "another device in the group already uses this device id",
            ));
        }
        if next.members.len() >= MAX_MEMBERS {
            return Err(Error::Device("the group already has 16 devices"));
        }
        next.members.push(member);
        next.members.sort_by_key(|m| m.identity);
        let next = next.signed(signer);
        next.verify()?;
        check_successor(self.current(), &next)?;
        self.push(next.clone());
        Ok(next)
    }

    /// Remove a device and introduce a new group key. Returns the new
    /// roster (for every member) and the new key (for the remaining
    /// members, each over its own authenticated channel). A device cannot
    /// remove itself; the user removes it from another device.
    pub fn remove_member(
        &mut self,
        signer: &DeviceIdentity,
        target: &PublicIdentity,
        now: i64,
    ) -> Result<(Roster, GroupKey)> {
        if *target == self.me {
            return Err(Error::Device(
                "a device cannot remove itself; remove it from another device",
            ));
        }
        let mut next = self.change(signer, now)?;
        let before = next.members.len();
        next.members.retain(|m| m.identity != *target);
        if next.members.len() == before {
            return Err(Error::Device("that device is not in the group"));
        }
        let key = GroupKey::generate(next.epoch);
        next.key_epoch = next.epoch;
        next.key_check = key.check(&next.group_id);
        let next = next.signed(signer);
        next.verify()?;
        check_successor(self.current(), &next)?;
        self.keys.push(key.clone());
        self.push(next.clone());
        Ok((next, key))
    }

    /// Apply a roster received from a member; [`Self::apply_chain`] with
    /// one roster.
    pub fn apply_roster(&mut self, roster: Roster) -> Result<RosterUpdate> {
        self.apply_chain(vec![roster])
    }

    /// Apply a member's rosters, oldest first and consecutive. The chain
    /// may overlap what this device holds. A peer on another branch sends
    /// its branch from the common parent on, so the two branches are
    /// compared whole ([`fork_winner`]); a single roster whose parent is
    /// not here is an error that asks for that chain.
    pub fn apply_chain(&mut self, chain: Vec<Roster>) -> Result<RosterUpdate> {
        if chain.is_empty() || chain.len() > KEPT_ROSTERS {
            return Err(Error::Roster("a roster chain holds 1 to 8 rosters"));
        }
        let gid = self.group_id();
        for (i, r) in chain.iter().enumerate() {
            r.verify()?;
            if r.group_id != gid {
                return Err(Error::Roster("roster belongs to another group"));
            }
            if i > 0 {
                check_successor(&chain[i - 1], r)?;
            }
        }
        let first = self.rosters[0].epoch;
        // A peer that is behind may start before what is kept here; those
        // rosters are history this device has already moved past.
        let older = chain.iter().take_while(|r| r.epoch < first).count();
        let chain = &chain[older..];
        let held_at = |epoch: u64| {
            epoch
                .checked_sub(first)
                .and_then(|i| self.rosters.get(i as usize))
        };
        let skip = chain
            .iter()
            .take_while(|r| held_at(r.epoch).is_some_and(|h| h.hash() == r.hash()))
            .count();
        let incoming = &chain[skip..];
        let Some(head) = incoming.first() else {
            return Ok(RosterUpdate::Unchanged);
        };
        // Both are at most MAX_EPOCH, so neither addition overflows.
        if head.epoch > self.current().epoch + 1 {
            return Err(Error::Roster(
                "rosters in between are missing; fetch the chain",
            ));
        }
        if head.epoch <= first {
            return Err(Error::Roster(
                "the chain forks before this device's roster history",
            ));
        }
        let idx = (head.epoch - first) as usize;
        let parent = self.rosters[idx - 1].clone();
        check_successor(&parent, head).map_err(|_| {
            Error::Roster("roster does not follow what this device holds; fetch the chain")
        })?;
        let update = if idx == self.rosters.len() {
            RosterUpdate::Advanced
        } else {
            if !fork_winner(&parent, incoming, &self.rosters[idx..]) {
                return Ok(RosterUpdate::ForkLost);
            }
            let lost = self.rosters.split_off(idx);
            RosterUpdate::Replaced {
                reapply: changes_to_reapply(&parent, &lost, &self.me),
            }
        };
        for r in incoming {
            self.push(r.clone());
        }
        if !self.is_member() && removed_along(&parent, incoming).contains(&self.me) {
            self.keys.clear();
            return Ok(RosterUpdate::RemovedThisDevice);
        }
        Ok(update)
    }

    /// Redo this device's changes that a lost fork dropped, where they
    /// still make sense. Returns the new rosters (and any new key) to send.
    pub fn reapply(
        &mut self,
        signer: &DeviceIdentity,
        changes: Vec<Change>,
        now: i64,
    ) -> Result<Vec<(Roster, Option<GroupKey>)>> {
        let mut out = Vec::new();
        for change in changes {
            match change {
                Change::Added(m) => {
                    let current = self.current();
                    let taken = current.member(&m.identity).is_some()
                        || current.members.iter().any(|x| x.device_id == m.device_id);
                    if !taken {
                        out.push((self.add_member(signer, m, now)?, None));
                    }
                }
                Change::Removed(id) => {
                    if self.is_peer(&id) {
                        let (roster, key) = self.remove_member(signer, &id, now)?;
                        out.push((roster, Some(key)));
                    }
                }
            }
        }
        Ok(out)
    }

    fn seal_aad(group_id: &GroupId, key_epoch: u64, key_id: &[u8; 8], context: &[u8]) -> Vec<u8> {
        let mut aad = b"panora-group-seal/1".to_vec();
        put(&mut aad, group_id);
        put(&mut aad, &key_epoch.to_be_bytes());
        put(&mut aad, key_id);
        put(&mut aad, context);
        aad
    }

    /// Encrypt `plaintext` under the current group key, bound to
    /// `context` (a label saying what the data is).
    pub fn seal(&self, context: &[u8], plaintext: &[u8]) -> Result<Sealed> {
        if !self.is_member() {
            return Err(Error::Roster("this device is not a member of the group"));
        }
        let key = self.current_key().ok_or(Error::Roster(
            "this device does not hold the current key yet",
        ))?;
        let gid = self.group_id();
        let mut key_id = [0u8; 8];
        key_id.copy_from_slice(&key.check(&gid)[..8]);
        let aad = Self::seal_aad(&gid, key.epoch, &key_id, context);
        Ok(Sealed {
            key_epoch: key.epoch,
            key_id,
            ciphertext: key.cipher().seal_with_aad(&aad, plaintext)?,
        })
    }

    /// Decrypt data a member sealed. Only the *current* key is accepted: a
    /// removed device still holds the older ones, and must not be able to
    /// inject data with them.
    pub fn open(&self, context: &[u8], sealed: &Sealed) -> Result<Vec<u8>> {
        if !self.is_member() {
            return Err(Error::Roster("this device is not a member of the group"));
        }
        let gid = self.group_id();
        let key = self
            .current_key()
            .filter(|k| k.epoch == sealed.key_epoch && k.check(&gid)[..8] == sealed.key_id)
            .ok_or(Error::Roster("sealed with an older or unknown group key"))?;
        let aad = Self::seal_aad(&gid, key.epoch, &sealed.key_id, context);
        Ok(key.cipher().open_with_aad(&aad, &sealed.ciphertext)?)
    }

    /// [`Self::seal`] for a history record from `SyncChanges`.
    pub fn seal_record(&self, record: &SyncRecord) -> Result<Sealed> {
        let json = zeroize::Zeroizing::new(serde_json::to_vec(record)?);
        self.seal(b"sync-record", &json)
    }

    /// [`Self::open`] for a history record, ready for `SyncApply` (which
    /// still verifies its content hash and runs the privacy gate).
    pub fn open_record(&self, sealed: &Sealed) -> Result<SyncRecord> {
        let json = zeroize::Zeroizing::new(self.open(b"sync-record", sealed)?);
        Ok(serde_json::from_slice(&json)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use panora_core::model::{MimePayload, Selection};

    struct Device {
        id: DeviceIdentity,
        device_id: String,
        name: String,
    }

    fn device(n: u8) -> Device {
        Device {
            id: DeviceIdentity::generate().unwrap(),
            device_id: format!("{n:032x}"),
            name: format!("device {n}"),
        }
    }

    fn member(d: &Device) -> Member {
        Member {
            identity: d.id.public(),
            device_id: d.device_id.clone(),
            name: d.name.clone(),
            added_at: 10,
        }
    }

    /// A replica of `group` as seen by `d`, as a welcome would build it.
    fn replica(group: &GroupState, d: &Device) -> GroupState {
        GroupState::joined(
            d.id.public(),
            group.welcome_chain(),
            group.current_key().unwrap().clone(),
        )
        .unwrap()
    }

    #[test]
    fn create_add_remove_rotates_the_key() {
        let a = device(1);
        let b = device(2);
        let c = device(3);
        let mut g = GroupState::create(&a.id, &a.device_id, &a.name, 1).unwrap();
        assert_eq!(g.devices().len(), 1);
        assert!(g.devices()[0].this_device);
        let k0 = g.current_key().unwrap().check(&g.group_id());

        g.add_member(&a.id, member(&b), 2).unwrap();
        g.add_member(&a.id, member(&c), 3).unwrap();
        assert_eq!(g.current().epoch, 2);
        assert_eq!(g.current().members.len(), 3);
        // Adding keeps the key.
        assert_eq!(g.current_key().unwrap().check(&g.group_id()), k0);

        let (roster, key) = g.remove_member(&a.id, &c.id.public(), 4).unwrap();
        assert_eq!(roster.key_epoch, 3);
        assert_ne!(key.check(&g.group_id()), k0);
        assert!(!g.is_peer(&c.id.public()));
        assert!(g.key_for_peer(&c.id.public()).is_err());
        assert!(g.key_for_peer(&b.id.public()).is_ok());
    }

    #[test]
    fn duplicates_and_self_removal_are_refused() {
        let a = device(1);
        let b = device(2);
        let mut g = GroupState::create(&a.id, &a.device_id, &a.name, 1).unwrap();
        g.add_member(&a.id, member(&b), 2).unwrap();
        assert!(g.add_member(&a.id, member(&b), 3).is_err());
        let mut clone = member(&device(3));
        clone.device_id = b.device_id.clone();
        assert!(g.add_member(&a.id, clone, 3).is_err());
        assert!(g.remove_member(&a.id, &a.id.public(), 3).is_err());
        // Only this device may sign for this state.
        assert!(g.add_member(&b.id, member(&device(4)), 3).is_err());
    }

    #[test]
    fn replicas_follow_the_chain_and_accept_the_rotated_key() {
        let a = device(1);
        let b = device(2);
        let c = device(3);
        let mut ga = GroupState::create(&a.id, &a.device_id, &a.name, 1).unwrap();
        ga.add_member(&a.id, member(&b), 2).unwrap();
        let mut gb = replica(&ga, &b);
        let r = ga.add_member(&a.id, member(&c), 3).unwrap();
        let mut gc = replica(&ga, &c);
        assert_eq!(gb.apply_roster(r.clone()).unwrap(), RosterUpdate::Advanced);
        assert_eq!(gb.apply_roster(r).unwrap(), RosterUpdate::Unchanged);
        assert_eq!(
            gb.apply_chain(ga.welcome_chain()).unwrap(),
            RosterUpdate::Unchanged
        );

        let (r, key) = ga.remove_member(&a.id, &c.id.public(), 4).unwrap();
        assert_eq!(gb.apply_roster(r.clone()).unwrap(), RosterUpdate::Advanced);
        // B has the roster but not the new key until A hands it over.
        assert!(gb.current_key().is_none());
        assert!(gb.accept_key(key.clone()).unwrap());
        assert!(!gb.accept_key(key.clone()).unwrap());
        assert!(gb.current_key().is_some());

        // C learns it was removed, and its keys are gone.
        assert_eq!(gc.apply_roster(r).unwrap(), RosterUpdate::RemovedThisDevice);
        assert!(!gc.is_member());
        assert!(gc.current_key().is_none());
        assert!(gc.accept_key(key).is_err());
    }

    #[test]
    fn a_key_that_does_not_match_the_roster_is_refused() {
        let a = device(1);
        let mut g = GroupState::create(&a.id, &a.device_id, &a.name, 1).unwrap();
        assert!(g.accept_key(GroupKey::generate(0)).is_err());
        assert!(g.accept_key(GroupKey::generate(5)).is_err());
    }

    #[test]
    fn forged_and_unauthorised_rosters_are_refused() {
        let a = device(1);
        let b = device(2);
        let outsider = device(9);
        let mut ga = GroupState::create(&a.id, &a.device_id, &a.name, 1).unwrap();
        ga.add_member(&a.id, member(&b), 2).unwrap();
        let mut gb = replica(&ga, &b);

        // Tampered after signing.
        let mut other = ga.clone();
        let mut r = other.add_member(&a.id, member(&device(3)), 3).unwrap();
        r.members[0].name = "evil".into();
        assert!(gb.apply_roster(r).is_err());

        // Signed correctly, but by a device that is not a member.
        let mut r = other.current().clone();
        r.epoch = 3;
        r.prev = other.current().hash();
        r.members.push(member(&outsider));
        r.members.sort_by_key(|m| m.identity);
        let r = r.signed(&outsider.id);
        assert!(r.verify().is_ok());
        assert!(check_successor(other.current(), &r).is_err());

        // Removing without rotating the key.
        let mut r = gb.current().clone();
        r.epoch += 1;
        r.prev = gb.current().hash();
        r.members.retain(|m| m.identity != a.id.public());
        let r = r.signed(&b.id);
        assert!(gb.apply_roster(r).is_err());

        // A gap needs the chain.
        let mut r = ga.clone();
        r.add_member(&a.id, member(&device(4)), 3).unwrap();
        let far = r.add_member(&a.id, member(&device(5)), 4).unwrap();
        assert!(gb.apply_roster(far).is_err());
    }

    #[test]
    fn removed_device_cannot_sign_the_next_roster() {
        let a = device(1);
        let b = device(2);
        let mut ga = GroupState::create(&a.id, &a.device_id, &a.name, 1).unwrap();
        ga.add_member(&a.id, member(&b), 2).unwrap();
        let mut gb = replica(&ga, &b);
        let (r, _) = ga.remove_member(&a.id, &b.id.public(), 3).unwrap();
        gb.apply_roster(r).unwrap();
        // B, now outside, tries to add a device on top anyway.
        let mut evil = gb.current().clone();
        evil.epoch += 1;
        evil.prev = gb.current().hash();
        evil.members.push(member(&device(7)));
        evil.members.sort_by_key(|m| m.identity);
        let evil = evil.signed(&b.id);
        assert!(ga.apply_roster(evil).is_err());
        assert!(gb.add_member(&b.id, member(&device(8)), 4).is_err());
    }

    #[test]
    fn a_removal_beats_the_removed_devices_concurrent_change() {
        // A stolen laptop (T) races its own removal by adding a device.
        let owner = device(1);
        let stolen = device(2);
        let mut g_owner = GroupState::create(&owner.id, &owner.device_id, &owner.name, 1).unwrap();
        g_owner.add_member(&owner.id, member(&stolen), 2).unwrap();
        let mut g_stolen = replica(&g_owner, &stolen);

        let (removal, key) = g_owner
            .remove_member(&owner.id, &stolen.id.public(), 3)
            .unwrap();
        let sneak = g_stolen
            .add_member(&stolen.id, member(&device(6)), 3)
            .unwrap();

        // The owner keeps its removal, whichever arrives first...
        assert_eq!(
            g_owner.apply_roster(sneak.clone()).unwrap(),
            RosterUpdate::ForkLost
        );
        assert!(g_owner.current_key().is_some());
        // ...and the stolen device, applying the owner's roster, loses.
        assert_eq!(
            g_stolen.apply_roster(removal).unwrap(),
            RosterUpdate::RemovedThisDevice
        );
        assert!(g_stolen.accept_key(key).is_err());
    }

    #[test]
    fn concurrent_adds_converge_and_the_loser_reapplies() {
        let a = device(1);
        let b = device(2);
        let x = device(3);
        let y = device(4);
        let mut ga = GroupState::create(&a.id, &a.device_id, &a.name, 1).unwrap();
        ga.add_member(&a.id, member(&b), 2).unwrap();
        let mut gb = replica(&ga, &b);

        let ra = ga.add_member(&a.id, member(&x), 3).unwrap();
        let rb = gb.add_member(&b.id, member(&y), 3).unwrap();

        let ua = ga.apply_roster(rb.clone()).unwrap();
        let ub = gb.apply_roster(ra.clone()).unwrap();
        // Exactly one side lost, and both now hold the same roster.
        let (loser, loser_dev, winner) = match (&ua, &ub) {
            (RosterUpdate::Replaced { .. }, RosterUpdate::ForkLost) => (&mut ga, &a, &mut gb),
            (RosterUpdate::ForkLost, RosterUpdate::Replaced { .. }) => (&mut gb, &b, &mut ga),
            other => panic!("unexpected fork outcome {other:?}"),
        };
        assert_eq!(loser.current().hash(), winner.current().hash());

        let reapply = match if std::ptr::eq(loser_dev, &a) { ua } else { ub } {
            RosterUpdate::Replaced { reapply } => reapply,
            _ => unreachable!(),
        };
        assert_eq!(reapply.len(), 1);
        let redone = loser.reapply(&loser_dev.id, reapply, 4).unwrap();
        assert_eq!(redone.len(), 1);
        assert_eq!(
            winner.apply_roster(redone[0].0.clone()).unwrap(),
            RosterUpdate::Advanced
        );
        assert_eq!(winner.current().members.len(), 4);
        assert_eq!(loser.current().hash(), winner.current().hash());
    }

    #[test]
    fn fork_winner_is_symmetric() {
        let a = device(1);
        let b = device(2);
        let mut ga = GroupState::create(&a.id, &a.device_id, &a.name, 1).unwrap();
        ga.add_member(&a.id, member(&b), 2).unwrap();
        let parent = ga.current().clone();
        let mut gb = replica(&ga, &b);
        let x = ga.add_member(&a.id, member(&device(3)), 3).unwrap();
        let y = gb.add_member(&b.id, member(&device(4)), 3).unwrap();
        let one = std::slice::from_ref;
        assert_ne!(
            fork_winner(&parent, one(&x), one(&y)),
            fork_winner(&parent, one(&y), one(&x))
        );
        // Mutual removal falls back to the hash, still symmetric.
        let mut ga2 = replica(&ga, &a);
        ga2.rosters = vec![parent.clone()];
        let mut gb2 = replica(&ga, &b);
        gb2.rosters = vec![parent.clone()];
        ga2.keys = ga.keys.clone();
        gb2.keys = ga.keys.clone();
        let (x, _) = ga2.remove_member(&a.id, &b.id.public(), 3).unwrap();
        let (y, _) = gb2.remove_member(&b.id, &a.id.public(), 3).unwrap();
        // Each removes the other's signer: every device keeps what it has.
        assert!(!fork_winner(&parent, one(&x), one(&y)));
        assert!(!fork_winner(&parent, one(&y), one(&x)));
    }

    #[test]
    fn seal_open_and_old_keys_are_refused() {
        let a = device(1);
        let b = device(2);
        let c = device(3);
        let mut ga = GroupState::create(&a.id, &a.device_id, &a.name, 1).unwrap();
        ga.add_member(&a.id, member(&b), 2).unwrap();
        ga.add_member(&a.id, member(&c), 3).unwrap();
        let mut gb = replica(&ga, &b);
        let gc = replica(&ga, &c);

        let record = SyncRecord {
            content_hash: panora_core::sync::payload_hash(&[MimePayload::new("text/plain", "hi")]),
            selection: Selection::Clipboard,
            source_app: None,
            created_at: 1,
            last_seen_at: 1,
            pinned: false,
            deleted: false,
            device_id: a.device_id.clone(),
            lamport: 7,
            payloads: vec![MimePayload::new("text/plain", "hi")],
        };
        let sealed = ga.seal_record(&record).unwrap();
        assert_eq!(gb.open_record(&sealed).unwrap(), record);
        // Context binding.
        assert!(gb.open(b"something-else", &sealed).is_err());
        let mut tampered = sealed.clone();
        *tampered.ciphertext.last_mut().unwrap() ^= 1;
        assert!(gb.open_record(&tampered).is_err());

        // C is removed; what C seals with the old key is no longer accepted.
        let (r, key) = ga.remove_member(&a.id, &c.id.public(), 4).unwrap();
        gb.apply_roster(r).unwrap();
        gb.accept_key(key).unwrap();
        let from_c = gc.seal_record(&record).unwrap();
        assert!(ga.open_record(&from_c).is_err());
        assert!(gb.open_record(&from_c).is_err());
        let fresh = ga.seal_record(&record).unwrap();
        assert_eq!(gb.open_record(&fresh).unwrap(), record);
    }

    #[test]
    fn state_round_trips_and_tampering_is_caught() {
        let a = device(1);
        let mut g = GroupState::create(&a.id, &a.device_id, &a.name, 1).unwrap();
        g.add_member(&a.id, member(&device(2)), 2).unwrap();
        let json = serde_json::to_string(&g).unwrap();
        let back: GroupState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.current().hash(), g.current().hash());
        assert!(back.current_key().is_some());

        let tampered = json.replace("device 2", "device X");
        assert!(serde_json::from_str::<GroupState>(&tampered).is_err());
    }

    #[test]
    fn roster_history_is_bounded() {
        let a = device(1);
        let mut g = GroupState::create(&a.id, &a.device_id, &a.name, 1).unwrap();
        for n in 2..14 {
            g.add_member(&a.id, member(&device(n)), n as i64).unwrap();
        }
        assert_eq!(g.rosters.len(), KEPT_ROSTERS);
        assert!(g.current_key().is_some());
        let json = serde_json::to_string(&g).unwrap();
        assert!(serde_json::from_str::<GroupState>(&json).is_ok());
    }

    /// O creates the group, then adds P, T and Q, then removes T. Returns
    /// the devices, O's state, and T's state as of just after T joined.
    fn stolen_setup() -> (Device, Device, Device, GroupState, GroupState, GroupState) {
        let o = device(1);
        let p = device(2);
        let t = device(3);
        let mut go = GroupState::create(&o.id, &o.device_id, &o.name, 1).unwrap();
        go.add_member(&o.id, member(&p), 2).unwrap();
        let gp = replica(&go, &p);
        go.add_member(&o.id, member(&t), 3).unwrap();
        let gt = replica(&go, &t);
        go.add_member(&o.id, member(&device(4)), 4).unwrap();
        go.remove_member(&o.id, &t.id.public(), 5).unwrap();
        (o, p, t, go, gp, gt)
    }

    #[test]
    fn a_removed_device_cannot_win_back_with_an_older_fork() {
        let (o, _p, t, go, mut gp, gt) = stolen_setup();
        gp.apply_chain(go.welcome_chain()).unwrap();
        gp.accept_key(go.current_key().unwrap().clone()).unwrap();
        assert!(!gp.is_peer(&t.id.public()));
        // T grinds: many sibling rosters on the parent where it was still a
        // member, each removing O (and so rotating to a key T knows).
        for attempt in 0..40 {
            let mut fork = gt.clone();
            let (evil, _) = fork
                .remove_member(&t.id, &o.id.public(), 100 + attempt)
                .unwrap();
            for g in [&mut go.clone(), &mut gp] {
                assert_eq!(
                    g.apply_roster(evil.clone()).unwrap(),
                    RosterUpdate::ForkLost
                );
                assert!(!g.is_peer(&t.id.public()));
                assert!(g.current_key().is_some());
            }
        }
        // Same one level later, as a sibling of the removal itself.
        let mut late = gt.clone();
        late.apply_chain(go.welcome_chain()[..4].to_vec()).unwrap();
        let (evil, _) = late.remove_member(&t.id, &o.id.public(), 200).unwrap();
        assert_eq!(gp.apply_roster(evil).unwrap(), RosterUpdate::ForkLost);
    }

    #[test]
    fn a_device_that_missed_the_removal_follows_it_once_it_sees_the_branch() {
        let (o, p, t, go, _gp, gt) = stolen_setup();
        // P' has everything up to Q's addition but not T's removal yet.
        let mut behind = replica(&go, &p);
        behind.rosters.truncate(behind.rosters.len() - 1);
        behind.keys = go.keys.clone();
        behind.prune();
        let _ = o;
        // T races with a harmless-looking sibling that adds a device of its
        // own; whether it wins there is a coin flip...
        let mut fork = gt.clone();
        let evil = fork.add_member(&t.id, member(&device(9)), 50).unwrap();
        let first = behind.apply_roster(evil).unwrap();
        assert!(matches!(
            first,
            RosterUpdate::ForkLost | RosterUpdate::Replaced { .. }
        ));
        // ...but the owner's branch removes T, so P' ends up on it.
        let chain: Vec<Roster> = go.welcome_chain()[3..].to_vec();
        behind.apply_chain(chain).unwrap();
        assert_eq!(behind.current().hash(), go.current().hash());
        assert!(!behind.is_peer(&t.id.public()));
    }

    #[test]
    fn reapply_does_not_bring_back_a_device_removed_later_on_the_losing_branch() {
        // A adds X and B then removes X on top of that, while C, offline,
        // makes its own change on the same parent and wins the fork. Retry
        // with fresh devices until C wins (a coin flip per identity).
        for _ in 0..64 {
            let (a, b, c, x) = (device(1), device(2), device(3), device(4));
            let mut ga = GroupState::create(&a.id, &a.device_id, &a.name, 1).unwrap();
            ga.add_member(&a.id, member(&b), 2).unwrap();
            ga.add_member(&a.id, member(&c), 3).unwrap();
            let parent = ga.current().clone();
            let mut gb = replica(&ga, &b);
            let mut gc = replica(&ga, &c);
            let ra = ga.add_member(&a.id, member(&x), 4).unwrap();
            gb.apply_roster(ra.clone()).unwrap();
            let (rb, _) = gb.remove_member(&b.id, &x.id.public(), 5).unwrap();
            ga.apply_roster(rb.clone()).unwrap();
            let rc = gc.add_member(&c.id, member(&device(5)), 4).unwrap();
            if !fork_winner(&parent, std::slice::from_ref(&rc), &[ra, rb]) {
                continue;
            }
            let RosterUpdate::Replaced { reapply } = ga.apply_roster(rc.clone()).unwrap() else {
                panic!("C's branch should win on A");
            };
            // A's addition of X was undone by B later on: A must not add it
            // back, and B's removal has nothing left to remove.
            assert!(!reapply.contains(&Change::Added(member(&x))), "{reapply:?}");
            assert!(ga.reapply(&a.id, reapply, 6).unwrap().is_empty());
            let RosterUpdate::Replaced { reapply } = gb.apply_roster(rc).unwrap() else {
                panic!("C's branch should win on B");
            };
            assert_eq!(reapply, vec![Change::Removed(x.id.public())]);
            // X is not in the winning roster, so there is nothing to remove.
            assert!(gb.reapply(&b.id, reapply, 6).unwrap().is_empty());
            assert!(!ga.is_peer(&x.id.public()) && !gb.is_peer(&x.id.public()));
            assert_eq!(ga.current().hash(), gb.current().hash());
            return;
        }
        panic!("C never won the fork in 64 tries");
    }

    #[test]
    fn a_newcomer_judges_forks_with_the_history_it_was_given() {
        let a = device(1);
        let b = device(2);
        let e = device(5);
        let mut ga = GroupState::create(&a.id, &a.device_id, &a.name, 1).unwrap();
        ga.add_member(&a.id, member(&b), 2).unwrap();
        let mut gb = replica(&ga, &b);
        ga.add_member(&a.id, member(&device(3)), 3).unwrap();
        ga.add_member(&a.id, member(&e), 4).unwrap();
        let mut ge = replica(&ga, &e);
        assert_eq!(ge.rosters.len(), 4);

        // B, offline, made a concurrent change two epochs back.
        let rb = gb.add_member(&b.id, member(&device(6)), 3).unwrap();
        let on_a = ga.apply_roster(rb.clone()).unwrap();
        let on_e = ge.apply_roster(rb).unwrap();
        assert_eq!(ga.current().hash(), ge.current().hash());
        match (on_a, on_e) {
            (RosterUpdate::ForkLost, RosterUpdate::ForkLost) => {}
            (RosterUpdate::Replaced { reapply }, RosterUpdate::Replaced { .. }) => {
                // E was added on the losing branch: waiting, not removed.
                assert!(!ge.is_member());
                assert!(ge.current_key().is_some());
                for (r, _) in ga.reapply(&a.id, reapply, 5).unwrap() {
                    ge.apply_roster(r).unwrap();
                }
                assert!(ge.is_member());
            }
            other => panic!("devices disagree: {other:?}"),
        }

        // With only the last two rosters (what an older welcome gave), a
        // fork that deep cannot be judged, and says so.
        let mut short = ge.clone();
        short.rosters.drain(..short.rosters.len() - 2);
        let mut gb2 = replica(&ga, &b);
        gb2.rosters.truncate(gb2.rosters.len() - 2);
        gb2.keys = ga.keys.clone();
        gb2.prune();
        let deep = gb2.add_member(&b.id, member(&device(7)), 9).unwrap();
        assert!(short.apply_roster(deep).is_err());
    }

    /// O, P, T and Y are members; O removes T and P applies it. Y, offline,
    /// made its own change on the old parent.
    struct Race {
        o: Device,
        p: Device,
        t: Device,
        y: Device,
        go: GroupState,
        gp: GroupState,
        gt: GroupState,
        gy: GroupState,
        removal: Roster,
    }

    fn race() -> Race {
        let (o, p, t, y) = (device(1), device(2), device(3), device(4));
        let mut go = GroupState::create(&o.id, &o.device_id, &o.name, 1).unwrap();
        go.add_member(&o.id, member(&p), 2).unwrap();
        go.add_member(&o.id, member(&t), 3).unwrap();
        go.add_member(&o.id, member(&y), 4).unwrap();
        let mut gp = replica(&go, &p);
        let gt = replica(&go, &t);
        let gy = replica(&go, &y);
        let (removal, key) = go.remove_member(&o.id, &t.id.public(), 5).unwrap();
        assert_eq!(
            gp.apply_roster(removal.clone()).unwrap(),
            RosterUpdate::Advanced
        );
        gp.accept_key(key).unwrap();
        Race {
            o,
            p,
            t,
            y,
            go,
            gp,
            gt,
            gy,
            removal,
        }
    }

    #[test]
    fn an_offline_devices_unrelated_change_does_not_readmit_a_removed_one() {
        for _ in 0..32 {
            let Race {
                t,
                y,
                mut go,
                mut gp,
                mut gy,
                removal,
                ..
            } = race();
            let ry = gy.add_member(&y.id, member(&device(5)), 5).unwrap();
            // The removal beats a branch that removes nobody, on every device.
            assert_eq!(gp.apply_roster(ry.clone()).unwrap(), RosterUpdate::ForkLost);
            assert_eq!(go.apply_roster(ry).unwrap(), RosterUpdate::ForkLost);
            assert!(!gp.is_peer(&t.id.public()));
            assert!(gp.key_for_peer(&t.id.public()).is_err());
            // Y switches to the removal and gets its own addition back.
            let RosterUpdate::Replaced { reapply } = gy.apply_roster(removal).unwrap() else {
                panic!("Y should follow the removal");
            };
            assert!(!gy.is_peer(&t.id.public()));
            assert_eq!(reapply.len(), 1);
            assert!(matches!(reapply[0], Change::Added(_)));
        }
    }

    #[test]
    fn a_removal_that_loses_a_tie_break_is_redone_by_whoever_sees_it() {
        // Y, offline, removes P while O removes T. Both branches remove a
        // device the other still lists, so the hash decides; retry until
        // Y's branch wins.
        for _ in 0..64 {
            let Race {
                o,
                p,
                t,
                y,
                mut go,
                mut gp,
                mut gt,
                mut gy,
                removal,
            } = race();
            let (ry, _) = gy.remove_member(&y.id, &p.id.public(), 5).unwrap();
            let parent = go.welcome_chain()[3].clone();
            if !fork_winner(
                &parent,
                std::slice::from_ref(&ry),
                std::slice::from_ref(&removal),
            ) {
                continue;
            }
            // P is removed on the winning branch: it learns so and stops.
            assert_eq!(
                gp.apply_roster(ry.clone()).unwrap(),
                RosterUpdate::RemovedThisDevice
            );
            // O's removal of T lost the tie-break; it comes back to redo.
            let RosterUpdate::Replaced { reapply } = go.apply_roster(ry.clone()).unwrap() else {
                panic!("Y's branch should win on O");
            };
            assert!(reapply.contains(&Change::Removed(t.id.public())));
            // Redoing it shuts T out again, whatever T signs meanwhile.
            let redone = go.reapply(&o.id, reapply, 6).unwrap();
            assert_eq!(redone.len(), 1);
            assert!(!go.is_peer(&t.id.public()));
            assert_eq!(gt.apply_roster(ry.clone()).unwrap(), RosterUpdate::Advanced);
            let (evil, _) = gt.remove_member(&t.id, &o.id.public(), 7).unwrap();
            assert_eq!(
                go.apply_roster(evil.clone()).unwrap(),
                RosterUpdate::ForkLost
            );
            assert!(!go.is_peer(&t.id.public()));
            // Y, which holds only its own branch, also takes the re-removal.
            assert_eq!(
                gy.apply_roster(redone[0].0.clone()).unwrap(),
                RosterUpdate::Advanced
            );
            assert_eq!(gy.apply_roster(evil).unwrap(), RosterUpdate::ForkLost);
            assert!(!gy.is_peer(&t.id.public()));
            return;
        }
        panic!("Y's branch never won the tie-break in 64 tries");
    }

    #[test]
    fn a_matching_chain_from_a_peer_that_is_behind_is_unchanged() {
        let a = device(1);
        let mut g = GroupState::create(&a.id, &a.device_id, &a.name, 1).unwrap();
        for n in 2..12 {
            g.add_member(&a.id, member(&device(n)), n as i64).unwrap();
        }
        let behind = g.rosters.clone();
        let mut ahead = g.clone();
        for n in 12..15 {
            ahead
                .add_member(&a.id, member(&device(n)), n as i64)
                .unwrap();
        }
        assert_eq!(ahead.apply_chain(behind).unwrap(), RosterUpdate::Unchanged);
    }

    #[test]
    fn epochs_are_bounded() {
        let a = device(1);
        let g = GroupState::create(&a.id, &a.device_id, &a.name, 1).unwrap();
        let mut r = g.current().clone();
        r.epoch = MAX_EPOCH + 1;
        r.prev = [1; 32];
        let r = r.signed(&a.id);
        assert!(r.verify().is_err());
        let mut r = g.current().clone();
        r.epoch = u64::MAX;
        r.prev = [1; 32];
        assert!(r.signed(&a.id).verify().is_err());
    }

    #[test]
    fn names_and_ids_are_validated() {
        assert!(validate_name("Yusuf's laptop").is_ok());
        assert!(validate_name("Çalışma masası").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name(" padded").is_err());
        assert!(validate_name("line\nbreak").is_err());
        assert!(validate_name("evil\u{202E}pot.exe").is_err());
        // Invisible or line-breaking characters that could fake another
        // device's name or a line of a pairing prompt.
        for sneaky in [
            "\u{2028}",
            "\u{2029}",
            "\u{200B}",
            "\u{2060}",
            "\u{FEFF}",
            "\u{00AD}",
            "\u{E0041}",
        ] {
            assert!(
                validate_name(&format!("lap{sneaky}top")).is_err(),
                "{sneaky:?}"
            );
        }
        assert!(validate_name(&"x".repeat(65)).is_err());
        assert!(validate_device_id("0123abcd").is_ok());
        assert!(validate_device_id("").is_err());
        assert!(validate_device_id("a b").is_err());
        assert!(validate_device_id(&"a".repeat(65)).is_err());
    }
}
