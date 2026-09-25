// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Finding the other devices with mDNS (DNS-SD).
//!
//! - `_panora-sync._udp`: members of a group announce a tag derived from
//!   the group id and the current hour, nothing else. Only members can
//!   compute the tag, and it changes every hour, so a stranger on the
//!   network learns neither which group a device is in nor, for long, that
//!   two sightings are the same group. The instance name is random per run.
//! - `_panora-pair._udp`: only while a pairing window is open, with the
//!   device name (so the joining user sees whom they are joining) and a
//!   short tag of the inviter's identity key (so a device holding an
//!   invitation finds the right inviter).
//!
//! Every address learned here still goes through
//! [`crate::transport::is_allowed_peer`] before anything is sent to it.

use crate::error::{Error, Result};
use crate::group::GroupId;
use crate::identity::PublicIdentity;
use crate::transport::is_allowed_peer;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use rand::RngCore;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::{debug, warn};

const SYNC_TYPE: &str = "_panora-sync._udp.local.";
const PAIR_TYPE: &str = "_panora-pair._udp.local.";
/// Longest device name put in a TXT record.
const MAX_TXT_NAME: usize = 63;
/// Most announced services remembered; spoofed names cannot grow it more.
const MAX_SEEN: usize = 256;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hour() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 3600)
        .unwrap_or(0)
}

/// The announced tag of `group` during `hour`.
fn group_tag(group: &GroupId, hour: u64) -> String {
    let mut input = group.to_vec();
    input.extend_from_slice(&hour.to_be_bytes());
    hex(&blake3::derive_key("panora mdns group tag v1", &input)[..8])
}

/// A short, public tag of an identity key, announced by an inviter.
pub fn identity_tag(identity: &PublicIdentity) -> String {
    hex(&blake3::derive_key("panora mdns identity tag v1", identity.as_bytes())[..6])
}

/// An address another device can dial as it is: allowed, and not IPv6
/// link-local, which needs an interface index to be reachable.
fn dialable(ip: &IpAddr) -> bool {
    let v6_link_local = matches!(ip, IpAddr::V6(v6) if (v6.segments()[0] & 0xffc0) == 0xfe80);
    is_allowed_peer(*ip) && !v6_link_local
}

/// This machine's local-network addresses, with `port`, for an
/// invitation (at most four).
pub fn local_addresses(port: u16) -> Vec<SocketAddr> {
    let Ok(interfaces) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    let mut addrs: Vec<SocketAddr> = interfaces
        .into_iter()
        .map(|i| i.ip())
        .filter(|ip| !ip.is_loopback() && dialable(ip))
        .map(|ip| SocketAddr::new(ip, port))
        .collect();
    // IPv4 first: what most home networks route between devices.
    addrs.sort_by_key(|a| !a.is_ipv4());
    addrs.dedup();
    addrs.truncate(4);
    addrs
}

/// A device waiting for a pairing.
#[derive(Debug, Clone)]
pub struct Found {
    /// [`identity_tag`] of the inviter.
    pub tag: String,
    /// Its name.
    pub name: String,
    /// Where it listens.
    pub addr: SocketAddr,
}

#[derive(Default)]
struct Seen {
    tags: HashMap<String, (String, Vec<SocketAddr>)>,
}

/// The mDNS responder and browser of one node.
pub struct Discovery {
    daemon: ServiceDaemon,
    port: u16,
    instance: String,
    sync_name: Mutex<Option<(String, GroupId, u64)>>,
    pair_name: Mutex<Option<String>>,
    seen: Arc<Mutex<Seen>>,
}

fn mdns_err(e: impl std::fmt::Display) -> Error {
    Error::Transport(format!("mDNS: {e}"))
}

fn resolved_addrs(addresses: impl Iterator<Item = IpAddr>, port: u16) -> Vec<SocketAddr> {
    let mut addrs: Vec<SocketAddr> = addresses
        .filter(dialable)
        .map(|ip| SocketAddr::new(ip, port))
        .collect();
    addrs.sort_by_key(|a| !a.is_ipv4());
    addrs
}

