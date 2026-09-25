// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! The pairing protocol, `panora-pair/1` (ADR 0005), as two sans-IO state
//! machines: [`Joiner`] (the new device) and [`Inviter`] (a member). They
//! only turn messages into messages; [`crate::wire`] runs them over a byte
//! stream, and SYNC-04 supplies the stream.
//!
//! ```text
//! Joiner                                   Inviter
//!   Commit { mode, H(eJ) }          ─────►
//!                                   ◄─────  Offer { eI, identity I, group id }
//!   Reveal { eJ }                   ─────►  (checks H(eJ))
//!        both: shared = X25519(eI, eJ), th = H(transcript)
//!        keys = KDF(shared ‖ invitation secret or zeros ‖ th)
//!        code mode: both screens show the same six digits
//!   Join (sealed) { identity J, device id, name, sig_J(th) } ─────►
//!                                   ◄─────  Welcome (sealed) { sig_I(th), rosters, group key }
//! ```
//!
//! *Invitation mode*: the joiner holds the inviter's QR code / link, which
//! pins the inviter's identity key and carries a 256-bit one-time secret
//! mixed into the keys. Only a device that read the invitation can seal a
//! `Join` the inviter can open; only the pinned inviter can sign the
//! `Welcome`.
//!
//! *Code mode* (no camera, no way to copy a link): nothing is shared in
//! advance, so both devices show a six-digit code derived from the keys and
//! their users confirm the codes match. The joiner commits to its
//! ephemeral key before it sees the inviter's, so a machine in the middle
//! has to fix its keys on both sides before learning what either code will
//! be, and gets a one-in-a-million chance per attempt. Posing as the joiner
//! it can learn the inviter's code before revealing its own key and drop
//! the session, so the inviter's [`crate::PairingWindow`] counts every
//! session that starts, finished or not, and allows three.
//!
//! The mode is part of the transcript and each side accepts only the mode
//! its user chose, so neither can be downgraded to the other.

use crate::bytes::{b64, b64_vec, put};
use crate::error::{Error, Result};
use crate::group::{
    check_successor, validate_device_id, validate_name, GroupId, GroupKey, GroupState, Member,
    Roster,
};
use crate::identity::{DeviceIdentity, PublicIdentity, SIGNATURE_LEN};
use crate::invite::{Invitation, PairingWindow};
use panora_core::storage::{Cipher, MasterKey};
use ring::agreement::{self, EphemeralPrivateKey, UnparsedPublicKey, X25519};
use ring::rand::SystemRandom;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// Protocol name and version, sent in the first message and mixed into
/// every derivation.
pub const PROTOCOL: &str = "panora-pair/1";

/// How the two devices authenticate each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingMode {
    /// The joiner has the inviter's QR code or link.
    Invitation,
    /// The users compare a six-digit code on both screens.
    Code,
}

/// Why a device ended the pairing; sent in [`PairMessage::Abort`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbortReason {
    /// The user said no, or the codes did not match.
    Rejected,
    /// The devices were set to different modes.
    WrongMode,
    /// The inviter is not accepting devices (no window open, or it expired).
    Closed,
    /// The group cannot take this device (already a member, device id in
    /// use, group full).
    Refused,
    /// A protocol or authentication check failed.
    Failed,
}

impl std::fmt::Display for AbortReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            AbortReason::Rejected => "rejected",
            AbortReason::WrongMode => "the devices are in different pairing modes",
            AbortReason::Closed => "pairing is not open on the other device",
            AbortReason::Refused => "the group cannot take this device",
            AbortReason::Failed => "verification failed",
        })
    }
}

/// A pairing protocol message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PairMessage {
    /// Joiner → inviter: protocol, mode and a commitment to its ephemeral key.
    Commit {
        /// Must be [`PROTOCOL`].
        protocol: String,
        /// The joiner's mode.
        mode: PairingMode,
        /// Hash of the joiner's ephemeral public key.
        #[serde(with = "b64")]
        commitment: [u8; 32],
    },
    /// Inviter → joiner: its ephemeral key, identity and group.
    Offer {
        /// Inviter's X25519 ephemeral public key.
        #[serde(with = "b64")]
        ephemeral: [u8; 32],
        /// Inviter's identity key.
        inviter: PublicIdentity,
        /// The group being joined.
        #[serde(with = "b64")]
        group_id: GroupId,
    },
    /// Joiner → inviter: the committed ephemeral key.
    Reveal {
        /// Joiner's X25519 ephemeral public key.
        #[serde(with = "b64")]
        ephemeral: [u8; 32],
    },
    /// Joiner → inviter, sealed: who the joiner is.
    Join {
        /// AEAD envelope of the join body.
        #[serde(with = "b64_vec")]
        sealed: Vec<u8>,
    },
    /// Inviter → joiner, sealed: the group.
    Welcome {
        /// AEAD envelope of the welcome body.
        #[serde(with = "b64_vec")]
        sealed: Vec<u8>,
    },
    /// Either side: pairing ends.
    Abort {
        /// Why.
        reason: AbortReason,
    },
}

