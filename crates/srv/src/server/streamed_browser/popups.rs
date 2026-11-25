// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who owns the pages of the browser. The browser tells everyone about every new page and every page that ends. Each pane claims
//! the pages it shows, so a window opened by one of a pane's pages is told to that pane (which shows it). The rule is that no
//! page exists in the browser that no pane lists: a new page nobody claims is closed as it appears, whether or not it has an
//! opener. The pages srv makes for itself are known because srv made them, not because they lack an opener: a pane marks that it
//! is making a page before it asks for it, and claims the page the moment it has its id; the browser's own first (blank) page is
//! marked when the browser starts.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;

/// What a pane is told about its windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetEvent {
    /// A page of the pane opened a window (`target_id`); `opener_id` is the page that opened it.
    Opened { target_id: String, opener_id: String, url: String },
    /// A window of the pane ended (its page closed it, or it crashed).
    Gone { target_id: String },
}

enum Owner {
    /// A pane shows this page.
    Pane(UnboundedSender<TargetEvent>),
    /// srv made this page and no pane lists it: the browser's first, blank page, which keeps the browser from having none.
    Srv,
}

#[derive(Default)]
struct Registry {
    owners: HashMap<String, Owner>,
    /// How many pages srv is making for a pane right now (asked for, not yet claimed).
    making: usize,
    /// Pages that appeared with no opener while a page was being made: one of them is the page being made, which is claimed in a
    /// moment; whatever is left when the last one is claimed is nobody's.
    unclaimed: Vec<String>,
}

impl Registry {
    /// The pages to close now: the unclaimed ones, once no page is being made.
    fn strays(&mut self) -> Vec<String> {
        if self.making > 0 {
            return Vec::new();
        }
        std::mem::take(&mut self.unclaimed).into_iter().filter(|t| !self.owners.contains_key(t)).collect()
    }
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Registry::default()))
}

fn lock() -> std::sync::MutexGuard<'static, Registry> {
    registry().lock().unwrap_or_else(|e| e.into_inner())
}

/// This pane shows `target_id`: windows it opens, and its end, are told to `tx`.
pub fn claim(target_id: &str, tx: UnboundedSender<TargetEvent>) {
    let mut r = lock();
    r.unclaimed.retain(|t| t != target_id);
    r.owners.insert(target_id.to_string(), Owner::Pane(tx));
}

/// srv made `target_id` for itself and no pane lists it (the browser's first page): it is not closed as unowned.
pub fn claim_for_srv(target_id: &str) {
    let mut r = lock();
    r.unclaimed.retain(|t| t != target_id);
    r.owners.insert(target_id.to_string(), Owner::Srv);
}

pub fn release(target_id: &str) {
    lock().owners.remove(target_id);
}

/// The pages one pane has claimed, owned by the pane: they are all released when it is dropped, whether it ended through
/// `Pane::finish`, by a panic or because its task was dropped before `finish` ran.
#[derive(Default)]
pub struct Claims(Vec<String>);

impl Claims {
    /// The pane shows `target_id` (as [`claim`]): released with the pane at the latest.
    pub fn claim(&mut self, target_id: &str, tx: UnboundedSender<TargetEvent>) {
        claim(target_id, tx);
        if !self.0.iter().any(|t| t == target_id) {
            self.0.push(target_id.to_string());
        }
    }

    /// The pane no longer shows `target_id`.
    pub fn release(&mut self, target_id: &str) {
        release(target_id);
        self.0.retain(|t| t != target_id);
    }
}

impl Drop for Claims {
    fn drop(&mut self) {
        if self.0.is_empty() {
            return;
        }
        let mut r = lock();
        for t in self.0.drain(..) {
            r.owners.remove(&t);
        }
    }
}

/// A pane is about to ask the browser for a page. Until it says it is done (`Making::done`), a new page that has no opener is
/// held as maybe-the-one-being-made instead of being closed.
pub struct Making(());

pub fn making_a_page() -> Making {
    lock().making += 1;
    Making(())
}

impl Making {
    /// The pane has the page's id and has claimed it (or the browser made none). The pages to close: any that appeared in
    /// the meantime and nobody has claimed.
    pub fn done(self) -> Vec<String> {
        drop(self);
        lock().strays()
    }
}

