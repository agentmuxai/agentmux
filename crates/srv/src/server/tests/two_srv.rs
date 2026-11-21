// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Two srv instances in one test process, talking over a real socket: a host with a real viewer listener (HTTPS, a
//! self-signed certificate, a per-device token, signed sends), and a viewer with a client that pins the host's
//! certificate by fingerprint, the way a paired phone or computer does.
//!
//! ```text
//! let host = start_host();                       // listener on 127.0.0.1, a port the OS chose
//! let viewer = start_viewer(&host);              // its own srv state; the client pins the host's fingerprint
//! let device = viewer.pair(&host, "laptop").await; // one-time code, then the token and a signing key
//! let mut feed = viewer.open_feed(&device, &agent.name).await;
//! ```
//!
//! Nothing here leaves the loopback interface, and no listener is left running: dropping the [`Host`] stops it. An
//! agent is seeded on the host with the viewer tests' own helpers (`http_viewer::tests::register_defined`, `append`).

use std::net::SocketAddr;
use std::time::Duration;

use base64::Engine as _;
use ring::rand::SystemRandom;
use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_ASN1_SIGNING};
use tokio_util::sync::CancellationToken;

use crate::backend::viewer::{client, signing};
use crate::server::AppState;

/// How long a read from the host may take before a test gives up on it.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// The host: an srv state with its viewer listener bound on loopback.
pub(crate) struct Host {
    pub state: AppState,
    pub addr: SocketAddr,
    /// The certificate fingerprint a viewer pins, as a pairing link carries it.
    pub fingerprint: String,
    stop: CancellationToken,
    _cert_dir: tempfile::TempDir,
}

impl Drop for Host {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

/// An agent seeded on the host.
pub(crate) struct Seeded {
    pub name: String,
    pub block_id: String,
}

impl Host {
    /// A one-time pairing code, as the host's panel shows it.
    pub(crate) fn new_pairing_code(&self) -> String {
        self.state.viewer.pairing.start().0
    }

    /// An agent on the host, registered the way the viewer tests register one.
    pub(crate) fn seed_agent(&self, prefix: &str) -> Seeded {
        let (name, block_id, _def) = crate::server::http_viewer::tests::register_defined(&self.state, prefix);
        Seeded { name, block_id }
    }

    /// Appends a line to the agent's transcript on the host.
    pub(crate) fn append(&self, agent: &Seeded, line: &str) {
        crate::server::http_viewer::tests::append(&self.state, &agent.block_id, line);
    }

    /// The host's owner lets the device send (the grant `Settings > Paired devices` gives).
    pub(crate) fn allow_send(&self, device: &Paired) {
        self.state.mstore.viewer_device_set_can_send(&device.device_id, true).unwrap();
    }