/// The six-digit code both screens show in code mode.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SasCode(u32);

impl SasCode {
    /// The code as a number below 1 000 000.
    pub fn value(&self) -> u32 {
        self.0
    }
}

impl std::fmt::Display for SasCode {
    /// `042 917`: two groups of three, leading zeros kept.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:03} {:03}", self.0 / 1000, self.0 % 1000)
    }
}

/// What a joining device says about itself; shown to the inviter's user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinRequest {
    /// Its identity key.
    pub identity: PublicIdentity,
    /// Its `panod` device id.
    pub device_id: String,
    /// Its name.
    pub name: String,
}

#[derive(Serialize, Deserialize)]
struct JoinBody {
    identity: PublicIdentity,
    device_id: String,
    name: String,
    #[serde(with = "b64")]
    signature: [u8; SIGNATURE_LEN],
}

#[derive(Serialize, Deserialize)]
struct WelcomeBody {
    #[serde(with = "b64")]
    signature: [u8; SIGNATURE_LEN],
    rosters: Vec<Roster>,
    key: GroupKey,
}

struct Ephemeral {
    private: EphemeralPrivateKey,
    public: [u8; 32],
}

impl Ephemeral {
    fn generate() -> Result<Self> {
        let private = EphemeralPrivateKey::generate(&X25519, &SystemRandom::new())
            .map_err(|_| Error::Crypto)?;
        let public = private
            .compute_public_key()
            .map_err(|_| Error::Crypto)?
            .as_ref()
            .try_into()
            .map_err(|_| Error::Crypto)?;
        Ok(Self { private, public })
    }

    /// X25519 with the peer's key. `ring` refuses a peer key that yields an
    /// all-zero secret (a low-order point), which is how a machine in the
    /// middle would try to force a known key.
    fn agree(self, peer: &[u8; 32]) -> Result<Zeroizing<[u8; 32]>> {
        agreement::agree_ephemeral(
            self.private,
            &UnparsedPublicKey::new(&X25519, peer),
            |shared| {
                let mut out = Zeroizing::new([0u8; 32]);
                out.copy_from_slice(shared);
                out
            },
        )
        .map_err(|_| Error::Auth("the other device sent an invalid key"))
    }
}

fn commitment(ephemeral: &[u8; 32]) -> [u8; 32] {
    blake3::derive_key("panora-pair/1 commitment", ephemeral)
}

/// Transcript fields, in protocol order.
struct Transcript<'t> {
    mode: PairingMode,
    commitment: &'t [u8; 32],
    inviter_ephemeral: &'t [u8; 32],
    inviter: &'t PublicIdentity,
    group_id: &'t GroupId,
    joiner_ephemeral: &'t [u8; 32],
}

/// Keys of one pairing session.
struct Session {
    th: [u8; 32],
    joiner_to_inviter: Zeroizing<[u8; 32]>,
    inviter_to_joiner: Zeroizing<[u8; 32]>,
    code: SasCode,
}

impl Session {
    fn derive(t: Transcript<'_>, shared: &[u8; 32], secret: Option<&[u8; 32]>) -> Self {
        let mut transcript = PROTOCOL.as_bytes().to_vec();
        put(
            &mut transcript,
            match t.mode {
                PairingMode::Invitation => b"invitation",
                PairingMode::Code => b"code",
            },
        );
        put(&mut transcript, t.commitment);
        put(&mut transcript, t.inviter_ephemeral);
        put(&mut transcript, t.inviter.as_bytes());
        put(&mut transcript, t.group_id);
        put(&mut transcript, t.joiner_ephemeral);
        let th = blake3::derive_key("panora-pair/1 transcript", &transcript);

        let mut ikm = Zeroizing::new(Vec::with_capacity(96));
        ikm.extend_from_slice(shared);
        ikm.extend_from_slice(secret.unwrap_or(&[0; 32]));
        ikm.extend_from_slice(&th);
        let prk = Zeroizing::new(blake3::derive_key("panora-pair/1 session key", &ikm));
        let sub = |label: &[u8]| Zeroizing::new(*blake3::keyed_hash(&prk, label).as_bytes());
        let raw = sub(b"short authentication string");
        let mut first = [0u8; 8];
        first.copy_from_slice(&raw[..8]);
        Self {
            th,
            joiner_to_inviter: sub(b"joiner to inviter"),
            inviter_to_joiner: sub(b"inviter to joiner"),
            code: SasCode((u64::from_le_bytes(first) % 1_000_000) as u32),
        }
    }

    fn signed_context(&self, label: &[u8]) -> Vec<u8> {
        let mut out = PROTOCOL.as_bytes().to_vec();
        put(&mut out, label);
        put(&mut out, &self.th);
        out
    }
}

fn seal<T: Serialize>(key: &[u8; 32], th: &[u8; 32], body: &T) -> Result<Vec<u8>> {
    let json = Zeroizing::new(serde_json::to_vec(body)?);
    Ok(Cipher::new(&MasterKey::from_bytes(*key)).seal_with_aad(th, &json)?)
}

