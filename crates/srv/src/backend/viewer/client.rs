// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The paired-device side of the viewer: an HTTPS client that accepts exactly
//! one certificate, by the fingerprint the pairing link carried
//! (`cert::fingerprint`), the way a paired phone does. Used by Tower to read
//! another AgentMux machine's processes (`backend::tower_peers`).
//!
//! Nothing about the certificate is trusted but its fingerprint: no chain, no
//! name, no expiry. The TLS handshake signatures are still verified, so the
//! peer must hold the certificate's private key.

use std::sync::Arc;
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};

/// Accepts the one certificate whose fingerprint is `.0`.
#[derive(Debug)]
pub struct Pinned(pub String);

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let got = super::cert::fingerprint(end_entity);
        if agentmux_common::secret_eq::secret_eq(got.as_bytes(), self.0.to_ascii_lowercase().as_bytes()) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General("certificate fingerprint mismatch".into()))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        let algs = rustls::crypto::ring::default_provider().signature_verification_algorithms;
        rustls::crypto::verify_tls12_signature(message, cert, dss, &algs)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        let algs = rustls::crypto::ring::default_provider().signature_verification_algorithms;
        rustls::crypto::verify_tls13_signature(message, cert, dss, &algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider().signature_verification_algorithms.supported_schemes()
    }
}

/// An HTTPS client that talks only to the server holding the certificate
/// with `fingerprint`.
pub fn pinned_client(fingerprint: &str, timeout: Duration) -> Result<reqwest::Client, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Pinned(fingerprint.to_string())))
        .with_no_client_auth();
    reqwest::Client::builder()
        .use_preconfigured_tls(config)
        // Only ever a LAN address: straight there, never through a configured
        // proxy (which usually can't reach it).
        .no_proxy()
        .timeout(timeout)
        .build()
        .map_err(|e| e.to_string())
}
