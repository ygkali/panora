// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! QUIC on the local network (ADR 0004): the endpoint, which addresses it
//! will talk to, and how each side proves which device it is.
//!
//! TLS here provides confidentiality and integrity only. Each endpoint
//! presents a throwaway self-signed certificate and the client accepts any
//! certificate (it still checks the handshake signature). Who is on the
//! other end is established one step later by [`crate::transport::authenticate`]: both
//! sides sign a value exported from this TLS session (RFC 5705 / 8446
//! exporter) with their Ed25519 device identity. A machine in the middle
//! terminates two TLS sessions with two different exported values, so it
//! can relay neither signature, and forging one needs the identity key.
//! This binds the identity to the channel without parsing X.509 at all
//! (ADR 0004, 2026-09-25 update).

use crate::bytes::{b64, put};
use crate::error::{Error, Result};
use crate::frame;
use crate::identity::{DeviceIdentity, PublicIdentity, SIGNATURE_LEN};
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{Connection, Endpoint, Incoming, RecvStream, SendStream};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

/// ALPN of a sync session between two members.
pub const ALPN_SYNC: &[u8] = b"panora-sync/1";
/// ALPN of a pairing session (`panora-pair/1` over one bi-stream).
pub const ALPN_PAIR: &[u8] = b"panora-pair/1";
/// Exporter label the identity signatures cover.
const EXPORTER_LABEL: &[u8] = b"EXPORTER-panora-sync-auth/1";
/// Server name used in the (unverified) TLS handshake.
const SERVER_NAME: &str = "panora-sync";

/// Whether `ip` is an address `panora-sync` may talk to: loopback,
/// private (RFC 1918, IPv6 unique-local) or link-local. Everything else,
/// incoming or outgoing, is refused, whatever mDNS or the configuration
/// says.
pub fn is_allowed_peer(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_allowed_peer(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            v6.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    }
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn tls_err(e: impl std::fmt::Display) -> Error {
    Error::Transport(format!("TLS setup failed: {e}"))
}

/// Accepts any server certificate: identity is proven by [`crate::transport::authenticate`],
/// not by the certificate. The handshake signature is still verified, so
/// the TLS session itself is sound.
#[derive(Debug)]
struct AnyCertificate(Arc<CryptoProvider>);

impl ServerCertVerifier for AnyCertificate {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// Pre-authentication steps (stream, identity message) may take this long.
const AUTH_TIMEOUT: Duration = Duration::from_secs(10);

fn transport_config() -> Arc<quinn::TransportConfig> {
    let mut config = quinn::TransportConfig::default();
    // Each session, sync or pairing, is one bi-directional stream the
    // dialling side opens. Anything else a peer opens would only be
    // buffered, unread, by this side: allow none of it.
    config.max_concurrent_uni_streams(0u32.into());
    config.max_concurrent_bidi_streams(1u32.into());
    config.stream_receive_window((4u32 * 1024 * 1024).into());
    config.receive_window((8u32 * 1024 * 1024).into());
    config.keep_alive_interval(Some(Duration::from_secs(10)));
    config.max_idle_timeout(Some(
        Duration::from_secs(60)
            .try_into()
            .expect("60 s is a valid idle timeout"),
    ));
    Arc::new(config)
}

fn client_config(alpn: &[u8]) -> Result<quinn::ClientConfig> {
    let provider = provider();
    let mut tls = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(tls_err)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyCertificate(provider)))
        .with_no_client_auth();
    tls.alpn_protocols = vec![alpn.to_vec()];
    let quic = QuicClientConfig::try_from(tls).map_err(tls_err)?;
    let mut config = quinn::ClientConfig::new(Arc::new(quic));
    config.transport_config(transport_config());
    Ok(config)
}

fn server_config() -> Result<quinn::ServerConfig> {
    let certified =
        rcgen::generate_simple_self_signed(vec![SERVER_NAME.to_string()]).map_err(tls_err)?;
    let cert = CertificateDer::from(certified.cert.der().to_vec());
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
        certified.signing_key.serialize_der(),
    ));
    let mut tls = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(tls_err)?
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .map_err(tls_err)?;
    tls.alpn_protocols = vec![ALPN_SYNC.to_vec(), ALPN_PAIR.to_vec()];
    let quic = QuicServerConfig::try_from(tls).map_err(tls_err)?;
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(quic));
    config.transport_config(transport_config());
    Ok(config)
}

