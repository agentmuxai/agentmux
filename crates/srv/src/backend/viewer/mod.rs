// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The viewer: a read-only window onto this channel's agents for a paired
//! device (agentmux-mobile's SPEC_AGENT_STATUS_AND_LIVE_PANE_FEED_2026_10_07
//! §4, §5, §13.2, §13.3).
//!
//! What it adds next to the LAN routes, and why each part exists:
//! - a credential that is not public: a per-device viewer token, handed out
//!   once against a pairing code shown on this screen, stored hashed, and
//!   revocable (`pairing`, `storage::viewer_devices`). Never the `lan_key`
//!   (anyone on the network has it) and never the instance `auth_key`.
//! - encryption on the wire: the viewer routes are served only on a TLS
//!   listener whose self-signed certificate the device pins by the
//!   fingerprint in the QR (`cert`). The listener follows LAN discovery like
//!   the other LAN listeners (`lan_listeners`), on its own port of the LAN
//!   port range.
//! - the feed itself: an agent's transcript, a bounded snapshot then live
//!   appends (`feed`, the route in `server::http_viewer`).

pub mod cert;
pub mod client;
pub mod feed;
pub mod pairing;

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, OnceLock};

use tokio_util::sync::CancellationToken;

/// Open feeds one device may hold; the next gets 503.
pub const MAX_FEEDS_PER_DEVICE: usize = 4;
/// Open feeds this srv serves in all; the next gets 503.
pub const MAX_FEEDS: usize = 16;
/// A device's `last_seen_ms` is written at most this often.
pub const LAST_SEEN_EVERY_MS: i64 = 60_000;

/// Why a feed was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedRefused {
    PerDevice,
    Total,
}

#[derive(Default)]
struct FeedRegistry {
    by_device: HashMap<String, Vec<(u64, CancellationToken)>>,
    next_id: u64,
}

impl FeedRegistry {
    fn total(&self) -> usize {
        self.by_device.values().map(Vec::len).sum()
    }
}

/// One open feed's place under the caps. Dropping it frees the place;
/// `token` is cancelled when the device is revoked.
pub struct FeedLease {
    registry: Arc<parking_lot::Mutex<FeedRegistry>>,
    device_id: String,
    id: u64,
    pub token: CancellationToken,
}

impl Drop for FeedLease {
    fn drop(&mut self) {
        let mut reg = self.registry.lock();
        if let Some(list) = reg.by_device.get_mut(&self.device_id) {
            list.retain(|(id, _)| *id != self.id);
            if list.is_empty() {
                reg.by_device.remove(&self.device_id);
            }
        }
    }
}

/// The port the viewer listener is advertised on, process-wide, for the UDP
/// identity replies (which have no `AppState` in hand). Installed once by
/// `bootstrap`; absent in tests.
static ADVERTISED_PORT: OnceLock<Arc<AtomicU16>> = OnceLock::new();

/// The viewer listener's port while at least one viewer listener is bound.
pub fn advertised_viewer_port() -> Option<u16> {
    ADVERTISED_PORT.get().map(|p| p.load(Ordering::Relaxed)).filter(|p| *p != 0)
}

pub struct ViewerService {
    /// Where the certificate is kept; `None` keeps it in memory only.
    data_dir: Option<PathBuf>,
    tls: parking_lot::Mutex<Option<Arc<cert::ViewerTls>>>,
    pub pairing: pairing::Pairing,
    pub hub: Arc<feed::FeedHub>,
    feeds: Arc<parking_lot::Mutex<FeedRegistry>>,
    max_per_device: usize,
    max_total: usize,
    /// 0 while no viewer listener is bound.
    advertised_port: Arc<AtomicU16>,
    /// The addresses viewer listeners are bound on, for the QR's `host`.
    bound: parking_lot::Mutex<Vec<IpAddr>>,
    /// The viewer port, held on loopback (bound, never listening) from the
    /// first time the listener comes up until exit, so another instance on
    /// this host never takes it from the range and the port a device saw
    /// stays ours across LAN off/on.
    reservation: parking_lot::Mutex<Option<(u16, socket2::Socket)>>,
    /// When each device's `last_seen_ms` was last written.
    last_seen_written: parking_lot::Mutex<HashMap<String, i64>>,
}

