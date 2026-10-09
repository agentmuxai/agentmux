// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Other AgentMux computers on the LAN in Tower
//! (SPEC_TOWER_TASK_MANAGER_PANE_2026_10_08.md §8.3): this computer pairs as
//! a viewer device of the other one, the way a phone does, and reads its
//! processes from the viewer listener.
//!
//! - **Pairing:** the other computer's pairing link (`agentmux://pair?…`,
//!   its "Pair a device" panel) carries its address, viewer port, certificate
//!   fingerprint and a one-time code. [`pair`] redeems the code for a viewer
//!   token over TLS pinned to that fingerprint (`viewer::client`).
//! - **What is kept:** the peer's address, port and fingerprint in
//!   `tower-peers.json` in the data directory; the token in the secret store
//!   (the OS keychain), never in the file.
//! - **Reading:** `GET /agentmux/viewer/procs` with the token. The other
//!   computer answers only if its user turned sharing on, and its user can
//!   revoke this device from their paired-device list.
//! - **A moved peer:** the fingerprint, not the address, is what is trusted,
//!   so when the stored address doesn't answer, the address LAN discovery
//!   now has for that hostname is tried, and kept if it answers.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::backend::rpc_types::{TowerPeerInfo, TowerSnapshot};
use crate::backend::viewer::client::pinned_client;

/// `TowerSampleReq::connection` for a paired computer: `peer:<id>`.
pub const PEER_PREFIX: &str = "peer:";

const FILE: &str = "tower-peers.json";
const TIMEOUT: Duration = Duration::from_secs(8);

/// One paired computer, as kept on disk (no secret here).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Peer {
    pub id: String,
    pub hostname: String,
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
    /// This computer's device id on the peer (what its user revokes).
    pub device_id: String,
    pub paired_ms: i64,
}

impl Peer {
    fn info(&self) -> TowerPeerInfo {
        TowerPeerInfo {
            connection: format!("{PEER_PREFIX}{}", self.id),
            hostname: self.hostname.clone(),
            address: format!("{}:{}", self.host, self.port),
            paired_ms: self.paired_ms,
        }
    }

    fn secret_key(&self) -> String {
        format!("tower-peer:{}", self.id)
    }

    fn url(&self, host: &str, path: &str) -> String {
        url_at(host, self.port, path)
    }
}

fn url_at(host: &str, port: u16, path: &str) -> String {
    // An IPv6 address needs brackets in a URL.
    let host = if host.contains(':') && !host.starts_with('[') { format!("[{host}]") } else { host.to_string() };
    format!("https://{host}:{port}{path}")
}

/// The viewer ports `host`'s AgentMux instances advertise now, from the LAN
/// discovery probe a phone sends (UDP 47891): the answering instance's, then
/// its siblings' (other channels on that host). Empty when nothing answers.
async fn probe_viewer_ports(host: &str) -> Vec<u16> {
    use crate::backend::lan_discovery::{UDP_DISCOVERY_PORT, UDP_PROBE_TYPE, UDP_PROTOCOL_VERSION};
    let Ok(ip) = host.parse::<std::net::IpAddr>() else { return Vec::new() };
    let bind = if ip.is_ipv6() { "[::]:0" } else { "0.0.0.0:0" };
    let Ok(sock) = tokio::net::UdpSocket::bind(bind).await else { return Vec::new() };
    let probe = serde_json::json!({ "type": UDP_PROBE_TYPE, "v": UDP_PROTOCOL_VERSION }).to_string();
    if sock.send_to(probe.as_bytes(), (ip, UDP_DISCOVERY_PORT)).await.is_err() {
        return Vec::new();
    }
    let mut buf = vec![0u8; 8192];
    match tokio::time::timeout(Duration::from_millis(1500), sock.recv_from(&mut buf)).await {
        Ok(Ok((n, _))) => viewer_ports_of(&buf[..n]),
        _ => Vec::new(),
    }
}