/// A QUIC endpoint that both listens and dials.
pub struct Transport {
    endpoint: Endpoint,
    sync_client: quinn::ClientConfig,
    pair_client: quinn::ClientConfig,
}

impl Transport {
    /// Listen on `addr` (port 0 picks one).
    pub fn bind(addr: SocketAddr) -> Result<Self> {
        let endpoint = Endpoint::server(server_config()?, addr)?;
        Ok(Self {
            endpoint,
            sync_client: client_config(ALPN_SYNC)?,
            pair_client: client_config(ALPN_PAIR)?,
        })
    }

    /// The address actually bound.
    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.endpoint.local_addr()?)
    }

    /// Dial `addr` for a sync (`ALPN_SYNC`) or pairing (`ALPN_PAIR`)
    /// session.
    pub async fn connect(&self, addr: SocketAddr, alpn: &[u8]) -> Result<Connection> {
        if !is_allowed_peer(addr.ip()) {
            return Err(Error::Transport(format!(
                "refusing to contact {addr}: only local-network addresses are allowed"
            )));
        }
        let config = if alpn == ALPN_PAIR {
            self.pair_client.clone()
        } else {
            self.sync_client.clone()
        };
        let connecting = self
            .endpoint
            .connect_with(config, addr, SERVER_NAME)
            .map_err(|e| Error::Transport(format!("cannot connect to {addr}: {e}")))?;
        connecting
            .await
            .map_err(|e| Error::Transport(format!("cannot connect to {addr}: {e}")))
    }

    /// The next incoming connection from an allowed address; others are
    /// dropped before any handshake, without an answer that would tell a
    /// scanner a QUIC service is there. `None` once the endpoint is closed.
    pub async fn accept(&self) -> Option<Incoming> {
        loop {
            let incoming = self.endpoint.accept().await?;
            if is_allowed_peer(incoming.remote_address().ip()) {
                return Some(incoming);
            }
            incoming.ignore();
        }
    }

    /// Stop listening and close every connection.
    pub fn close(&self) {
        self.endpoint.close(0u32.into(), b"shutting down");
    }
}

/// The ALPN protocol a connection negotiated.
pub fn alpn(conn: &Connection) -> Option<Vec<u8>> {
    conn.handshake_data()?
        .downcast::<quinn::crypto::rustls::HandshakeData>()
        .ok()?
        .protocol
}

#[derive(Serialize, Deserialize)]
struct AuthMessage {
    identity: PublicIdentity,
    #[serde(with = "b64")]
    signature: [u8; SIGNATURE_LEN],
}

/// What a side signs: the session's exported value and its role, so a
/// signature can neither be moved to another session nor reflected back
/// to the side that made it.
fn auth_payload(conn: &Connection, initiator: bool) -> Result<Vec<u8>> {
    let mut exported = [0u8; 32];
    conn.export_keying_material(&mut exported, EXPORTER_LABEL, b"")
        .map_err(|_| Error::Crypto)?;
    let mut out = b"panora-sync/1 auth".to_vec();
    put(
        &mut out,
        if initiator {
            b"initiator"
        } else {
            b"responder"
        },
    );
    put(&mut out, &exported);
    Ok(out)
}

/// Prove this device's identity to the other side of `conn` and learn
/// theirs, over the connection's first bi-stream. `initiator` is true on
/// the side that dialled.
pub async fn authenticate(
    conn: &Connection,
    send: &mut SendStream,
    recv: &mut RecvStream,
    identity: &DeviceIdentity,
    initiator: bool,
) -> Result<PublicIdentity> {
    let mine = AuthMessage {
        identity: identity.public(),
        signature: identity.sign(&auth_payload(conn, initiator)?),
    };
    frame::write(send, &mine, &[]).await?;
    let (theirs, _): (AuthMessage, _) = frame::read(recv, 4096).await?;
    if theirs.identity == identity.public() {
        return Err(Error::Auth(
            "the other device claims this device's identity",
        ));
    }
    theirs
        .identity
        .verify(&auth_payload(conn, !initiator)?, &theirs.signature)?;
    Ok(theirs.identity)
}

/// Dial side: open the first bi-stream and authenticate.
pub async fn open_authenticated(
    conn: &Connection,
    identity: &DeviceIdentity,
) -> Result<(SendStream, RecvStream, PublicIdentity)> {
    let (mut send, mut recv) = tokio::time::timeout(AUTH_TIMEOUT, conn.open_bi())
        .await
        .map_err(|_| Error::Transport("the other device did not answer".into()))?
        .map_err(|e| Error::Transport(e.to_string()))?;
    let peer = tokio::time::timeout(
        AUTH_TIMEOUT,
        authenticate(conn, &mut send, &mut recv, identity, true),
    )
    .await
    .map_err(|_| Error::Auth("the other device did not identify itself in time"))??;
    Ok((send, recv, peer))
}