fn open<T: DeserializeOwned>(
    key: &[u8; 32],
    th: &[u8; 32],
    sealed: &[u8],
    failure: &'static str,
) -> Result<T> {
    let json = Zeroizing::new(
        Cipher::new(&MasterKey::from_bytes(*key))
            .open_with_aad(th, sealed)
            .map_err(|_| Error::Auth(failure))?,
    );
    Ok(serde_json::from_slice(&json)?)
}

fn out_of_order() -> Error {
    Error::Protocol("unexpected message, or pairing already ended")
}

fn check_abort(msg: &PairMessage) -> Result<()> {
    match msg {
        PairMessage::Abort { reason } => Err(Error::Aborted(*reason)),
        _ => Ok(()),
    }
}

enum JoinerState {
    Committed {
        ephemeral: Ephemeral,
        commitment: [u8; 32],
    },
    Keyed {
        session: Session,
        inviter: PublicIdentity,
        group_id: GroupId,
        sent_join: bool,
    },
}

/// The new device's side. Any error ends the pairing: a later call fails.
pub struct Joiner<'a> {
    identity: &'a DeviceIdentity,
    device_id: String,
    name: String,
    invitation: Option<Invitation>,
    state: Option<JoinerState>,
}

impl<'a> Joiner<'a> {
    /// Begin joining, with the inviter's invitation (invitation mode) or
    /// without one (code mode). Returns the first message to send.
    pub fn start(
        identity: &'a DeviceIdentity,
        device_id: &str,
        name: &str,
        invitation: Option<Invitation>,
        now: i64,
    ) -> Result<(Self, PairMessage)> {
        validate_device_id(device_id)?;
        validate_name(name)?;
        if let Some(inv) = &invitation {
            if inv.is_expired(now) {
                return Err(Error::Expired("the invitation has expired"));
            }
            if inv.inviter == identity.public() {
                return Err(Error::Invitation("this invitation was made on this device"));
            }
        }
        let ephemeral = Ephemeral::generate()?;
        let commitment = commitment(&ephemeral.public);
        let joiner = Self {
            identity,
            device_id: device_id.to_string(),
            name: name.to_string(),
            invitation,
            state: Some(JoinerState::Committed {
                ephemeral,
                commitment,
            }),
        };
        let msg = PairMessage::Commit {
            protocol: PROTOCOL.to_string(),
            mode: joiner.mode(),
            commitment,
        };
        Ok((joiner, msg))
    }

    /// The mode this joiner pairs in.
    pub fn mode(&self) -> PairingMode {
        if self.invitation.is_some() {
            PairingMode::Invitation
        } else {
            PairingMode::Code
        }
    }

    /// Handle the inviter's offer; returns the reveal to send.
    pub fn on_offer(&mut self, msg: PairMessage) -> Result<PairMessage> {
        let state = self.state.take();
        check_abort(&msg)?;
        let (
            Some(JoinerState::Committed {
                ephemeral,
                commitment,
            }),
            PairMessage::Offer {
                ephemeral: inviter_ephemeral,
                inviter,
                group_id,
            },
        ) = (state, msg)
        else {
            return Err(out_of_order());
        };
        if inviter == self.identity.public() {
            return Err(Error::Auth(
                "the other device claims this device's identity",
            ));
        }
        if let Some(inv) = &self.invitation {
            if inv.inviter != inviter {
                return Err(Error::Auth(
                    "the device that answered is not the one that made the invitation",
                ));
            }
        }
        let joiner_ephemeral = ephemeral.public;
        let shared = ephemeral.agree(&inviter_ephemeral)?;
        let session = Session::derive(
            Transcript {
                mode: self.mode(),
                commitment: &commitment,
                inviter_ephemeral: &inviter_ephemeral,
                inviter: &inviter,
                group_id: &group_id,
                joiner_ephemeral: &joiner_ephemeral,
            },
            &shared,
            self.invitation.as_ref().map(Invitation::secret),
        );
        self.state = Some(JoinerState::Keyed {
            session,
            inviter,
            group_id,
            sent_join: false,
        });
        Ok(PairMessage::Reveal {
            ephemeral: joiner_ephemeral,
        })
    }

    /// The code to show and compare, in code mode, once the offer is in.
    pub fn code(&self) -> Option<SasCode> {
        match (&self.state, self.mode()) {
            (Some(JoinerState::Keyed { session, .. }), PairingMode::Code) => Some(session.code),
            _ => None,
        }
    }

    /// The inviter's identity, once the offer is in.
    pub fn inviter(&self) -> Option<PublicIdentity> {
        match &self.state {
            Some(JoinerState::Keyed { inviter, .. }) => Some(*inviter),
            _ => None,
        }
    }