/// The viewer ports in a discovery reply: its own, then its siblings'.
fn viewer_ports_of(reply: &[u8]) -> Vec<u16> {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(reply) else { return Vec::new() };
    let port = |x: &serde_json::Value| x.get("viewer_port").and_then(|p| p.as_u64()).and_then(|p| u16::try_from(p).ok());
    let mut ports: Vec<u16> = port(&v).into_iter().collect();
    for s in v.get("siblings").and_then(|s| s.as_array()).into_iter().flatten() {
        if let Some(p) = port(s).filter(|p| !ports.contains(p)) {
            ports.push(p);
        }
    }
    ports
}

/// What a pairing link says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
    pub code: String,
    pub hostname: String,
}

/// Parse an `agentmux://pair?v=1&host=…&port=…&fp=…&code=…&hostname=…` link.
pub fn parse_link(link: &str) -> Result<Link, String> {
    let bad = |why: &str| format!("That isn't a pairing link ({why}). Copy it from the other computer's \u{201c}Pair a device\u{201d} panel.");
    let url = url::Url::parse(link.trim()).map_err(|_| bad("not a link"))?;
    if url.scheme() != "agentmux" || url.host_str() != Some("pair") {
        return Err(bad("not an agentmux://pair link"));
    }
    let get = |k: &str| url.query_pairs().find(|(key, _)| key == k).map(|(_, v)| v.into_owned());
    if get("v").as_deref() != Some("1") {
        return Err(bad("an unknown version"));
    }
    let fingerprint = get("fp").filter(|f| f.len() == 64 && f.bytes().all(|b| b.is_ascii_hexdigit())).ok_or_else(|| bad("no fingerprint"))?;
    let host = get("host").filter(|h| h.parse::<std::net::IpAddr>().is_ok()).ok_or_else(|| bad("no address"))?;
    Ok(Link {
        host,
        port: get("port").and_then(|p| p.parse().ok()).filter(|&p| p > 0).ok_or_else(|| bad("no port"))?,
        fingerprint: fingerprint.to_ascii_lowercase(),
        code: get("code").filter(|c| !c.is_empty()).ok_or_else(|| bad("no code"))?,
        hostname: get("hostname").unwrap_or_default(),
    })
}

/// Where peers' tokens are kept: the secret store (the OS keychain) in the
/// app; in memory in tests, which must not touch the user's keychain.
pub trait Tokens: Send + Sync {
    fn put(&self, key: &str, token: &str) -> Result<(), String>;
    fn get(&self, key: &str) -> Result<String, String>;
    fn delete(&self, key: &str);
}

struct SecretStore;

impl Tokens for SecretStore {
    fn put(&self, key: &str, token: &str) -> Result<(), String> {
        crate::identity::secret_store::put(key, token)
    }
    fn get(&self, key: &str) -> Result<String, String> {
        crate::identity::secret_store::get(key).map(|t| t.as_str().to_string())
    }
    fn delete(&self, key: &str) {
        let _ = crate::identity::secret_store::delete(key);
    }
}

/// The kept peers, from `dir` (the data directory).
pub struct Peers {
    dir: Option<PathBuf>,
    lock: Mutex<()>,
    tokens: Box<dyn Tokens>,
}

impl Peers {
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self::with_tokens(dir, Box::new(SecretStore))
    }

    pub fn with_tokens(dir: Option<PathBuf>, tokens: Box<dyn Tokens>) -> Self {
        Self { dir, lock: Mutex::new(()), tokens }
    }

    pub fn global() -> &'static Peers {
        static GLOBAL: std::sync::OnceLock<Peers> = std::sync::OnceLock::new();
        GLOBAL.get_or_init(|| Peers::new(Some(crate::backend::base::get_mux_data_dir())))
    }

    fn path(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(FILE))
    }

    fn load(&self) -> Vec<Peer> {
        self.path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save(&self, peers: &[Peer]) -> Result<(), String> {
        let path = self.path().ok_or("no data directory")?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(peers).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }

    pub fn list(&self) -> Vec<TowerPeerInfo> {
        let _g = self.lock.lock().unwrap();
        self.load().iter().map(Peer::info).collect()
    }

    pub fn get(&self, id: &str) -> Option<Peer> {
        let _g = self.lock.lock().unwrap();
        self.load().into_iter().find(|p| p.id == id)
    }

    fn put(&self, peer: Peer) -> Result<(), String> {
        let _g = self.lock.lock().unwrap();
        let mut all = self.load();
        all.retain(|p| p.id != peer.id);
        all.push(peer);
        self.save(&all)
    }

    /// Forget a peer here (its token too). The other computer still lists
    /// this device until its user revokes it.
    pub fn forget(&self, id: &str) -> Result<(), String> {
        let _g = self.lock.lock().unwrap();
        let mut all = self.load();
        let Some(peer) = all.iter().find(|p| p.id == id).cloned() else { return Ok(()) };
        all.retain(|p| p.id != id);
        self.save(&all)?;
        self.tokens.delete(&peer.secret_key());
        Ok(())
    }
}

