//! Tells every open window that an account changed, from the code that changes it.
//!
//! The Connectors pane, the pickers and the agent pane keep one cache of the accounts and refresh
//! it on `identityaccounts:changed`. A path that writes an account row without publishing it leaves
//! them stale until a remount, which is what an agent's Google connect did: the flow commits the
//! account in the background, with no request of the person's window to refresh after. So the
//! commit points (a connect finishing, a token refresh the provider refuses) announce it here, and a
//! new path has one call to make.

use std::sync::{Arc, OnceLock};

use crate::backend::mps::Broker;
use crate::backend::mps_events::{Event, GoogleNotice, IdentityAccountsChanged};

static BROKER: OnceLock<Arc<Broker>> = OnceLock::new();

/// Where the announcement goes, set once at startup. Until then (and in a unit test that doesn't
/// install one) a change is recorded but not announced.
pub fn install_broker(broker: Arc<Broker>) {
    let _ = BROKER.set(broker);
}

/// An account was added, removed, reconnected or changed state: windows read the accounts again.
pub fn announce() {
    if let Some(broker) = BROKER.get() {
        broker.publish_event::<IdentityAccountsChanged>(&(), vec![], 0);
    }
}

/// The `google:notice` event: one line for the windows' notices, asking nothing ("Naki-2 gave
/// Agent5 Calendar read"). Never mail, event or file content.
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct GoogleNoticeEvent {
    pub text: String,
}

/// Tells the windows what an agent did to a Google account, so the person sees it as it happens.
pub fn tell_windows(text: String) {
    if let Some(broker) = BROKER.get() {
        broker.publish_event::<GoogleNotice>(&GoogleNoticeEvent { text }, vec![], 0);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use super::*;

    /// How many times `identityaccounts:changed` has been published on the process's test broker.
    /// Tests run side by side, so a test asserts it went up, not by how much.
    pub(crate) fn announcements() -> usize {
        counter().load(Ordering::SeqCst)
    }

    /// Installs the process's test broker (once) and counts what it publishes.
    pub(crate) fn watch() {
        counter();
    }

    /// The `google:notice` payloads the test broker has seen.
    fn notices() -> &'static Mutex<Vec<String>> {
        static SEEN: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
        SEEN.get_or_init(Default::default)
    }

    fn counter() -> &'static AtomicUsize {
        static COUNT: OnceLock<Arc<AtomicUsize>> = OnceLock::new();
        COUNT
            .get_or_init(|| {
                let count = Arc::new(AtomicUsize::new(0));
                let seen = Arc::clone(&count);
                let broker = Arc::new(Broker::new());
                broker.add_observer(Arc::new(move |e| {
                    if e.event == IdentityAccountsChanged::NAME {
                        seen.fetch_add(1, Ordering::SeqCst);
                    }
                    if e.event == GoogleNotice::NAME {
                        notices().lock().unwrap().push(format!("{:?}", e.data));
                    }
                }));
                install_broker(broker);
                count
            })
            .as_ref()
    }

    #[test]
    fn telling_the_windows_publishes_one_line_for_their_notices() {
        watch();
        tell_windows("Naki-2 gave Agent5 Calendar read on me@example.com".to_string());
        assert!(notices().lock().unwrap().iter().any(|n| n.contains("Naki-2 gave Agent5 Calendar read")));
    }

    #[test]
    fn announcing_publishes_identityaccounts_changed() {
        watch();
        let before = announcements();
        announce();
        assert!(announcements() > before, "announce() publishes the event every window refreshes on");
    }
}