    /// Go ahead: in code mode, the user confirmed the codes match. Returns
    /// the sealed join message.
    pub fn confirm(&mut self) -> Result<PairMessage> {
        let Some(JoinerState::Keyed {
            session, sent_join, ..
        }) = &mut self.state
        else {
            return Err(out_of_order());
        };
        if *sent_join {
            return Err(out_of_order());
        }
        *sent_join = true;
        let body = JoinBody {
            identity: self.identity.public(),
            device_id: self.device_id.clone(),
            name: self.name.clone(),
            signature: self.identity.sign(&session.signed_context(b"join")),
        };
        let sealed = seal(&session.joiner_to_inviter, &session.th, &body)?;
        Ok(PairMessage::Join { sealed })
    }

    /// Stop: the user declined or the codes differ. Returns the abort to send.
    pub fn reject(&mut self) -> PairMessage {
        self.state = None;
        PairMessage::Abort {
            reason: AbortReason::Rejected,
        }
    }

    /// Handle the welcome; returns this device's new group.
    pub fn on_welcome(&mut self, msg: PairMessage) -> Result<GroupState> {
        let state = self.state.take();
        check_abort(&msg)?;
        let (
            Some(JoinerState::Keyed {
                session,
                inviter,
                group_id,
                sent_join: true,
            }),
            PairMessage::Welcome { sealed },
        ) = (state, msg)
        else {
            return Err(out_of_order());
        };
        let body: WelcomeBody = open(
            &session.inviter_to_joiner,
            &session.th,
            &sealed,
            "could not open the welcome",
        )?;
        inviter.verify(&session.signed_context(b"welcome"), &body.signature)?;
        let [.., parent, current] = body.rosters.as_slice() else {
            return Err(Error::Protocol("a welcome carries at least two rosters"));
        };
        let me = self.identity.public();
        parent.verify()?;
        current.verify()?;
        check_successor(parent, current)?;
        let entry = current.member(&me);
        let admitted = current.group_id == group_id
            && current.signer == inviter
            && parent.member(&me).is_none()
            && entry.is_some_and(|m| m.device_id == self.device_id && m.name == self.name);
        if !admitted {
            return Err(Error::Roster(
                "the welcome roster does not admit this device as it asked",
            ));
        }
        GroupState::joined(me, body.rosters, body.key)
    }
}

enum InviterState {
    AwaitCommit,
    Offered {
        ephemeral: Ephemeral,
        commitment: [u8; 32],
    },
    Keyed {
        session: Session,
    },
    Pending {
        session: Session,
        request: JoinRequest,
    },
}

/// A member's side, for one incoming device. Any error ends the pairing.
/// It holds the [`PairingWindow`] for the whole session.
pub struct Inviter<'a> {
    identity: &'a DeviceIdentity,
    window: &'a mut PairingWindow,
    group_id: GroupId,
    mode: PairingMode,
    secret: Option<Zeroizing<[u8; 32]>>,
    state: Option<InviterState>,
}

impl<'a> Inviter<'a> {
    /// Start answering one device, if `window` is open at `now`. Uses up
    /// one of the window's attempts.
    pub fn new(
        identity: &'a DeviceIdentity,
        group: &GroupState,
        window: &'a mut PairingWindow,
        now: i64,
    ) -> Result<Self> {
        if group.me() != identity.public() || !group.is_member() {
            return Err(Error::Roster("this device is not a member of the group"));
        }
        let secret = match window.invitation() {
            Some(inv) if inv.inviter != identity.public() => {
                return Err(Error::Invitation(
                    "the invitation was made by another device",
                ));
            }
            Some(inv) => Some(Zeroizing::new(*inv.secret())),
            None => None,
        };
        window.begin(now)?;
        Ok(Self {
            identity,
            mode: window.mode(),
            window,
            group_id: group.group_id(),
            secret,
            state: Some(InviterState::AwaitCommit),
        })
    }

    /// Handle the joiner's commitment; returns the offer to send.
    pub fn on_commit(&mut self, msg: PairMessage) -> Result<PairMessage> {
        let state = self.state.take();
        check_abort(&msg)?;
        let (
            Some(InviterState::AwaitCommit),
            PairMessage::Commit {
                protocol,
                mode,
                commitment,
            },
        ) = (state, msg)
        else {
            return Err(out_of_order());
        };
        if protocol != PROTOCOL {
            return Err(Error::Protocol("unsupported pairing protocol version"));
        }
        if mode != self.mode {
            return Err(Error::ModeMismatch);
        }
        let ephemeral = Ephemeral::generate()?;
        let offer = PairMessage::Offer {
            ephemeral: ephemeral.public,
            inviter: self.identity.public(),
            group_id: self.group_id,
        };
        self.state = Some(InviterState::Offered {
            ephemeral,
            commitment,
        });
        Ok(offer)
    }