/// Listen side: accept the first bi-stream and authenticate.
pub async fn accept_authenticated(
    conn: &Connection,
    identity: &DeviceIdentity,
) -> Result<(SendStream, RecvStream, PublicIdentity)> {
    let (mut send, mut recv) = tokio::time::timeout(AUTH_TIMEOUT, conn.accept_bi())
        .await
        .map_err(|_| Error::Transport("the other device opened no stream".into()))?
        .map_err(|e| Error::Transport(e.to_string()))?;
    let peer = tokio::time::timeout(
        AUTH_TIMEOUT,
        authenticate(conn, &mut send, &mut recv, identity, false),
    )
    .await
    .map_err(|_| Error::Auth("the other device did not identify itself in time"))??;
    Ok((send, recv, peer))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_local_addresses_are_allowed() {
        for ok in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.20",
            "169.254.3.4",
            "::1",
            "fd12:3456::1",
            "fe80::1",
            "::ffff:192.168.1.5",
        ] {
            assert!(is_allowed_peer(ok.parse().unwrap()), "{ok}");
        }
        for bad in [
            "8.8.8.8",
            "172.32.0.1",
            "100.64.0.1",
            "224.0.0.251",
            "0.0.0.0",
            "2001:db8::1",
            "2a00:1450::1",
            "::ffff:8.8.8.8",
            "ff02::fb",
        ] {
            assert!(!is_allowed_peer(bad.parse().unwrap()), "{bad}");
        }
    }

    async fn pair_of_endpoints() -> (Transport, Transport) {
        let a = Transport::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let b = Transport::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        (a, b)
    }

    #[tokio::test]
    async fn both_sides_learn_each_others_identity() {
        let (a, b) = pair_of_endpoints().await;
        let (ida, idb) = (
            DeviceIdentity::generate().unwrap(),
            DeviceIdentity::generate().unwrap(),
        );
        let b_addr = b.local_addr().unwrap();
        let dial = async {
            let conn = a.connect(b_addr, ALPN_SYNC).await.unwrap();
            assert_eq!(alpn(&conn).as_deref(), Some(ALPN_SYNC));
            let (_s, _r, peer) = open_authenticated(&conn, &ida).await.unwrap();
            (conn, peer)
        };
        let listen = async {
            let conn = b.accept().await.unwrap().await.unwrap();
            let (_s, _r, peer) = accept_authenticated(&conn, &idb).await.unwrap();
            (conn, peer)
        };
        let ((_ca, seen_by_a), (_cb, seen_by_b)) = tokio::join!(dial, listen);
        assert_eq!(seen_by_a, idb.public());
        assert_eq!(seen_by_b, ida.public());
    }

    #[tokio::test]
    async fn a_signature_from_another_session_or_role_is_refused() {
        let (a, b) = pair_of_endpoints().await;
        let (ida, idb) = (
            DeviceIdentity::generate().unwrap(),
            DeviceIdentity::generate().unwrap(),
        );
        let b_addr = b.local_addr().unwrap();
        // A signs as responder (the wrong role) over the right session: a
        // reflected signature must not verify.
        let dial = async {
            let conn = a.connect(b_addr, ALPN_SYNC).await.unwrap();
            let (mut send, mut recv) = conn.open_bi().await.unwrap();
            let forged = AuthMessage {
                identity: ida.public(),
                signature: ida.sign(&auth_payload(&conn, false).unwrap()),
            };
            frame::write(&mut send, &forged, &[]).await.unwrap();
            let _ = frame::read::<_, AuthMessage>(&mut recv, 4096).await;
            conn
        };
        let listen = async {
            let conn = b.accept().await.unwrap().await.unwrap();
            accept_authenticated(&conn, &idb).await
        };
        let (_conn, result) = tokio::join!(dial, listen);
        assert!(matches!(result, Err(Error::Auth(_))));
    }

    #[tokio::test]
    async fn public_addresses_are_not_dialled() {
        let (a, _b) = pair_of_endpoints().await;
        let err = a
            .connect("8.8.8.8:47100".parse().unwrap(), ALPN_SYNC)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Transport(_)));
    }
}