impl Drop for Making {
    fn drop(&mut self) {
        let mut r = lock();
        r.making = r.making.saturating_sub(1);
    }
}

/// What to do with a browser-level event: the pages to close. A pane that owns a page is told what happened to it.
pub fn route(method: &str, params: &Value) -> Vec<String> {
    let mut r = lock();
    let mut close = r.strays();
    match method {
        "Target.targetCreated" => {
            let info = &params["targetInfo"];
            if info["type"] != "page" {
                return close;
            }
            let Some(target) = info["targetId"].as_str() else { return close };
            if r.owners.contains_key(target) {
                // srv's own page, or one a pane claimed before the browser's news of it came.
                return close;
            }
            let opener = info.get("openerId").and_then(Value::as_str).filter(|o| !o.is_empty());
            let url = info["url"].as_str().unwrap_or("").to_string();
            match opener {
                Some(opener) => match r.owners.get(opener) {
                    Some(Owner::Pane(tx)) if tx.send(TargetEvent::Opened { target_id: target.to_string(), opener_id: opener.to_string(), url }).is_ok() => {}
                    Some(Owner::Pane(_)) => {
                        // The pane that owned the opener is gone.
                        r.owners.remove(opener);
                        close.push(target.to_string());
                    }
                    _ => close.push(target.to_string()),
                },
                // No opener: either a page srv is making (claimed in a moment), or nobody's.
                None if r.making > 0 => r.unclaimed.push(target.to_string()),
                None => close.push(target.to_string()),
            }
        }
        "Target.targetDestroyed" => {
            if let Some(target) = params["targetId"].as_str() {
                r.unclaimed.retain(|t| t != target);
                if let Some(Owner::Pane(tx)) = r.owners.get(target) {
                    let _ = tx.send(TargetEvent::Gone { target_id: target.to_string() });
                }
            }
        }
        _ => {}
    }
    close
}