    /// Handle the joiner's reveal. In code mode, [`Self::code`] is ready
    /// to show afterwards.
    pub fn on_reveal(&mut self, msg: PairMessage) -> Result<()> {
        let state = self.state.take();
        check_abort(&msg)?;
        let (
            Some(InviterState::Offered {
                ephemeral,
                commitment: committed,
            }),
            PairMessage::Reveal {
                ephemeral: joiner_ephemeral,
            },
        ) = (state, msg)
        else {
            return Err(out_of_order());
        };
        if commitment(&joiner_ephemeral) != committed {
            return Err(Error::Auth("the other device broke its commitment"));
        }
        let inviter_ephemeral = ephemeral.public;
        let shared = ephemeral.agree(&joiner_ephemeral)?;
        let session = Session::derive(
            Transcript {
                mode: self.mode,
                commitment: &committed,
                inviter_ephemeral: &inviter_ephemeral,
                inviter: &self.identity.public(),
                group_id: &self.group_id,
                joiner_ephemeral: &joiner_ephemeral,
            },
            &shared,
            self.secret.as_deref(),
        );
        self.state = Some(InviterState::Keyed { session });
        Ok(())
    }

    /// The code to show and compare, in code mode, after the reveal.
    pub fn code(&self) -> Option<SasCode> {
        match (&self.state, self.mode) {
            (
                Some(InviterState::Keyed { session } | InviterState::Pending { session, .. }),
                PairingMode::Code,
            ) => Some(session.code),
            _ => None,
        }
    }

    /// Handle the joiner's sealed introduction. Returns who is asking, for
    /// the user to approve (code mode) or for the log (invitation mode).
    pub fn on_join(&mut self, msg: PairMessage) -> Result<JoinRequest> {
        let state = self.state.take();
        check_abort(&msg)?;
        let (Some(InviterState::Keyed { session }), PairMessage::Join { sealed }) = (state, msg)
        else {
            return Err(out_of_order());
        };
        let failure = match self.mode {
            PairingMode::Invitation => "the other device does not hold this invitation",
            PairingMode::Code => "the keys differ: the codes did not match",
        };
        let body: JoinBody = open(&session.joiner_to_inviter, &session.th, &sealed, failure)?;
        body.identity
            .verify(&session.signed_context(b"join"), &body.signature)?;
        validate_device_id(&body.device_id)?;
        validate_name(&body.name)?;
        if body.identity == self.identity.public() {
            return Err(Error::Auth(
                "the other device claims this device's identity",
            ));
        }
        let request = JoinRequest {
            identity: body.identity,
            device_id: body.device_id,
            name: body.name,
        };
        self.state = Some(InviterState::Pending {
            session,
            request: request.clone(),
        });
        Ok(request)
    }

    /// Admit the device: adds it to `group`, closes the window, and returns
    /// the welcome to send and the new roster (for the other members).
    pub fn approve(&mut self, group: &mut GroupState, now: i64) -> Result<(PairMessage, Roster)> {
        let Some(InviterState::Pending { session, request }) = self.state.take() else {
            return Err(out_of_order());
        };
        if !self.window.is_live(now) {
            return Err(Error::Invitation("the pairing window closed"));
        }
        if group.group_id() != self.group_id {
            return Err(Error::Roster("the group changed during pairing"));
        }
        let roster = group.add_member(
            self.identity,
            Member {
                identity: request.identity,
                device_id: request.device_id,
                name: request.name,
                added_at: now,
            },
            now,
        )?;
        let key = group.current_key().cloned().ok_or(Error::Roster(
            "this device does not hold the current key yet",
        ))?;
        let body = WelcomeBody {
            signature: self.identity.sign(&session.signed_context(b"welcome")),
            rosters: group.welcome_chain(),
            key,
        };
        let sealed = seal(&session.inviter_to_joiner, &session.th, &body)?;
        self.window.close();
        Ok((PairMessage::Welcome { sealed }, roster))
    }