    /// The host's owner revokes the device: its next request is refused and its feeds end.
    pub(crate) fn revoke(&self, device: &Paired) {
        let result = crate::server::app_api::viewer::revoke_impl(&self.state, &device.device_id).unwrap();
        assert!(result.revoked, "the device was paired");
    }
}

/// Starts the host. Must run inside a tokio runtime (the listener is a task on it).
pub(crate) fn start_host() -> Host {
    let cert_dir = tempfile::tempdir().unwrap();
    let mut state = crate::server::tests::test_state();
    // The feed hub listens to this state's own broker, so what the host appends reaches a feed.
    state.viewer = crate::backend::viewer::ViewerService::new(Some(cert_dir.path().to_path_buf()), &state.broker);
    let tls = state.viewer.tls().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let stop = CancellationToken::new();
    crate::backend::lan::listeners::LanListenerSupervisor::serve_tls(
        crate::server::build_viewer_router(state.clone()),
        listener,
        tokio_rustls::TlsAcceptor::from(tls.server_config().unwrap()),
        stop.clone(),
    );
    Host { state, addr, fingerprint: tls.fingerprint.clone(), stop, _cert_dir: cert_dir }
}

/// The viewer: its own srv state, and an HTTPS client that accepts only the certificate it was pinned to.
pub(crate) struct Viewer {
    pub state: AppState,
    client: reqwest::Client,
    base: String,
}

/// Starts a viewer pinned to `host`'s certificate.
pub(crate) fn start_viewer(host: &Host) -> Viewer {
    start_viewer_pinned_to(host, &host.fingerprint)
}

/// Starts a viewer pinned to `fingerprint`, which is not the host's when a test says so.
pub(crate) fn start_viewer_pinned_to(host: &Host, fingerprint: &str) -> Viewer {
    Viewer {
        state: crate::server::tests::test_state(),
        client: client::pinned_client(fingerprint, READ_TIMEOUT).unwrap(),
        base: format!("https://{}", host.addr),
    }
}

/// A device the viewer paired as: its token and signing key.
pub(crate) struct Paired {
    pub device_id: String,
    pub token: String,
    key: EcdsaKeyPair,
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn now_ms() -> i64 {
    agentmux_common::time::now_ms()
}

impl Viewer {
    /// A path of the host's viewer listener as a URL.
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// Pairs with `host` the way a phone does: a one-time code, the device's public key; the host answers with
    /// the device's id and token.
    pub(crate) async fn pair(&self, host: &Host, name: &str) -> Result<Paired, String> {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).map_err(|e| e.to_string())?;
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng).map_err(|e| e.to_string())?;
        let body = serde_json::json!({
            "code": host.new_pairing_code(),
            "device_name": name,
            "device_key": b64(key.public_key().as_ref()),
        });
        let resp = self.client.post(self.url("/agentmux/viewer/pair")).json(&body).send().await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("pairing refused: {}", resp.status()));
        }
        let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        let field = |k: &str| json[k].as_str().map(str::to_string).ok_or_else(|| format!("the answer has no {k}"));
        Ok(Paired { device_id: field("device_id")?, token: field("token")?, key })
    }

    /// A request with the device's token. `Err` when the connection itself failed (a wrong pin ends here).
    pub(crate) async fn get(&self, device: &Paired, path: &str) -> Result<reqwest::Response, String> {
        self.client.get(self.url(path)).bearer_auth(&device.token).send().await.map_err(|e| e.to_string())
    }

    /// Opens an agent's feed on the host.
    pub(crate) async fn open_feed(&self, device: &Paired, agent: &str) -> Result<Feed, String> {
        let resp = self.get(device, &format!("/agentmux/viewer/agents/{agent}/feed")).await?;
        if !resp.status().is_success() {
            return Err(format!("the feed was refused: {}", resp.status()));
        }
        Ok(Feed { resp, buf: String::new() })
    }

    /// Sends `message` to the agent. Signed with the device's key when `signed`, as the app does; an unsigned send
    /// is what a stolen token alone could make. The status and the JSON answer.
    pub(crate) async fn send(&self, device: &Paired, agent: &str, message: &str, signed: bool) -> Result<(reqwest::StatusCode, serde_json::Value), String> {
        let path = format!("/agentmux/viewer/agents/{agent}/input");
        let body = serde_json::json!({ "message": message, "message_id": uuid::Uuid::new_v4().to_string() }).to_string().into_bytes();
        let mut req = self.client.post(self.url(&path)).bearer_auth(&device.token).header("Content-Type", "application/json");
        if signed {
            let ts = now_ms();
            let text = signing::signed_text("POST", &path, ts, &body);
            let sig = device.key.sign(&SystemRandom::new(), text.as_bytes()).map_err(|e| e.to_string())?;
            req = req.header("X-Amxv-Timestamp", ts.to_string()).header("X-Amxv-Signature", b64(sig.as_ref()));
        }
        let resp = req.body(body).send().await.map_err(|e| e.to_string())?;
        let status = resp.status();
        Ok((status, resp.json().await.unwrap_or(serde_json::Value::Null)))
    }
}

/// An open feed: the host's server-sent events, read off the socket.
pub(crate) struct Feed {
    resp: reqwest::Response,
    buf: String,
}

impl Feed {
    /// The next event: `(name, id, data)`, heartbeats skipped. `None` when the host ends the feed or says nothing
    /// for [`READ_TIMEOUT`].
    pub(crate) async fn next_event(&mut self) -> Option<(String, Option<String>, serde_json::Value)> {
        loop {
            if let Some(end) = self.buf.find("\n\n") {
                let frame: String = self.buf.drain(..end + 2).collect();
                if frame.starts_with(':') || frame.starts_with("retry:") {
                    continue;
                }
                let (mut name, mut id, mut data) = (String::new(), None, serde_json::Value::Null);
                for line in frame.lines() {
                    if let Some(v) = line.strip_prefix("event: ") {
                        name = v.to_string();
                    } else if let Some(v) = line.strip_prefix("id: ") {
                        id = Some(v.to_string());
                    } else if let Some(v) = line.strip_prefix("data: ") {
                        data = serde_json::from_str(v).unwrap_or(serde_json::Value::Null);
                    }
                }
                return Some((name, id, data));
            }
            let chunk = tokio::time::timeout(READ_TIMEOUT, self.resp.chunk()).await.ok()?.ok()??;
            self.buf.push_str(&String::from_utf8_lossy(&chunk));
        }
    }
}