#[derive(Deserialize)]
struct PairReply {
    token: String,
    device_id: String,
    #[serde(default)]
    hostname: String,
}

/// Pair with the computer the link names, as `device_name` (this computer).
pub async fn pair(peers: &'static Peers, link: &str, device_name: &str) -> Result<TowerPeerInfo, String> {
    let link = parse_link(link)?;
    let client = pinned_client(&link.fingerprint, TIMEOUT)?;
    let probe = Peer {
        id: uuid::Uuid::new_v4().to_string(),
        hostname: link.hostname.clone(),
        host: link.host.clone(),
        port: link.port,
        fingerprint: link.fingerprint.clone(),
        device_id: String::new(),
        paired_ms: agentmux_common::time::now_ms(),
    };
    let resp = client
        .post(probe.url(&link.host, "/agentmux/viewer/pair"))
        .json(&serde_json::json!({ "code": link.code, "device_name": device_name }))
        .send()
        .await
        .map_err(|e| unreachable(&probe, &e))?;
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err("The code was refused: it works once, for two minutes. Show a new one on the other computer.".into());
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err("Too many wrong codes: wait a minute and try again.".into());
    }
    if !status.is_success() {
        return Err(format!("The other computer refused to pair ({status})."));
    }
    let reply: PairReply = resp.json().await.map_err(|e| format!("The other computer answered oddly: {e}"))?;
    let peer = Peer {
        hostname: if reply.hostname.is_empty() { link.hostname } else { reply.hostname },
        device_id: reply.device_id,
        ..probe
    };
    // The keychain write may wait on the OS asking the user's consent, which
    // can't be cancelled: never on an async worker.
    let info = peer.info();
    let token = reply.token;
    tokio::task::spawn_blocking(move || {
        peers.tokens.put(&peer.secret_key(), &token)?;
        if let Err(e) = peers.put(peer.clone()) {
            peers.tokens.delete(&peer.secret_key());
            return Err(e);
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(info)
}

fn unreachable(peer: &Peer, e: &reqwest::Error) -> String {
    let why = if e.is_timeout() {
        "it didn't answer".to_string()
    } else if format!("{e:?}").contains("fingerprint") {
        "its certificate isn't the one it was paired with".to_string()
    } else {
        "the connection failed".to_string()
    };
    format!("Can't reach {} at {}:{} ({why}).", if peer.hostname.is_empty() { "the other computer" } else { &peer.hostname }, peer.host, peer.port)
}

/// A paired computer's snapshot. `moved`: the addresses LAN discovery has for
/// its hostname, tried in turn when the kept one doesn't answer.
pub async fn sample(peers: &'static Peers, id: &str, filter: &str, moved: Vec<String>) -> Result<TowerSnapshot, String> {
    let mut peer = peers.get(id).ok_or("That computer is no longer paired.")?;
    // A keychain read (bounded, but blocking): off the async workers.
    let key = peer.secret_key();
    let token = tokio::task::spawn_blocking(move || peers.tokens.get(&key))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("Couldn't read the pairing with {}: {e}", peer.hostname))?;
    let client = pinned_client(&peer.fingerprint, TIMEOUT)?;
    let mut hosts = vec![peer.host.clone()];
    hosts.extend(moved.into_iter().filter(|h| *h != peer.host));
    // The kept port at each address first; only if none answers, the ports the
    // addresses advertise now (a restarted peer may have another). The pinned
    // fingerprint decides which of them is really this peer.
    let mut candidates: Vec<(String, u16)> = hosts.iter().map(|h| (h.clone(), peer.port)).collect();
    let mut probed = false;
    let mut next = 0;
    let mut last_err = String::new();
    loop {
        if next == candidates.len() {
            if probed {
                break;
            }
            probed = true;
            for h in &hosts {
                for p in probe_viewer_ports(h).await {
                    if !candidates.contains(&(h.clone(), p)) {
                        candidates.push((h.clone(), p));
                    }
                }
            }
            if next == candidates.len() {
                break;
            }
        }
        let (host, port) = candidates[next].clone();
        next += 1;
        let resp = match client
            .get(url_at(&host, port, "/agentmux/viewer/procs"))
            .query(&[("filter", filter)])
            .bearer_auth(&token)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                last_err = unreachable(&Peer { host: host.clone(), port, ..peer.clone() }, &e);
                continue;
            }
        };
        match resp.status() {
            s if s.is_success() => {
                if host != peer.host || port != peer.port {
                    (peer.host, peer.port) = (host, port);
                    let _ = peers.put(peer.clone());
                }
                let mut snap: TowerSnapshot = resp.json().await.map_err(|e| format!("{} answered oddly: {e}", peer.hostname))?;
                snap.remote = true;
                return Ok(snap);
            }
            reqwest::StatusCode::UNAUTHORIZED => {
                return Err(format!("{} no longer accepts this computer (it was unpaired there). Forget it and pair again.", peer.hostname))
            }
            reqwest::StatusCode::FORBIDDEN => {
                return Err(format!("{} doesn't share its processes. Turn on \u{201c}Share this computer's processes with paired devices\u{201d} in its Tower.", peer.hostname))
            }
            s => return Err(format!("{} refused ({s}).", peer.hostname)),
        }
    }
    Err(last_err)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FP: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn a_pairing_link_is_read_and_anything_else_refused() {
        let link = format!("agentmux://pair?v=1&host=198.51.100.20&port=47900&fp={FP}&code=ABCDEFGHJK&hostname=studio&channel=stable");
        assert_eq!(
            parse_link(&link).unwrap(),
            Link { host: "198.51.100.20".into(), port: 47900, fingerprint: FP.into(), code: "ABCDEFGHJK".into(), hostname: "studio".into() }
        );
        assert!(parse_link("https://example.com/pair?v=1").is_err());
        assert!(parse_link(&link.replace("v=1", "v=9")).is_err(), "unknown version");
        assert!(parse_link(&link.replace(FP, "abc")).is_err(), "short fingerprint");
        assert!(parse_link(&link.replace("198.51.100.20", "evil.example")).is_err(), "an address, not a name");
        assert!(parse_link(&link.replace("&code=ABCDEFGHJK", "")).is_err());
    }

    /// Tokens in memory: tests never touch the user's keychain.
    #[derive(Default)]
    struct MemTokens(std::sync::Mutex<std::collections::HashMap<String, String>>);

    impl Tokens for std::sync::Arc<MemTokens> {
        fn put(&self, key: &str, token: &str) -> Result<(), String> {
            self.0.lock().unwrap().insert(key.into(), token.into());
            Ok(())
        }
        fn get(&self, key: &str) -> Result<String, String> {
            self.0.lock().unwrap().get(key).cloned().ok_or_else(|| "no token".into())
        }
        fn delete(&self, key: &str) {
            self.0.lock().unwrap().remove(key);
        }
    }

    /// Peers for one test, kept for the test binary's life (`pair` and
    /// `sample` take `'static`, as the app's `Peers::global()` is).
    fn peers_in(dir: &std::path::Path) -> (&'static Peers, std::sync::Arc<MemTokens>) {
        let tokens = std::sync::Arc::new(MemTokens::default());
        (Box::leak(Box::new(Peers::with_tokens(Some(dir.to_path_buf()), Box::new(tokens.clone())))), tokens)
    }

    #[test]
    fn a_discovery_reply_gives_its_viewer_port_then_its_siblings() {
        let reply = br#"{"type":"agentmux_discover_response","v":1,"viewer_port":29702,
            "siblings":[{"channel":"dev","viewer_port":29703},{"channel":"x"},{"viewer_port":29702}]}"#;
        assert_eq!(viewer_ports_of(reply), vec![29702, 29703]);
        assert!(viewer_ports_of(b"not json").is_empty());
        assert!(viewer_ports_of(br#"{"type":"agentmux_discover_response"}"#).is_empty(), "no viewer listener up");
    }

    #[test]
    fn peers_are_kept_without_their_token_and_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let (peers, _) = peers_in(dir.path());
        let peer = Peer {
            id: "p1".into(),
            hostname: "studio".into(),
            host: "2001:db8::1".into(),
            port: 47900,
            fingerprint: FP.into(),
            device_id: "d1".into(),
            paired_ms: 5,
        };
        peers.put(peer.clone()).unwrap();
        let list = peers.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].connection, "peer:p1");
        assert_eq!(peer.url("2001:db8::1", "/x"), "https://[2001:db8::1]:47900/x");
        let on_disk = std::fs::read_to_string(dir.path().join(FILE)).unwrap();
        assert!(!on_disk.contains("amxv_"), "no token on disk");
        peers.forget("p1").unwrap();
        assert!(peers.list().is_empty());
    }

    /// Nothing listening: an error that says so, and nothing kept.
    #[tokio::test]
    async fn pairing_with_nothing_listening_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let (peers, _) = peers_in(dir.path());
        // Nothing listens on this port; the error is about reaching it.
        let link = format!("agentmux://pair?v=1&host=127.0.0.1&port=9&fp={FP}&code=ABCDEFGHJK&hostname=studio");
        let err = pair(peers, &link, "me").await.unwrap_err();
        assert!(err.contains("Can't reach studio"), "{err}");
        assert!(peers.list().is_empty(), "nothing kept");
    }

    /// End to end against a real viewer listener over TLS: pair with the
    /// link, get refused until the user shares, then read the snapshot; a
    /// link pinning another certificate never pairs; a revoked device is
    /// told so.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_peer_pairs_over_pinned_tls_and_reads_once_sharing_is_on() {
        let viewer_dir = tempfile::tempdir().unwrap();
        let mut state = crate::server::tests::test_state();
        let broker = crate::backend::mps::Broker::new();
        state.viewer = crate::backend::viewer::ViewerService::new(Some(viewer_dir.path().to_path_buf()), &broker);
        let tls = state.viewer.tls().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let token = tokio_util::sync::CancellationToken::new();
        crate::backend::lan_listeners::LanListenerSupervisor::serve_tls(
            crate::server::build_viewer_router(state.clone()),
            listener,
            tokio_rustls::TlsAcceptor::from(tls.server_config().unwrap()),
            token.clone(),
        );
        let link = |fp: &str| {
            let (code, _) = state.viewer.pairing.start();
            format!("agentmux://pair?v=1&host=127.0.0.1&port={port}&fp={fp}&code={code}&hostname=studio")
        };

        let dir = tempfile::tempdir().unwrap();
        let (peers, tokens) = peers_in(dir.path());
        // Another certificate's fingerprint: the handshake fails, no pairing.
        let err = pair(peers, &link(FP), "laptop (Tower)").await.unwrap_err();
        assert!(err.contains("Can't reach studio"), "{err}");
        assert!(peers.list().is_empty());

        let info = pair(peers, &link(&tls.fingerprint), "laptop (Tower)").await.unwrap();
        let id = info.connection.strip_prefix(PEER_PREFIX).unwrap().to_string();
        assert_eq!(tokens.0.lock().unwrap().len(), 1, "the token went to the token store");
        assert!(!std::fs::read_to_string(dir.path().join(FILE)).unwrap().contains("amxv_"));

        let err = sample(peers, &id, "", vec![]).await.unwrap_err();
        assert!(err.contains("doesn't share"), "{err}");

        let mut settings = state.config_watcher.get_settings();
        settings.extra.insert("tower:sharewithpaired".to_string(), serde_json::json!(true));
        state.config_watcher.update_settings(settings);
        let snap = sample(peers, &id, "", vec![]).await.unwrap();
        assert!(snap.remote);
        assert!(snap.host.unwrap().total > 0);

        // The other computer's user revokes this device.
        let device = peers.get(&id).unwrap().device_id;
        state.mstore.viewer_device_delete(&device).unwrap();
        let err = sample(peers, &id, "", vec![]).await.unwrap_err();
        assert!(err.contains("no longer accepts"), "{err}");
        token.cancel();
    }
}