    /// Decline. Returns the abort to send.
    pub fn reject(&mut self) -> PairMessage {
        self.state = None;
        PairMessage::Abort {
            reason: AbortReason::Rejected,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000;

    struct Dev {
        id: DeviceIdentity,
        device_id: String,
        name: String,
    }

    fn dev(n: u8) -> Dev {
        Dev {
            id: DeviceIdentity::generate().unwrap(),
            device_id: format!("{n:032x}"),
            name: format!("device {n}"),
        }
    }

    fn group(d: &Dev) -> GroupState {
        GroupState::create(&d.id, &d.device_id, &d.name, NOW).unwrap()
    }

    /// Run the whole exchange in memory; `approve` answers both users.
    fn pair(
        inviter: &Dev,
        group: &mut GroupState,
        window: &mut PairingWindow,
        joiner: &Dev,
        invitation: Option<Invitation>,
    ) -> Result<(GroupState, SasCode, SasCode)> {
        let mut i = Inviter::new(&inviter.id, group, window, NOW)?;
        let (mut j, commit) =
            Joiner::start(&joiner.id, &joiner.device_id, &joiner.name, invitation, NOW)?;
        let offer = i.on_commit(commit)?;
        let reveal = j.on_offer(offer)?;
        i.on_reveal(reveal)?;
        let codes = (
            i.code().unwrap_or(SasCode(0)),
            j.code().unwrap_or(SasCode(0)),
        );
        let join = j.confirm()?;
        i.on_join(join)?;
        let (welcome, _) = i.approve(group, NOW)?;
        Ok((j.on_welcome(welcome)?, codes.0, codes.1))
    }

    #[test]
    fn invitation_pairing_builds_the_same_group() {
        let a = dev(1);
        let b = dev(2);
        let mut ga = group(&a);
        let inv = Invitation::new(a.id.public(), vec![], NOW);
        let mut window = PairingWindow::for_invitation(inv.clone());
        let (gb, _, _) = pair(&a, &mut ga, &mut window, &b, Some(inv)).unwrap();
        assert_eq!(ga.current().hash(), gb.current().hash());
        assert_eq!(gb.devices().len(), 2);
        assert!(gb.is_member() && ga.is_peer(&b.id.public()) && gb.is_peer(&a.id.public()));
        let sealed = ga.seal(b"x", b"hello").unwrap();
        assert_eq!(gb.open(b"x", &sealed).unwrap(), b"hello");
    }

    #[test]
    fn code_pairing_shows_the_same_code_on_both_sides() {
        let a = dev(1);
        let b = dev(2);
        let mut ga = group(&a);
        let mut window = PairingWindow::for_code(NOW);
        let (gb, ci, cj) = pair(&a, &mut ga, &mut window, &b, None).unwrap();
        assert_eq!(ci, cj);
        assert_eq!(ci.to_string().len(), 7);
        assert_eq!(ga.current().hash(), gb.current().hash());
    }

    #[test]
    fn wrong_invitation_secret_is_caught_by_the_inviter() {
        let a = dev(1);
        let b = dev(2);
        let mut ga = group(&a);
        let mut window = PairingWindow::for_invitation(Invitation::new(a.id.public(), vec![], NOW));
        // Same inviter key, different secret: an old or forged link.
        let other = Invitation::new(a.id.public(), vec![], NOW);
        let err = pair(&a, &mut ga, &mut window, &b, Some(other)).unwrap_err();
        assert!(matches!(err, Error::Auth(_)), "{err}");
        assert_eq!(ga.current().epoch, 0);
    }

    #[test]
    fn modes_must_match() {
        let a = dev(1);
        let b = dev(2);
        let mut ga = group(&a);
        let mut window = PairingWindow::for_code(NOW);
        let inv = Invitation::new(a.id.public(), vec![], NOW);
        let err = pair(&a, &mut ga, &mut window, &b, Some(inv)).unwrap_err();
        assert!(matches!(err, Error::ModeMismatch));
    }

    #[test]
    fn closed_window_and_expired_invitation_refuse() {
        let a = dev(1);
        let b = dev(2);
        let ga = group(&a);
        let mut window = PairingWindow::for_code(NOW);
        window.close();
        assert!(Inviter::new(&a.id, &ga, &mut window, NOW).is_err());
        let inv = Invitation::new(a.id.public(), vec![], NOW - 10_000);
        assert!(Joiner::start(&b.id, &b.device_id, &b.name, Some(inv), NOW).is_err());
    }

    #[test]
    fn broken_commitment_is_refused() {
        let a = dev(1);
        let b = dev(2);
        let ga = group(&a);
        let mut window = PairingWindow::for_code(NOW);
        let mut i = Inviter::new(&a.id, &ga, &mut window, NOW).unwrap();
        let (_j, commit) = Joiner::start(&b.id, &b.device_id, &b.name, None, NOW).unwrap();
        i.on_commit(commit).unwrap();
        // A different key than the one committed to (a MITM choosing its
        // key after seeing the inviter's).
        let other = Ephemeral::generate().unwrap();
        let err = i
            .on_reveal(PairMessage::Reveal {
                ephemeral: other.public,
            })
            .unwrap_err();
        assert!(matches!(err, Error::Auth(_)));
        // And the session is over.
        assert!(i.on_join(PairMessage::Join { sealed: vec![] }).is_err());
    }

    #[test]
    fn low_order_ephemeral_key_is_refused() {
        let a = dev(1);
        let b = dev(2);
        let ga = group(&a);
        let mut window = PairingWindow::for_code(NOW);
        let mut i = Inviter::new(&a.id, &ga, &mut window, NOW).unwrap();
        let zero = [0u8; 32];
        i.on_commit(PairMessage::Commit {
            protocol: PROTOCOL.into(),
            mode: PairingMode::Code,
            commitment: commitment(&zero),
        })
        .unwrap();
        assert!(matches!(
            i.on_reveal(PairMessage::Reveal { ephemeral: zero }),
            Err(Error::Auth(_))
        ));
        let (mut j, _) = Joiner::start(&b.id, &b.device_id, &b.name, None, NOW).unwrap();
        assert!(j
            .on_offer(PairMessage::Offer {
                ephemeral: zero,
                inviter: a.id.public(),
                group_id: ga.group_id(),
            })
            .is_err());
    }

    #[test]
    fn out_of_order_and_abort_messages_end_the_session() {
        let a = dev(1);
        let b = dev(2);
        let ga = group(&a);
        let mut window = PairingWindow::for_code(NOW);
        let mut i = Inviter::new(&a.id, &ga, &mut window, NOW).unwrap();
        assert!(i
            .on_reveal(PairMessage::Reveal { ephemeral: [1; 32] })
            .is_err());
        let (mut j, _) = Joiner::start(&b.id, &b.device_id, &b.name, None, NOW).unwrap();
        assert!(j.confirm().is_err());
        let err = j
            .on_offer(PairMessage::Abort {
                reason: AbortReason::Closed,
            })
            .unwrap_err();
        assert!(matches!(err, Error::Aborted(AbortReason::Closed)));
        let wrong_version = PairMessage::Commit {
            protocol: "panora-pair/9".into(),
            mode: PairingMode::Code,
            commitment: [0; 32],
        };
        let mut window = PairingWindow::for_code(NOW);
        let mut i = Inviter::new(&a.id, &ga, &mut window, NOW).unwrap();
        assert!(matches!(
            i.on_commit(wrong_version),
            Err(Error::Protocol(_))
        ));
    }

    #[test]
    fn duplicate_device_id_is_refused_at_approval() {
        let a = dev(1);
        let mut b = dev(2);
        b.device_id = a.device_id.clone();
        let mut ga = group(&a);
        let mut window = PairingWindow::for_code(NOW);
        let err = pair(&a, &mut ga, &mut window, &b, None).unwrap_err();
        assert!(matches!(err, Error::Device(_)), "{err}");
        assert_eq!(ga.current().epoch, 0);
    }

    #[test]
    fn code_mode_man_in_the_middle_gets_different_codes() {
        // M sits between J and I and runs the protocol honestly with each,
        // as an inviter towards J and a joiner towards I.
        let i_dev = dev(1);
        let j_dev = dev(2);
        let m_dev = dev(3);
        let gi = group(&i_dev);
        let gm = group(&m_dev);
        let mut window_i = PairingWindow::for_code(NOW);
        let mut window_m = PairingWindow::for_code(NOW);

        let mut real_inviter = Inviter::new(&i_dev.id, &gi, &mut window_i, NOW).unwrap();
        let mut fake_inviter = Inviter::new(&m_dev.id, &gm, &mut window_m, NOW).unwrap();
        let (mut real_joiner, commit_j) =
            Joiner::start(&j_dev.id, &j_dev.device_id, &j_dev.name, None, NOW).unwrap();
        let (mut fake_joiner, commit_m) =
            Joiner::start(&m_dev.id, &m_dev.device_id, &m_dev.name, None, NOW).unwrap();

        let offer_m = fake_inviter.on_commit(commit_j).unwrap();
        let offer_i = real_inviter.on_commit(commit_m).unwrap();
        let reveal_j = real_joiner.on_offer(offer_m).unwrap();
        let reveal_m = fake_joiner.on_offer(offer_i).unwrap();
        fake_inviter.on_reveal(reveal_j).unwrap();
        real_inviter.on_reveal(reveal_m).unwrap();

        // What the two users compare: J's screen against I's screen.
        // Equal only by a one-in-a-million accident.
        assert_ne!(real_joiner.code(), real_inviter.code());
        // J also sees that it is talking to M's key, not I's.
        assert_eq!(real_joiner.inviter(), Some(m_dev.id.public()));
    }

    #[test]
    fn invitation_mode_attacker_without_the_secret_cannot_read_the_join() {
        let i_dev = dev(1);
        let j_dev = dev(2);
        let gi = group(&i_dev);
        let inv = Invitation::new(i_dev.id.public(), vec![], NOW);
        let (mut j, commit) = Joiner::start(
            &j_dev.id,
            &j_dev.device_id,
            &j_dev.name,
            Some(inv.clone()),
            NOW,
        )
        .unwrap();
        let PairMessage::Commit { commitment: c, .. } = commit else {
            unreachable!()
        };
        // The attacker answers in I's name with its own ephemeral key.
        let attacker = Ephemeral::generate().unwrap();
        let attacker_public = attacker.public;
        let reveal = j
            .on_offer(PairMessage::Offer {
                ephemeral: attacker_public,
                inviter: i_dev.id.public(),
                group_id: gi.group_id(),
            })
            .unwrap();
        let PairMessage::Reveal { ephemeral: ej } = reveal else {
            unreachable!()
        };
        let PairMessage::Join { sealed } = j.confirm().unwrap() else {
            unreachable!()
        };
        let shared = attacker.agree(&ej).unwrap();
        let gid = gi.group_id();
        let transcript = || Transcript {
            mode: PairingMode::Invitation,
            commitment: &c,
            inviter_ephemeral: &attacker_public,
            inviter: &inv.inviter,
            group_id: &gid,
            joiner_ephemeral: &ej,
        };
        let guess = Session::derive(transcript(), &shared, Some(&[7; 32]));
        assert!(open::<JoinBody>(&guess.joiner_to_inviter, &guess.th, &sealed, "x").is_err());

        // Even with a leaked secret it cannot finish: the welcome must be
        // signed by I's identity key, which the attacker does not hold.
        let leaked = Session::derive(transcript(), &shared, Some(inv.secret()));
        assert!(open::<JoinBody>(&leaked.joiner_to_inviter, &leaked.th, &sealed, "x").is_ok());
        let mut forged_group = gi.clone();
        forged_group
            .add_member(
                &i_dev.id,
                Member {
                    identity: j_dev.id.public(),
                    device_id: j_dev.device_id.clone(),
                    name: j_dev.name.clone(),
                    added_at: NOW,
                },
                NOW,
            )
            .unwrap();
        let attacker_id = DeviceIdentity::generate().unwrap();
        let body = WelcomeBody {
            signature: attacker_id.sign(&leaked.signed_context(b"welcome")),
            rosters: forged_group.welcome_chain(),
            key: forged_group.current_key().unwrap().clone(),
        };
        let sealed = seal(&leaked.inviter_to_joiner, &leaked.th, &body).unwrap();
        assert!(matches!(
            j.on_welcome(PairMessage::Welcome { sealed }),
            Err(Error::Auth(_))
        ));
    }

    #[test]
    fn joiner_refuses_an_offer_from_a_device_other_than_the_invitation() {
        let i_dev = dev(1);
        let j_dev = dev(2);
        let m_dev = dev(3);
        let gm = group(&m_dev);
        let inv = Invitation::new(i_dev.id.public(), vec![], NOW);
        let mut window =
            PairingWindow::for_invitation(Invitation::new(m_dev.id.public(), vec![], NOW));
        let mut m = Inviter::new(&m_dev.id, &gm, &mut window, NOW).unwrap();
        let (mut j, commit) =
            Joiner::start(&j_dev.id, &j_dev.device_id, &j_dev.name, Some(inv), NOW).unwrap();
        let offer = m.on_commit(commit).unwrap();
        assert!(matches!(j.on_offer(offer), Err(Error::Auth(_))));
    }

    #[test]
    fn every_started_session_uses_up_an_attempt() {
        // A machine in the middle posing as the joiner sees the inviter's
        // code after the offer and can drop the session if the code does
        // not suit it. Each such session must still count.
        let a = dev(1);
        let ga = group(&a);
        let m = dev(9);
        let mut window = PairingWindow::for_code(NOW);
        for _ in 0..crate::invite::MAX_CODE_ATTEMPTS {
            let (_m, commit) = Joiner::start(&m.id, &m.device_id, &m.name, None, NOW).unwrap();
            let mut i = Inviter::new(&a.id, &ga, &mut window, NOW).unwrap();
            i.on_commit(commit).unwrap();
            // ...and the attacker disconnects.
        }
        assert!(Inviter::new(&a.id, &ga, &mut window, NOW).is_err());
    }

    #[test]
    fn an_invitation_works_once() {
        let a = dev(1);
        let mut ga = group(&a);
        let inv = Invitation::new(a.id.public(), vec![], NOW);
        let mut window = PairingWindow::for_invitation(inv.clone());
        pair(&a, &mut ga, &mut window, &dev(2), Some(inv.clone())).unwrap();
        let err = pair(&a, &mut ga, &mut window, &dev(3), Some(inv)).unwrap_err();
        assert!(matches!(err, Error::Invitation(_)), "{err}");
        assert_eq!(ga.devices().len(), 2);
    }

    #[test]
    fn a_session_cannot_finish_after_the_window_expires() {
        let a = dev(1);
        let b = dev(2);
        let mut ga = group(&a);
        let mut window = PairingWindow::for_code(NOW);
        let mut i = Inviter::new(&a.id, &ga, &mut window, NOW).unwrap();
        let (mut j, commit) = Joiner::start(&b.id, &b.device_id, &b.name, None, NOW).unwrap();
        let offer = i.on_commit(commit).unwrap();
        i.on_reveal(j.on_offer(offer).unwrap()).unwrap();
        i.on_join(j.confirm().unwrap()).unwrap();
        let late = NOW + crate::invite::CODE_WINDOW_SECS;
        assert!(matches!(
            i.approve(&mut ga, late),
            Err(Error::Invitation(_))
        ));
        assert_eq!(ga.devices().len(), 1);
    }

    #[test]
    fn sas_code_formatting() {
        assert_eq!(SasCode(42_917).to_string(), "042 917");
        assert_eq!(SasCode(0).to_string(), "000 000");
        assert_eq!(SasCode(999_999).to_string(), "999 999");
    }

    #[test]
    fn messages_round_trip_as_json() {
        let msg = PairMessage::Offer {
            ephemeral: [3; 32],
            inviter: PublicIdentity::from_bytes([4; 32]),
            group_id: [5; 16],
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"offer\""));
        assert_eq!(serde_json::from_str::<PairMessage>(&json).unwrap(), msg);
    }
}
