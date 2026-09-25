// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Invitations: what the inviting device shows as a QR code or a link, and
//! the window it keeps open while waiting for the other device.
//!
//! An invitation carries the inviter's identity key (so the joiner knows
//! exactly which device to trust), a 256-bit one-time secret (so the
//! inviter knows the joiner saw the invitation), an expiry time, and
//! optionally the addresses the inviter listens on:
//!
//! ```text
//! panora-pair:1?id=<base64url key>&s=<base64url secret>&exp=<unix time>&addr=192.168.1.20:47100
//! ```
//!
//! Whoever reads the invitation can join the group, so it is shown only on
//! the inviter's own screen, lives ten minutes, and works once.

use crate::error::{Error, Result};
use crate::identity::{PublicIdentity, PUBLIC_KEY_LEN};
use crate::pairing::PairingMode;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use rand::RngCore;
use std::net::SocketAddr;
use zeroize::Zeroizing;

/// How long an invitation stays valid.
pub const INVITATION_TTL_SECS: i64 = 600;
/// How long the inviter accepts code-compared pairing once the user opens it.
pub const CODE_WINDOW_SECS: i64 = 300;
/// Pairing sessions a code window allows. Every session that starts
/// counts, finished or not: a machine in the middle posing as the joiner
/// learns the inviter's code as soon as it has the offer, and could
/// otherwise drop the session and retry until the code matches.
pub const MAX_CODE_ATTEMPTS: u32 = 3;
/// Pairing sessions an invitation allows. Its 256-bit secret already stops
/// guessing; the limit only bounds the work a stranger can cause.
pub const MAX_INVITATION_ATTEMPTS: u32 = 10;
/// Scheme and version prefix of an invitation link.
pub const URI_PREFIX: &str = "panora-pair:1?";
/// Longest invitation link accepted.
const MAX_URI_LEN: usize = 1024;
/// Most addresses an invitation may list.
const MAX_ADDRS: usize = 8;

/// A one-time invitation to join the inviter's group.
#[derive(Clone)]
pub struct Invitation {
    /// The inviting device's identity key.
    pub inviter: PublicIdentity,
    secret: Zeroizing<[u8; 32]>,
    /// Unix time after which it no longer works.
    pub expires_at: i64,
    /// Addresses the inviter listens on, if known; otherwise mDNS finds it
    /// by its identity key (SYNC-04).
    pub addrs: Vec<SocketAddr>,
}

impl Invitation {
    /// A fresh invitation from `inviter`, valid for
    /// [`INVITATION_TTL_SECS`].
    pub fn new(inviter: PublicIdentity, addrs: Vec<SocketAddr>, now: i64) -> Self {
        let mut secret = Zeroizing::new([0u8; 32]);
        rand::rngs::OsRng.fill_bytes(secret.as_mut());
        Self {
            inviter,
            secret,
            expires_at: now + INVITATION_TTL_SECS,
            addrs: addrs.into_iter().take(MAX_ADDRS).collect(),
        }
    }

    /// The one-time secret.
    pub(crate) fn secret(&self) -> &[u8; 32] {
        &self.secret
    }

    /// Whether it has expired at `now`.
    pub fn is_expired(&self, now: i64) -> bool {
        now >= self.expires_at
    }

    /// The link / QR payload.
    pub fn to_uri(&self) -> String {
        let mut uri = format!(
            "{URI_PREFIX}id={}&s={}&exp={}",
            URL_SAFE_NO_PAD.encode(self.inviter.as_bytes()),
            URL_SAFE_NO_PAD.encode(self.secret.as_ref()),
            self.expires_at
        );
        for addr in &self.addrs {
            uri.push_str("&addr=");
            // A socket address never contains '&' or '#'; the only
            // character that needs escaping is '%' in an IPv6 zone id.
            uri.push_str(&addr.to_string().replace('%', "%25"));
        }
        uri
    }