impl ViewerService {
    /// A viewer whose feeds watch `broker`.
    pub fn new(data_dir: Option<PathBuf>, broker: &crate::backend::mps::Broker) -> Arc<Self> {
        Self::with_limits(data_dir, broker, Arc::new(feed::FeedHub::default()), MAX_FEEDS_PER_DEVICE, MAX_FEEDS)
    }

    pub fn with_limits(
        data_dir: Option<PathBuf>,
        broker: &crate::backend::mps::Broker,
        hub: Arc<feed::FeedHub>,
        max_per_device: usize,
        max_total: usize,
    ) -> Arc<Self> {
        hub.install(broker);
        Arc::new(Self {
            data_dir,
            tls: parking_lot::Mutex::new(None),
            pairing: pairing::Pairing::new(),
            hub,
            feeds: Arc::new(parking_lot::Mutex::new(FeedRegistry::default())),
            max_per_device,
            max_total,
            advertised_port: Arc::new(AtomicU16::new(0)),
            bound: parking_lot::Mutex::new(Vec::new()),
            reservation: parking_lot::Mutex::new(None),
            last_seen_written: parking_lot::Mutex::new(HashMap::new()),
        })
    }

    /// Make this the process's advertised viewer port (`advertised_viewer_port`).
    pub fn install_global(&self) {
        let _ = ADVERTISED_PORT.set(Arc::clone(&self.advertised_port));
    }

    /// The handle the fleet feed reads `viewer_port` from.
    pub fn advertised_port_handle(&self) -> Arc<AtomicU16> {
        Arc::clone(&self.advertised_port)
    }

    pub fn advertised_port(&self) -> Option<u16> {
        Some(self.advertised_port.load(Ordering::Relaxed)).filter(|p| *p != 0)
    }

    /// The certificate, made and kept on first need.
    pub fn tls(&self) -> Result<Arc<cert::ViewerTls>, String> {
        let mut slot = self.tls.lock();
        if let Some(tls) = slot.as_ref() {
            return Ok(Arc::clone(tls));
        }
        let tls = Arc::new(match &self.data_dir {
            Some(dir) => cert::load_or_create(dir)?,
            None => return Err("the viewer has no data directory for its certificate".to_string()),
        });
        *slot = Some(Arc::clone(&tls));
        Ok(tls)
    }