impl Discovery {
    /// Start the responder and browse for group members.
    pub fn start(port: u16) -> Result<Self> {
        let daemon = ServiceDaemon::new().map_err(mdns_err)?;
        let events = daemon.browse(SYNC_TYPE).map_err(mdns_err)?;
        let seen = Arc::new(Mutex::new(Seen::default()));
        let sink = seen.clone();
        std::thread::Builder::new()
            .name("mdns-browse".into())
            .spawn(move || {
                while let Ok(event) = events.recv() {
                    let mut seen = sink.lock().unwrap_or_else(|e| e.into_inner());
                    match event {
                        ServiceEvent::ServiceResolved(info) => {
                            let Some(tag) = info.get_property_val_str("g") else {
                                continue;
                            };
                            let addrs = resolved_addrs(
                                info.addresses.iter().map(|a| a.to_ip_addr()),
                                info.port,
                            );
                            if seen.tags.len() >= MAX_SEEN
                                && !seen.tags.contains_key(&info.fullname)
                            {
                                continue;
                            }
                            seen.tags
                                .insert(info.fullname.clone(), (tag.to_string(), addrs));
                        }
                        ServiceEvent::ServiceRemoved(_, fullname) => {
                            seen.tags.remove(&fullname);
                        }
                        _ => {}
                    }
                }
            })
            .map_err(mdns_err)?;
        let mut instance = [0u8; 6];
        rand::rngs::OsRng.fill_bytes(&mut instance);
        Ok(Self {
            daemon,
            port,
            instance: hex(&instance),
            sync_name: Mutex::new(None),
            pair_name: Mutex::new(None),
            seen,
        })
    }