    /// Parse a link. Strict: every field once, known fields only checked
    /// for shape, unknown fields ignored so a later version can add some.
    pub fn parse(uri: &str) -> Result<Self> {
        let uri = uri.trim();
        if uri.len() > MAX_URI_LEN {
            return Err(Error::Invitation("link is too long"));
        }
        let query = uri.strip_prefix(URI_PREFIX).ok_or(Error::Invitation(
            "not a Panora pairing link of a known version",
        ))?;
        let mut id = None;
        let mut secret = None;
        let mut exp = None;
        let mut addrs = Vec::new();
        for pair in query.split('&') {
            let (key, value) = pair
                .split_once('=')
                .ok_or(Error::Invitation("malformed field"))?;
            match key {
                "id" => set_once(&mut id, decode_32(value)?)?,
                "s" => set_once(&mut secret, Zeroizing::new(decode_32(value)?))?,
                "exp" => set_once(
                    &mut exp,
                    value
                        .parse::<i64>()
                        .map_err(|_| Error::Invitation("expiry is not a number"))?,
                )?,
                "addr" => {
                    if addrs.len() == MAX_ADDRS {
                        return Err(Error::Invitation("too many addresses"));
                    }
                    let text = value.replace("%25", "%");
                    if text.contains('%') && !text.starts_with('[') {
                        return Err(Error::Invitation("malformed address"));
                    }
                    addrs.push(
                        text.parse::<SocketAddr>()
                            .map_err(|_| Error::Invitation("malformed address"))?,
                    );
                }
                _ => {}
            }
        }
        Ok(Self {
            inviter: PublicIdentity::from_bytes(
                id.ok_or(Error::Invitation("inviter key missing"))?,
            ),
            secret: secret.ok_or(Error::Invitation("secret missing"))?,
            expires_at: exp.ok_or(Error::Invitation("expiry missing"))?,
            addrs,
        })
    }
}

impl std::fmt::Debug for Invitation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Invitation")
            .field("inviter", &self.inviter)
            .field("secret", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .field("addrs", &self.addrs)
            .finish()
    }
}

fn set_once<T>(slot: &mut Option<T>, value: T) -> Result<()> {
    if slot.replace(value).is_some() {
        return Err(Error::Invitation("a field appears twice"));
    }
    Ok(())
}

fn decode_32(value: &str) -> Result<[u8; PUBLIC_KEY_LEN]> {
    let bytes = Zeroizing::new(
        URL_SAFE_NO_PAD
            .decode(value)
            .map_err(|_| Error::Invitation("malformed key field"))?,
    );
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| Error::Invitation("key field has the wrong length"))
}

/// The inviter's side of "I am accepting a new device now": which mode the
/// user picked, until when, and how many sessions it still allows. Each
/// [`crate::Inviter`] borrows it for the whole session, so only one runs at
/// a time; starting one uses up an attempt, and a successful pairing
/// closes the window (an invitation works once, a code window pairs one
/// device).
#[derive(Debug)]
pub struct PairingWindow {
    invitation: Option<Invitation>,
    expires_at: i64,
    attempts: u32,
    closed: bool,
}

impl PairingWindow {
    /// Accept exactly the device that holds `invitation`.
    pub fn for_invitation(invitation: Invitation) -> Self {
        Self {
            expires_at: invitation.expires_at,
            invitation: Some(invitation),
            attempts: 0,
            closed: false,
        }
    }

    /// Accept a device whose user compares a six-digit code, for
    /// [`CODE_WINDOW_SECS`].
    pub fn for_code(now: i64) -> Self {
        Self {
            invitation: None,
            expires_at: now + CODE_WINDOW_SECS,
            attempts: 0,
            closed: false,
        }
    }

    /// The mode this window pairs in.
    pub fn mode(&self) -> PairingMode {
        if self.invitation.is_some() {
            PairingMode::Invitation
        } else {
            PairingMode::Code
        }
    }

    /// The invitation, in invitation mode.
    pub fn invitation(&self) -> Option<&Invitation> {
        self.invitation.as_ref()
    }

    fn max_attempts(&self) -> u32 {
        match self.mode() {
            PairingMode::Invitation => MAX_INVITATION_ATTEMPTS,
            PairingMode::Code => MAX_CODE_ATTEMPTS,
        }
    }

    /// Sessions that may still start.
    pub fn attempts_left(&self) -> u32 {
        self.max_attempts().saturating_sub(self.attempts)
    }