/// Takes a turn: the registry is the process's, so tests that use it one at a time.
#[cfg(test)]
pub(super) async fn serial() -> tokio::sync::MutexGuard<'static, ()> {
    static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    TURN.lock().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::sync::mpsc;

    fn created(target: &str, opener: Option<&str>, kind: &str) -> Value {
        let mut info = json!({ "targetId": target, "type": kind, "url": "https://idp.example/auth" });
        if let Some(o) = opener {
            info["openerId"] = json!(o);
        }
        json!({ "targetInfo": info })
    }

    fn only(closed: Vec<String>) -> Option<String> {
        assert!(closed.len() <= 1, "{closed:?}");
        closed.into_iter().next()
    }

    #[tokio::test]
    async fn a_window_a_pane_s_page_opens_is_told_to_that_pane_and_is_not_closed() {
        let _turn = serial().await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        claim("t-owned-1", tx);
        assert_eq!(route("Target.targetCreated", &created("t-pop-1", Some("t-owned-1"), "page")), Vec::<String>::new());
        assert_eq!(rx.try_recv().unwrap(), TargetEvent::Opened { target_id: "t-pop-1".into(), opener_id: "t-owned-1".into(), url: "https://idp.example/auth".into() });
        release("t-owned-1");
    }

    #[tokio::test]
    async fn a_page_nobody_claims_is_closed_whether_or_not_it_has_an_opener() {
        let _turn = serial().await;
        // An opener that no pane owns.
        assert_eq!(only(route("Target.targetCreated", &created("t-pop-2", Some("t-unowned"), "page"))).as_deref(), Some("t-pop-2"));
        // No opener at all, with nothing being made: it is nobody's.
        assert_eq!(only(route("Target.targetCreated", &created("t-new", None, "page"))).as_deref(), Some("t-new"));
        // Not a page, or not news of a page: left alone.
        assert!(route("Target.targetCreated", &created("t-w", Some("t-unowned"), "service_worker")).is_empty());
        assert!(route("Target.targetCreated", &json!({})).is_empty());
        assert!(route("Target.attachedToTarget", &json!({})).is_empty());
    }

    #[tokio::test]
    async fn a_page_srv_is_making_is_not_closed_and_one_that_appears_beside_it_is() {
        let _turn = serial().await;
        let (tx, _rx) = mpsc::unbounded_channel();
        let making = making_a_page();
        // The browser's news of the page comes before the answer that gives its id.
        assert!(route("Target.targetCreated", &created("t-own-6", None, "page")).is_empty(), "held, not closed: it may be the page being made");
        assert!(route("Target.targetCreated", &created("t-stray-6", None, "page")).is_empty());
        claim("t-own-6", tx);
        // Once the pane has its page, what is left over is nobody's.
        assert_eq!(making.done(), vec!["t-stray-6".to_string()]);
        // And the claimed page stays: its own later news does not close it.
        assert!(route("Target.targetCreated", &created("t-own-6", None, "page")).is_empty());
        release("t-own-6");
    }

    #[tokio::test]
    async fn a_page_that_no_answer_ever_claims_is_closed_when_the_making_ends() {
        let _turn = serial().await;
        let making = making_a_page();
        route("Target.targetCreated", &created("t-lost-7", None, "page"));
        // The pane never got an id (the browser refused, or the pane was dropped): nothing claims it.
        drop(making);
        assert_eq!(route("Target.targetDestroyed", &json!({ "targetId": "t-other" })), vec!["t-lost-7".to_string()], "the next news of the browser sweeps it");
    }

    #[tokio::test]
    async fn the_page_srv_keeps_for_itself_is_not_closed_and_is_not_a_panes() {
        let _turn = serial().await;
        claim_for_srv("t-blank-8");
        assert!(route("Target.targetCreated", &created("t-blank-8", None, "page")).is_empty());
        // It opens nothing, but if it did, its window would be nobody's: srv's own page is not a pane.
        assert_eq!(only(route("Target.targetCreated", &created("t-pop-8", Some("t-blank-8"), "page"))).as_deref(), Some("t-pop-8"));
        release("t-blank-8");
    }

    #[tokio::test]
    async fn a_pane_hears_only_of_its_own_windows() {
        let _turn = serial().await;
        let (a_tx, mut a_rx) = mpsc::unbounded_channel();
        let (b_tx, mut b_rx) = mpsc::unbounded_channel();
        claim("t-a-3", a_tx);
        claim("t-b-3", b_tx);
        route("Target.targetCreated", &created("t-pop-3", Some("t-b-3"), "page"));
        assert!(a_rx.try_recv().is_err());
        assert!(matches!(b_rx.try_recv(), Ok(TargetEvent::Opened { .. })));
        route("Target.targetDestroyed", &json!({ "targetId": "t-a-3" }));
        assert_eq!(a_rx.try_recv().unwrap(), TargetEvent::Gone { target_id: "t-a-3".into() });
        assert!(b_rx.try_recv().is_err());
        release("t-a-3");
        release("t-b-3");
    }

    #[tokio::test]
    async fn the_claims_of_a_pane_dropped_without_finishing_are_released() {
        let _turn = serial().await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut claims = Claims::default();
        claims.claim("t-held-6", tx.clone());
        claims.claim("t-held-7", tx);
        claims.release("t-held-7");
        // While the pane lives its page is its own: a window it opens is told to it, not closed.
        assert!(route("Target.targetCreated", &created("t-pop-6", Some("t-held-6"), "page")).is_empty());
        assert!(matches!(rx.try_recv(), Ok(TargetEvent::Opened { .. })));
        // Dropped with no finish (a panic, a task dropped): nothing of it is claimed, though its channel is still open.
        drop(claims);
        assert_eq!(only(route("Target.targetCreated", &created("t-pop-7", Some("t-held-6"), "page"))).as_deref(), Some("t-pop-7"));
        assert!(rx.try_recv().is_err(), "a page of a dropped pane is still told its news");
    }

    #[tokio::test]
    async fn a_window_whose_pane_has_gone_is_closed_and_the_claim_is_dropped() {
        let _turn = serial().await;
        let (tx, rx) = mpsc::unbounded_channel();
        claim("t-gone-4", tx);
        drop(rx);
        assert_eq!(only(route("Target.targetCreated", &created("t-pop-4", Some("t-gone-4"), "page"))).as_deref(), Some("t-pop-4"));
        assert_eq!(only(route("Target.targetCreated", &created("t-pop-5", Some("t-gone-4"), "page"))).as_deref(), Some("t-pop-5"));
    }
}
