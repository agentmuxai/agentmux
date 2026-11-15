// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The viewer listener's certificate: self-signed ECDSA P-256, made once and
//! kept in this instance's data directory, so its fingerprint (the one a
//! paired device pinned from the QR) survives restarts. No CA is involved and
//! nothing about the certificate itself is trusted; only the fingerprint is.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest, Sha256};

/// Where the certificate and its key live under the data directory.
const DIR_NAME: &str = "viewer";
const CERT_FILE: &str = "cert.der";
const KEY_FILE: &str = "key.der";

/// The certificate in use, its key and its fingerprint.
pub struct ViewerTls {
    pub cert_der: Vec<u8>,
    key_der: Vec<u8>,
    /// Lowercase hex SHA-256 of `cert_der`, as the QR carries it.
    pub fingerprint: String,
}

impl ViewerTls {
    /// A rustls server configuration for this certificate, HTTP/1.1 only.
    pub fn server_config(&self) -> Result<Arc<rustls::ServerConfig>, String> {
        use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(self.cert_der.clone())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key_der.clone())),
            )
            .map_err(|e| e.to_string())?;
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(Arc::new(config))
    }
}

/// Lowercase hex SHA-256 of a certificate's DER bytes.
pub fn fingerprint(cert_der: &[u8]) -> String {
    hex::encode(Sha256::digest(cert_der))
}

/// The certificate directory under `data_dir`.
pub fn cert_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(DIR_NAME)
}

/// Load the certificate kept under `data_dir`, or make and keep a new one when
/// there is none or what is there can't be used.
pub fn load_or_create(data_dir: &Path) -> Result<ViewerTls, String> {
    let dir = cert_dir(data_dir);
    match load(&dir) {
        Ok(Some(tls)) => return Ok(tls),
        Ok(None) => {}
        Err(e) => tracing::warn!(error = %e, "viewer certificate unreadable; making a new one (paired devices must pair again)"),
    }
    let tls = generate()?;
    store(&dir, &tls).map_err(|e| format!("could not keep the viewer certificate: {e}"))?;
    tracing::info!(fingerprint = %tls.fingerprint, "viewer certificate created");
    Ok(tls)
}

fn load(dir: &Path) -> Result<Option<ViewerTls>, String> {
    let (cert, key) = (dir.join(CERT_FILE), dir.join(KEY_FILE));
    if !cert.exists() && !key.exists() {
        return Ok(None);
    }
    let cert_der = std::fs::read(&cert).map_err(|e| e.to_string())?;
    let key_der = std::fs::read(&key).map_err(|e| e.to_string())?;
    let tls = ViewerTls { fingerprint: fingerprint(&cert_der), cert_der, key_der };
    // A pair that doesn't make a working configuration (truncated file, a key
    // from another certificate) is as good as none.
    tls.server_config()?;
    Ok(Some(tls))
}

fn generate() -> Result<ViewerTls, String> {
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).map_err(|e| e.to_string())?;
    let mut params = rcgen::CertificateParams::new(vec!["agentmux-viewer".to_string()]).map_err(|e| e.to_string())?;
    params.distinguished_name = rcgen::DistinguishedName::new();
    params.distinguished_name.push(rcgen::DnType::CommonName, "AgentMux viewer");
    let cert = params.self_signed(&key).map_err(|e| e.to_string())?;
    let cert_der = cert.der().to_vec();
    Ok(ViewerTls { fingerprint: fingerprint(&cert_der), cert_der, key_der: key.serialize_der() })
}

/// Write both files through a temporary name, the key readable by this user
/// only where the platform has modes.
fn store(dir: &Path, tls: &ViewerTls) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    write_atomic(&dir.join(KEY_FILE), &tls.key_der, true)?;
    write_atomic(&dir.join(CERT_FILE), &tls.cert_der, false)
}

fn write_atomic(path: &Path, bytes: &[u8], private: bool) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        #[cfg(not(unix))]
        let _ = private;
        use std::io::Write;
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fingerprint_is_lowercase_hex_sha256_of_the_der() {
        let fp = fingerprint(b"abc");
        assert_eq!(fp, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(fp.len(), 64);
        assert!(fp.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)));
    }

    #[test]
    fn a_certificate_is_made_once_and_reused() {
        let dir = tempfile::tempdir().unwrap();
        let first = load_or_create(dir.path()).unwrap();
        assert_eq!(first.fingerprint, fingerprint(&first.cert_der));
        assert!(cert_dir(dir.path()).join(CERT_FILE).exists());
        assert!(cert_dir(dir.path()).join(KEY_FILE).exists());
        first.server_config().expect("a usable rustls configuration");

        let second = load_or_create(dir.path()).unwrap();
        assert_eq!(second.fingerprint, first.fingerprint, "a restart keeps the pinned certificate");
        assert_eq!(second.cert_der, first.cert_der);
    }

    #[test]
    fn a_damaged_certificate_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let first = load_or_create(dir.path()).unwrap();
        std::fs::write(cert_dir(dir.path()).join(KEY_FILE), b"not a key").unwrap();
        let second = load_or_create(dir.path()).unwrap();
        assert_ne!(second.fingerprint, first.fingerprint);
        second.server_config().unwrap();
    }

    #[test]
    fn the_certificate_is_ecdsa_p256() {
        let tls = generate().unwrap();
        // id-ecPublicKey (1.2.840.10045.2.1) and prime256v1 (1.2.840.10045.3.1.7).
        let ec_public_key = [0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
        let p256 = [0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];
        let has = |needle: &[u8]| tls.cert_der.windows(needle.len()).any(|w| w == needle);
        assert!(has(&ec_public_key) && has(&p256));
    }
}