    /// Whether another session may start at `now`.
    pub fn is_open(&self, now: i64) -> bool {
        self.is_live(now) && self.attempts < self.max_attempts()
    }

    /// Not closed and not past the deadline; a running session may finish.
    pub(crate) fn is_live(&self, now: i64) -> bool {
        !self.closed && now < self.expires_at
    }

    /// Use up one attempt for a session starting at `now`.
    pub(crate) fn begin(&mut self, now: i64) -> Result<()> {
        if !self.is_open(now) {
            return Err(Error::Invitation("pairing is not open on this device"));
        }
        self.attempts += 1;
        Ok(())
    }

    /// Stop accepting devices (after a pairing, or when the user cancels).
    pub fn close(&mut self) {
        self.closed = true;
        self.invitation = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::DeviceIdentity;

    fn invitation(addrs: Vec<SocketAddr>) -> Invitation {
        Invitation::new(DeviceIdentity::generate().unwrap().public(), addrs, 1000)
    }

    #[test]
    fn uri_round_trip() {
        let inv = invitation(vec![
            "192.168.1.20:47100".parse().unwrap(),
            "[fe80::1%2]:47100".parse().unwrap(),
        ]);
        let uri = inv.to_uri();
        assert!(uri.starts_with(URI_PREFIX));
        let back = Invitation::parse(&uri).unwrap();
        assert_eq!(back.inviter, inv.inviter);
        assert_eq!(back.secret(), inv.secret());
        assert_eq!(back.expires_at, 1000 + INVITATION_TTL_SECS);
        assert_eq!(back.addrs, inv.addrs);
        assert!(!back.is_expired(1000));
        assert!(back.is_expired(1000 + INVITATION_TTL_SECS));
    }

    #[test]
    fn malformed_links_are_refused() {
        let good = invitation(vec![]).to_uri();
        assert!(Invitation::parse(&good).is_ok());
        assert!(Invitation::parse(&good.replace("panora-pair:1", "panora-pair:2")).is_err());
        assert!(Invitation::parse(&format!("{good}&exp=5")).is_err());
        assert!(Invitation::parse(&format!("{good}&addr=nonsense")).is_err());
        assert!(Invitation::parse(&format!("{good}&addr=1.2.3.4:5%41")).is_err());
        assert!(Invitation::parse(&format!("{good}&x")).is_err());
        assert!(Invitation::parse(&format!("{good}{}", "&future=1".repeat(200))).is_err());
        // Unknown fields are ignored.
        assert!(Invitation::parse(&format!("{good}&future=1")).is_ok());
        let short = good.replacen("s=", "s=AA", 1);
        assert!(Invitation::parse(&short).is_err());
        let no_secret: String = good
            .split('&')
            .filter(|f| !f.starts_with("s="))
            .collect::<Vec<_>>()
            .join("&");
        assert!(Invitation::parse(&no_secret).is_err());
    }

    #[test]
    fn debug_hides_the_secret() {
        let inv = invitation(vec![]);
        let shown = format!("{inv:?}");
        assert!(shown.contains("REDACTED"));
        assert!(!shown.contains(&URL_SAFE_NO_PAD.encode(inv.secret().as_ref())));
    }

    #[test]
    fn window_counts_attempts_and_closes() {
        let mut w = PairingWindow::for_code(100);
        assert_eq!(w.mode(), PairingMode::Code);
        assert!(w.is_open(100));
        assert!(!w.is_open(100 + CODE_WINDOW_SECS));
        for _ in 0..MAX_CODE_ATTEMPTS {
            w.begin(101).unwrap();
        }
        assert_eq!(w.attempts_left(), 0);
        assert!(!w.is_open(101));
        assert!(w.begin(101).is_err());
        // A session already running may still finish.
        assert!(w.is_live(101));

        let mut w = PairingWindow::for_invitation(invitation(vec![]));
        assert_eq!(w.mode(), PairingMode::Invitation);
        assert_eq!(w.attempts_left(), MAX_INVITATION_ATTEMPTS);
        assert!(w.is_open(1000));
        w.close();
        assert!(!w.is_open(1000));
        assert!(!w.is_live(1000));
        assert!(w.invitation().is_none());
    }
}