    fn register(&self, ty: &str, instance: &str, props: &[(&str, &str)]) -> Option<String> {
        let host = format!("{}.local.", self.instance);
        match ServiceInfo::new(ty, instance, &host, "", self.port, props) {
            Ok(info) => {
                let info = info.enable_addr_auto();
                let name = info.get_fullname().to_string();
                match self.daemon.register(info) {
                    Ok(()) => Some(name),
                    Err(e) => {
                        warn!(error = %e, "mDNS registration failed");
                        None
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "mDNS service record rejected");
                None
            }
        }
    }

    fn unregister(&self, name: Option<String>) {
        if let Some(name) = name {
            let _ = self.daemon.unregister(&name);
        }
    }

    /// Announce membership of `group` (or stop, with `None`).
    pub fn advertise_sync(&self, group: Option<&GroupId>) {
        let mut current = self.sync_name.lock().unwrap_or_else(|e| e.into_inner());
        self.unregister(current.take().map(|(name, _, _)| name));
        if let Some(group) = group {
            let hour = hour();
            let tag = group_tag(group, hour);
            if let Some(name) = self.register(SYNC_TYPE, &self.instance, &[("v", "1"), ("g", &tag)])
            {
                *current = Some((name, *group, hour));
            }
        }
    }

    /// Re-announce with the new hour's tag when the hour changed.
    pub fn refresh(&self) {
        let group = {
            let current = self.sync_name.lock().unwrap_or_else(|e| e.into_inner());
            match &*current {
                Some((_, group, h)) if *h != hour() => Some(*group),
                _ => None,
            }
        };
        if let Some(group) = group {
            self.advertise_sync(Some(&group));
        }
    }

    /// Addresses of devices announcing `group` this hour or the last.
    pub fn sync_peers(&self, group: &GroupId) -> Vec<SocketAddr> {
        let now = hour();
        let tags = [
            group_tag(group, now),
            group_tag(group, now.saturating_sub(1)),
        ];
        let seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        seen.tags
            .iter()
            .filter(|(name, (tag, _))| tags.contains(tag) && !name.starts_with(&self.instance))
            .filter_map(|(_, (_, addrs))| addrs.first().copied())
            .collect()
    }

    /// Announce an open pairing window.
    pub fn advertise_pair(&self, identity: &PublicIdentity, name: &str) {
        let mut current = self.pair_name.lock().unwrap_or_else(|e| e.into_inner());
        self.unregister(current.take());
        let mut shown: String = name.to_string();
        while shown.len() > MAX_TXT_NAME {
            shown.pop();
        }
        let tag = identity_tag(identity);
        *current = self.register(
            PAIR_TYPE,
            &self.instance,
            &[("v", "1"), ("i", &tag), ("n", &shown)],
        );
    }

    /// Stop announcing the pairing window.
    pub fn stop_pair(&self) {
        let mut current = self.pair_name.lock().unwrap_or_else(|e| e.into_inner());
        self.unregister(current.take());
    }

    /// Devices with an open pairing window, gathered for `wait`.
    pub async fn find_pair(&self, wait: Duration) -> Vec<Found> {
        let daemon = self.daemon.clone();
        let own = self.instance.clone();
        tokio::task::spawn_blocking(move || {
            let Ok(events) = daemon.browse(PAIR_TYPE) else {
                return Vec::new();
            };
            let deadline = std::time::Instant::now() + wait;
            let mut found: HashMap<String, Found> = HashMap::new();
            while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
                let Ok(event) = events.recv_timeout(left) else {
                    break;
                };
                if let ServiceEvent::ServiceResolved(info) = event {
                    if info.fullname.starts_with(&own) {
                        continue;
                    }
                    let addr =
                        resolved_addrs(info.addresses.iter().map(|a| a.to_ip_addr()), info.port)
                            .into_iter()
                            .next();
                    let (Some(addr), Some(tag)) = (addr, info.get_property_val_str("i")) else {
                        continue;
                    };
                    found.insert(
                        info.fullname.clone(),
                        Found {
                            tag: tag.to_string(),
                            name: info.get_property_val_str("n").unwrap_or("").to_string(),
                            addr,
                        },
                    );
                }
            }
            let _ = daemon.stop_browse(PAIR_TYPE);
            debug!(count = found.len(), "pairing windows found");
            found.into_values().collect()
        })
        .await
        .unwrap_or_default()
    }

    /// Withdraw everything and stop the responder.
    pub fn shutdown(&self) {
        self.advertise_sync(None);
        self.stop_pair();
        let _ = self.daemon.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_depend_on_group_and_hour_only() {
        let a = [1u8; 16];
        let b = [2u8; 16];
        assert_eq!(group_tag(&a, 10), group_tag(&a, 10));
        assert_ne!(group_tag(&a, 10), group_tag(&a, 11));
        assert_ne!(group_tag(&a, 10), group_tag(&b, 10));
        assert_eq!(group_tag(&a, 10).len(), 16);
        let id = PublicIdentity::from_bytes([7; 32]);
        assert_eq!(identity_tag(&id).len(), 12);
    }

    #[test]
    fn only_local_addresses_are_kept() {
        let addrs = resolved_addrs(
            [
                "8.8.8.8".parse().unwrap(),
                "fe80::1".parse().unwrap(),
                "192.168.1.9".parse().unwrap(),
            ]
            .into_iter(),
            47100,
        );
        // Link-local IPv6 needs an interface index, which mDNS answers
        // and invitations do not carry: left out.
        assert_eq!(addrs, vec!["192.168.1.9:47100".parse().unwrap()]);
        let ula = resolved_addrs(["fd00::5".parse().unwrap()].into_iter(), 1);
        assert_eq!(ula, vec!["[fd00::5]:1".parse().unwrap()]);
        for addr in local_addresses(1) {
            assert!(dialable(&addr.ip()) && !addr.ip().is_loopback());
        }
    }
}
