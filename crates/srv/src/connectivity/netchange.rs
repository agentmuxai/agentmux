// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The OS's network-change notifications, so a switch of Wi-Fi, a VPN coming up or a cable pulled
//! shows at once, not at the next probe of a service already believed down (up to a minute) or at
//! the next failed request.
//!
//! A change makes every service due for a probe ([`super::Shared::network_changed_at`]). The OS
//! reports a change as a burst (the link, then its addresses, then its routes, and a Wi-Fi roam
//! repeats it), and right after it DNS often can't answer yet, so [`settle`] waits for the burst to
//! end and probes once.
//!
//! **Platforms.** Windows has it (`NotifyIpInterfaceChange`). Linux (netlink) and macOS
//! (`NWPathMonitor`) don't yet: [`Watch::start`] returns `None` there and the probes run as
//! before, on their own schedule. This is a known gap, not an error.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;

use super::Shared;

/// How long after the last notification the network is taken to have settled.
const SETTLE: Duration = Duration::from_secs(2);

/// Waits for a notification, then for [`SETTLE`] with none, and calls `changed` once for the burst.
/// Runs until srv exits.
async fn settle<F, Fut>(signal: Arc<Notify>, mut changed: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = ()>,
{
    loop {
        signal.notified().await;
        // Each notification starts the wait over.
        while tokio::time::timeout(SETTLE, signal.notified()).await.is_ok() {}
        changed().await;
    }
}

/// Starts watching the OS for network changes, on behalf of `shared`. The watch lasts as long as
/// srv does.
pub(super) fn start(shared: Arc<Shared>) {
    let signal = Arc::new(Notify::new());
    let Some(watch) = Watch::start(signal.clone()) else {
        tracing::info!("connectivity: no OS network-change notifications on this platform");
        return;
    };
    // Held by the task, so the registration ends with it.
    tokio::spawn(async move {
        let _watch = watch;
        settle(signal, || {
            let shared = shared.clone();
            async move { shared.network_changed_at(agentmux_common::time::now_ms()) }
        })
        .await;
    });
}

#[cfg(windows)]
pub(super) use windows_watch::Watch;

#[cfg(not(windows))]
pub(super) use no_watch::Watch;

#[cfg(not(windows))]
mod no_watch {
    use std::sync::Arc;

    use tokio::sync::Notify;

    /// The platforms without OS notifications.
    pub(in super::super) struct Watch;

    impl Watch {
        pub(in super::super) fn start(_signal: Arc<Notify>) -> Option<Watch> {
            None
        }
    }
}

#[cfg(windows)]
mod windows_watch {
    use std::ffi::c_void;
    use std::sync::Arc;

    use tokio::sync::Notify;
    use windows_sys::Win32::Foundation::{HANDLE, NO_ERROR};
    use windows_sys::Win32::NetworkManagement::IpHelper::{CancelMibChangeNotify2, NotifyIpInterfaceChange, MIB_IPINTERFACE_ROW};
    use windows_sys::Win32::Networking::WinSock::AF_UNSPEC;

    /// A registration with the OS for interface changes; ending it stops the notifications.
    pub(in super::super) struct Watch {
        handle: HANDLE,
        /// The reference the callback reads through its context pointer; given back on drop.
        context: *const Notify,
    }

    // The handle and the context are only used to end the registration; the callback never sees `Watch`.
    unsafe impl Send for Watch {}

    /// Runs on a thread of the OS's: it only sets a flag for the task that waits.
    unsafe extern "system" fn on_change(context: *const c_void, _row: *const MIB_IPINTERFACE_ROW, _kind: i32) {
        // SAFETY: `context` is the `Arc<Notify>` leaked in `start`, alive until `Watch` is dropped,
        // and that cancels the registration (waiting out a running callback) before it is released.
        if let Some(signal) = unsafe { (context as *const Notify).as_ref() } {
            signal.notify_one();
        }
    }

    impl Watch {
        pub(in super::super) fn start(signal: Arc<Notify>) -> Option<Watch> {
            let context = Arc::into_raw(signal);
            let mut handle: HANDLE = std::ptr::null_mut();
            // SAFETY: the callback matches the signature, `context` outlives the registration (see
            // `on_change`), and `handle` is a valid place for the OS to write the registration to.
            let status = unsafe { NotifyIpInterfaceChange(AF_UNSPEC, Some(on_change), context.cast(), 0, &mut handle) };
            if status != NO_ERROR || handle.is_null() {
                tracing::warn!(status, "connectivity: NotifyIpInterfaceChange failed; no OS network-change notifications");
                // SAFETY: from `into_raw` above; nothing else holds the pointer.
                drop(unsafe { Arc::from_raw(context) });
                return None;
            }
            Some(Watch { handle, context })
        }
    }

    impl Drop for Watch {
        fn drop(&mut self) {
            // SAFETY: `handle` came from `NotifyIpInterfaceChange`; cancelling returns only after a
            // running callback has finished, so the context can be released after it.
            unsafe {
                CancelMibChangeNotify2(self.handle);
                drop(Arc::from_raw(self.context));
            }
        }
    }
}

#[cfg(test)]
#[path = "netchange_tests.rs"]
mod tests;