    /// The viewer port: the one already held, else the first port of
    /// `candidates` free on loopback, now held. `None` when none is free.
    pub fn reserve_port(&self, candidates: impl IntoIterator<Item = u16>) -> Option<u16> {
        let mut slot = self.reservation.lock();
        if let Some((port, _)) = slot.as_ref() {
            return Some(*port);
        }
        for port in candidates {
            let Ok(socket) = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None) else {
                continue;
            };
            let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
            if socket.bind(&addr.into()).is_ok() {
                *slot = Some((port, socket));
                return Some(port);
            }
        }
        None
    }

    /// Record which addresses viewer listeners are bound on, and advertise
    /// the port while there is at least one.
    pub fn set_bound(&self, addrs: Vec<IpAddr>, port: u16) {
        let up = !addrs.is_empty();
        *self.bound.lock() = addrs;
        self.advertised_port.store(if up { port } else { 0 }, Ordering::Relaxed);
    }

    /// The IPv4 address the QR names: the primary outbound one when a viewer
    /// listener is bound there, else the first bound IPv4 address.
    pub fn pair_host(&self) -> Option<IpAddr> {
        let bound = self.bound.lock().clone();
        let primary = crate::backend::lan_listeners::primary_lan_ipv4();
        primary
            .filter(|p| bound.contains(p))
            .or_else(|| bound.iter().copied().find(IpAddr::is_ipv4))
    }

    /// Take a place for one more feed of `device_id`, or say why not.
    pub fn open_feed(&self, device_id: &str) -> Result<FeedLease, FeedRefused> {
        let mut reg = self.feeds.lock();
        if reg.total() >= self.max_total {
            return Err(FeedRefused::Total);
        }
        if reg.by_device.get(device_id).map_or(0, Vec::len) >= self.max_per_device {
            return Err(FeedRefused::PerDevice);
        }
        reg.next_id += 1;
        let id = reg.next_id;
        let token = CancellationToken::new();
        reg.by_device.entry(device_id.to_string()).or_default().push((id, token.clone()));
        Ok(FeedLease { registry: Arc::clone(&self.feeds), device_id: device_id.to_string(), id, token })
    }

    /// Close every open feed of `device_id` (it was revoked). Returns how many.
    pub fn close_device_feeds(&self, device_id: &str) -> usize {
        let reg = self.feeds.lock();
        let Some(list) = reg.by_device.get(device_id) else { return 0 };
        for (_, token) in list {
            token.cancel();
        }
        list.len()
    }

    #[cfg(test)]
    pub fn open_feeds(&self) -> usize {
        self.feeds.lock().total()
    }

    /// Whether `device_id`'s `last_seen_ms` is due to be written at `now_ms`;
    /// records it when it is.
    pub fn last_seen_due(&self, device_id: &str, now_ms: i64) -> bool {
        let mut written = self.last_seen_written.lock();
        match written.get(device_id) {
            Some(at) if now_ms - at < LAST_SEEN_EVERY_MS => false,
            _ => {
                written.insert(device_id.to_string(), now_ms);
                true
            }
        }
    }

    /// Forget a revoked device's throttle entry.
    pub fn forget_device(&self, device_id: &str) {
        self.last_seen_written.lock().remove(device_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(per_device: usize, total: usize) -> Arc<ViewerService> {
        let broker = crate::backend::mps::Broker::new();
        ViewerService::with_limits(None, &broker, Arc::new(feed::FeedHub::default()), per_device, total)
    }

    #[test]
    fn feeds_are_capped_per_device_and_in_all() {
        let v = service(2, 3);
        let a1 = v.open_feed("a").unwrap();
        let _a2 = v.open_feed("a").unwrap();
        assert_eq!(v.open_feed("a").err(), Some(FeedRefused::PerDevice));
        let _b1 = v.open_feed("b").unwrap();
        assert_eq!(v.open_feed("c").err(), Some(FeedRefused::Total));
        drop(a1);
        assert_eq!(v.open_feeds(), 2);
        assert!(v.open_feed("c").is_ok(), "a closed feed frees its place");
    }

    #[test]
    fn closing_a_devices_feeds_cancels_only_its_own() {
        let v = service(4, 16);
        let a1 = v.open_feed("a").unwrap();
        let a2 = v.open_feed("a").unwrap();
        let b1 = v.open_feed("b").unwrap();
        assert_eq!(v.close_device_feeds("a"), 2);
        assert!(a1.token.is_cancelled() && a2.token.is_cancelled());
        assert!(!b1.token.is_cancelled());
        assert_eq!(v.close_device_feeds("nobody"), 0);
    }

    #[test]
    fn last_seen_is_written_at_most_once_a_minute() {
        let v = service(4, 16);
        assert!(v.last_seen_due("a", 1_000));
        assert!(!v.last_seen_due("a", 1_000 + LAST_SEEN_EVERY_MS - 1));
        assert!(v.last_seen_due("b", 1_000), "per device");
        assert!(v.last_seen_due("a", 1_000 + LAST_SEEN_EVERY_MS));
    }

    #[test]
    fn the_port_is_reserved_once_and_skips_a_taken_one() {
        let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let taken_port = taken.local_addr().unwrap().port();
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let v = service(4, 16);
        assert_eq!(v.reserve_port([taken_port, free]), Some(free));
        assert!(
            std::net::TcpListener::bind(("127.0.0.1", free)).is_err(),
            "the reservation holds the port against other instances"
        );
        assert_eq!(v.reserve_port([taken_port]), Some(free), "kept, not chosen again");
    }

    #[test]
    fn the_port_is_advertised_only_while_bound() {
        let v = service(4, 16);
        assert_eq!(v.advertised_port(), None);
        v.set_bound(vec![IpAddr::from([198, 51, 100, 7])], 29702);
        assert_eq!(v.advertised_port(), Some(29702));
        assert_eq!(v.pair_host(), Some(IpAddr::from([198, 51, 100, 7])));
        v.set_bound(Vec::new(), 29702);
        assert_eq!(v.advertised_port(), None);
        assert_eq!(v.pair_host(), None);
    }

    #[test]
    fn without_a_data_directory_there_is_no_certificate() {
        assert!(service(4, 16).tls().is_err());
    }
}
