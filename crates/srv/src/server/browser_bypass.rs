// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Bypass: the person lets the agent driving a browser pane act without
//! asking first, on the site(s) the pane is on, for a time they choose
//! (docs/specs/SPEC_BROWSER_PANE_BYPASS_2026_10_16.md).
//!
//! A grant lives only here, in memory: srv restarting ends every grant, and
//! the pane's `browser:bypass` meta key (srv-only, like `browser:attention`)
//! only mirrors it for the header strip. Only the person grants one: through
//! the host (`/api/v1/host/browser_bypass`, the Bypass button) or by answering
//! an agent's `request_bypass` banner, which also arrives only through the
//! host. An agent may end its own grant, never start one.
//!
//! A grant is for one pane (and the popup windows its page opens; a popup
//! pane has its own header and its own grant), one agent (the owner
//! when it was granted), the sites the person was shown, and a time limit.
//! Take over, closing the pane, a change of owner and the time limit end it,
//! and so do [`IDLE_LIMIT`] minutes without a `Browser` call from its agent
//! on the pane: a hung or stopped agent doesn't keep one alive.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::server::AppState;

/// Block meta key the browser pane's header reads.
pub(crate) const BYPASS_META_KEY: &str = "browser:bypass";

/// The durations the person can pick, in minutes. 60 is the default.
pub(crate) const CHOICES_MINUTES: [u64; 3] = [15, 60, 240];
pub(crate) const DEFAULT_MINUTES: u64 = 60;

/// After the person says No to an agent's request, the agent may not ask
/// again on that pane for this long.
pub(crate) const REQUEST_COOLDOWN: Duration = Duration::from_secs(10 * 60);

/// A grant ends after this long without a `Browser` call from its agent on
/// the pane ([`touch`]), with [`IDLE_REASON`]. Time the pane spends waiting
/// on the person (a hand-off, a question) doesn't count: the agent is waiting
/// for them then, not stuck.
pub(crate) const IDLE_LIMIT: Duration = Duration::from_secs(15 * 60);
pub(crate) const IDLE_REASON: &str = "no activity for 15 minutes";
const TIME_UP_REASON: &str = "its time ran out";

#[derive(Debug, Clone)]
pub(crate) struct Grant {
    /// Distinguishes this grant from a later one on the same pane, so an old
    /// expiry timer can't end a new grant.
    pub id: String,
    pub agent: String,
    /// The bound sites, in `allowed_origins` normal form.
    pub origins: Vec<String>,
    pub minutes: u64,
    pub expires: Instant,
    /// For the header's countdown: milliseconds since the Unix epoch.
    pub expires_at_ms: i64,
    /// The agent's last `Browser` call on the pane, or the grant itself.
    last_active: Instant,
}

impl Grant {
    fn live(&self) -> bool {
        Instant::now() < self.expires
    }

    pub(crate) fn minutes_left(&self) -> u64 {
        self.expires.saturating_duration_since(Instant::now()).as_secs().div_ceil(60)
    }

    /// Is `url` on the sites this grant is bound to? A page with no web
    /// origin (`about:blank`, `data:`) never is.
    pub(crate) fn covers(&self, url: &str) -> bool {
        agentmux_common::allowed_origins::target_origin(url).is_some()
            && agentmux_common::allowed_origins::allows(&self.origins, url)
    }
}

/// A grant, and the srv state it was made with: [`forget`]'s callers have
/// none of their own, and it still takes the header strip down through it.
struct Live {
    grant: Grant,
    state: Option<AppState>,
}

#[derive(Default)]
struct Table {
    grants: HashMap<String, Live>,
    declined: HashMap<String, Instant>,
}

fn table() -> std::sync::MutexGuard<'static, Table> {
    static T: OnceLock<Mutex<Table>> = OnceLock::new();
    T.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner())
}

/// Round `asked` to one of [`CHOICES_MINUTES`]: the nearest one not longer
/// than asked, and never past the longest. None gives the default.
pub(crate) fn clamp_minutes(asked: Option<u64>) -> u64 {
    let Some(asked) = asked else { return DEFAULT_MINUTES };
    CHOICES_MINUTES
        .iter()
        .rev()
        .find(|m| **m <= asked)
        .copied()
        .unwrap_or(CHOICES_MINUTES[0])
}

/// The live grant on `pane` for `agent`, if any. An expired one is dropped
/// here, without waiting for its timer.
pub(crate) fn active_for(pane: &str, agent: &str) -> Option<Grant> {
    let mut t = table();
    let g = t.grants.get(pane)?.grant.clone();
    if !g.live() {
        t.grants.remove(pane);
        return None;
    }
    g.agent.eq_ignore_ascii_case(agent).then_some(g)
}

/// Did the person decline a request on `pane` too recently for another?
pub(crate) fn cooling_down(pane: &str) -> Option<Duration> {
    let mut t = table();
    let at = *t.declined.get(pane)?;
    let left = REQUEST_COOLDOWN.checked_sub(at.elapsed());
    if left.is_none() {
        t.declined.remove(pane);
    }
    left
}

pub(crate) fn note_declined(pane: &str) {
    table().declined.insert(pane.to_string(), Instant::now());
}

/// Record a grant on `pane` for `agent` over `origins`, show it in the
/// pane's header, and end it when its time is up or its agent goes quiet.
/// Replaces any grant the pane had.
pub(crate) fn grant(state: &AppState, pane: &str, agent: &str, origins: Vec<String>, minutes: u64) -> Grant {
    grant_watched(state, pane, agent, origins, minutes, IDLE_LIMIT)
}

fn grant_watched(state: &AppState, pane: &str, agent: &str, origins: Vec<String>, minutes: u64, idle: Duration) -> Grant {
    let minutes = clamp_minutes(Some(minutes));
    let dur = Duration::from_secs(minutes * 60);
    let now = Instant::now();
    let g = Grant {
        id: uuid::Uuid::new_v4().to_string(),
        agent: agent.to_string(),
        origins,
        minutes,
        expires: now + dur,
        expires_at_ms: chrono::Utc::now().timestamp_millis() + dur.as_millis() as i64,
        last_active: now,
    };
    {
        let mut t = table();
        t.declined.remove(pane);
        t.grants.insert(pane.to_string(), Live { grant: g.clone(), state: Some(state.clone()) });
    }
    tracing::info!(
        pane = %pane, agent = %agent, origins = ?g.origins, minutes,
        "[browser-bypass] granted: the agent acts without asking"
    );
    publish(state, pane, strip_value(&g));
    tokio::spawn(watch(state.clone(), pane.to_string(), g.id.clone(), idle));
    g
}

/// A `Browser` call from `agent` on `target` (a pane, or a popup window,
/// whose grant is its opener's): it keeps that agent's grant there alive.
/// Every agent browser call resolves its pane through
/// `ui_handlers::target_block_id`, which calls this.
pub(crate) fn touch(target: &str, agent: &str) {
    let pane = crate::server::ui_handlers::banner_for(target).0;
    if let Some(l) = table().grants.get_mut(&pane).filter(|l| l.grant.agent.eq_ignore_ascii_case(agent)) {
        l.grant.last_active = Instant::now();
    }
}

/// What the watcher of grant `id` on `pane` does next.
#[derive(Debug, PartialEq)]
enum Due {
    /// The grant has ended or been replaced: stop watching.
    Gone,
    End(&'static str),
    /// Nothing yet: look again then.
    At(Instant),
}

/// Grant `id` on `pane` at `now`: past its time, or `idle` since its agent's
/// last `Browser` call there. `busy`: the pane waits on the person, which
/// counts as activity.
fn due(pane: &str, id: &str, idle: Duration, now: Instant, busy: bool) -> Due {
    let mut t = table();
    let Some(g) = t.grants.get_mut(pane).map(|l| &mut l.grant).filter(|g| g.id == id) else {
        return Due::Gone;
    };
    if now >= g.expires {
        return Due::End(TIME_UP_REASON);
    }
    if busy {
        g.last_active = now;
    }
    let idle_at = g.last_active + idle;
    if now >= idle_at {
        return Due::End(IDLE_REASON);
    }
    Due::At(idle_at.min(g.expires))
}

/// One task per grant ends it on time or when its agent goes quiet. A
/// [`touch`] only moves the deadline: the task wakes at the old one, finds
/// the grant still in use, and sleeps until the new one.
async fn watch(state: AppState, pane: String, id: String, idle: Duration) {
    loop {
        let busy = crate::server::browser_attention::waiting_on_user(&pane).is_some();
        match due(&pane, &id, idle, Instant::now(), busy) {
            Due::Gone => return,
            Due::End(reason) => {
                end_if(&state, &pane, Some(&id), reason, true);
                return;
            }
            Due::At(at) => tokio::time::sleep_until(tokio::time::Instant::from_std(at)).await,
        }
    }
}

/// End `pane`'s grant, if it has one, and take the header strip down; with
/// `say_why`, the strip says why it ended instead (the person who ended it
/// themselves needs no note). Returns whether there was one.
pub(crate) fn end(state: &AppState, pane: &str, reason: &str, say_why: bool) -> bool {
    end_if(state, pane, None, reason, say_why)
}

/// End `pane`'s grant for callers that have no [`AppState`]: Take over,
/// closing the pane, a new owner. The strip is cleared through the state the
/// grant was made with, so it can't go on showing a grant that has ended.
pub(crate) fn forget(pane: &str, reason: &str) {
    let Some(live) = table().grants.remove(pane) else { return };
    tracing::info!(pane = %pane, reason = %reason, "[browser-bypass] ended");
    if let Some(state) = live.state {
        publish(&state, pane, Value::Null);
    }
}

fn end_if(state: &AppState, pane: &str, id: Option<&str>, reason: &str, say_why: bool) -> bool {
    let removed = {
        let mut t = table();
        match t.grants.get(pane) {
            Some(l) if id.is_none_or(|id| l.grant.id == id) => t.grants.remove(pane).map(|l| l.grant),
            _ => None,
        }
    };
    let Some(g) = removed else { return false };
    tracing::info!(pane = %pane, reason = %reason, "[browser-bypass] ended");
    // The note has no `expires_at`, so the header never takes it for a
    // grant; the next grant, or the host registering, replaces it.
    publish(state, pane, if say_why { json!({ "agent": g.agent, "ended": reason }) } else { Value::Null });
    true
}

/// The header strip's copy of a live grant.
fn strip_value(g: &Grant) -> Value {
    json!({ "agent": g.agent, "origins": g.origins, "minutes": g.minutes, "expires_at": g.expires_at_ms })
}

/// Write `value` as the header strip's `browser:bypass`.
fn publish(state: &AppState, pane: &str, value: Value) {
    let mut meta = crate::backend::obj::MetaMapType::new();
    meta.insert(BYPASS_META_KEY.to_string(), value);
    if let Err(e) = crate::server::http_shell::broadcast_meta_update(state, pane, &meta) {
        tracing::debug!(pane = %pane, error = %e, "[browser-bypass] header strip not updated");
    }
}

/// Clear every pane's strip. Called when the host registers: a grant never
/// survives srv restarting, and the strip, saved with the pane, must not
/// keep claiming one.
pub(crate) fn reset_strips(state: &AppState) {
    let live: Vec<String> = table().grants.keys().cloned().collect();
    let blocks = match state.mstore.get_all::<crate::backend::obj::Block>() {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "[browser-bypass] couldn't list panes to clear stale bypass strips");
            return;
        }
    };
    for b in blocks
        .into_iter()
        .filter(|b| !live.contains(&b.oid) && b.meta.get(BYPASS_META_KEY).is_some_and(|v| !v.is_null()))
    {
        publish(state, &b.oid, Value::Null);
    }
}

/// Does the committing click `what` (the approval's "Click \"…\"") spend
/// or move money? Those still ask under bypass (spec §11.2): the cost of a
/// mistake is money, and bulk jobs rarely pay. Matched by word stem
/// ("pay" covers payment, payments, paying), so it errs towards asking.
pub(crate) fn is_payment(what: &str) -> bool {
    let w = what.to_ascii_lowercase();
    let words: Vec<&str> = w.split(|c: char| !c.is_ascii_alphanumeric()).filter(|s| !s.is_empty()).collect();
    // A word starting with one of these: pay, payment, purchase, checkout,
    // transfer, donation, subscription, withdrawal, remittance, deposit, refund.
    const STEMS: [&str; 11] =
        ["pay", "purchas", "checkout", "transfer", "donat", "subscri", "withdraw", "remit", "deposit", "refund", "billing"];
    // Words a stem would get wrong ("payload"), or that only count whole.
    const NOT: [&str; 2] = ["payload", "payloads"];
    const WHOLE: [&str; 8] = ["buy", "buys", "order", "orders", "paid", "wire", "bid", "tip"];
    const PAIRS: [[&str; 2]; 6] =
        [["check", "out"], ["place", "order"], ["send", "money"], ["send", "funds"], ["add", "funds"], ["add", "money"]];
    words.iter().any(|x| !NOT.contains(x) && (STEMS.iter().any(|s| x.starts_with(s)) || WHOLE.contains(x)))
        || words.windows(2).any(|p| PAIRS.iter().any(|q| p == q))
}

/// The sites a grant on `pane` binds to: the origin of the page it shows
/// (`url`, as the host reports it), and its site list if it has one.
pub(crate) fn origins_for(pane: &str, url: &str) -> Result<Vec<String>, String> {
    let origin = agentmux_common::allowed_origins::target_origin(url)
        .ok_or_else(|| "the pane isn't showing a web page; open the site first".to_string())?;
    let mut out = vec![origin];
    for o in crate::server::browser_allowlist::list_for(pane).unwrap_or_default() {
        if !out.contains(&o) {
            out.push(o);
        }
    }
    Ok(out)
}

/// A live grant on `pane` for `agent` over github.com, for other modules'
/// tests (no `AppState` needed).
#[cfg(test)]
pub(crate) fn tests_put(pane: &str, agent: &str) {
    tests::put(pane, agent, &["https://github.com"], 60);
}

/// A real grant (strip and watcher) that ends after `idle` without a
/// `Browser` call, for other modules' tests.
#[cfg(test)]
pub(crate) fn tests_grant_idle(state: &AppState, pane: &str, agent: &str, idle: Duration) -> Grant {
    grant_watched(state, pane, agent, vec!["https://github.com".to_string()], 15, idle)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn put(pane: &str, agent: &str, origins: &[&str], secs: u64) {
        let grant = Grant {
            id: format!("{pane}-id"),
            agent: agent.to_string(),
            origins: origins.iter().map(|s| s.to_string()).collect(),
            minutes: 1,
            expires: Instant::now() + Duration::from_secs(secs),
            expires_at_ms: 0,
            last_active: Instant::now(),
        };
        table().grants.insert(pane.to_string(), Live { grant, state: None });
    }

    fn quiet_for(pane: &str, d: Duration) {
        table().grants.get_mut(pane).unwrap().grant.last_active = Instant::now() - d;
    }

    #[test]
    fn a_grant_ends_after_its_agent_goes_quiet() {
        let idle = Duration::from_secs(15 * 60);
        put("bp-idle", "naki", &["https://github.com"], 3600);
        let id = "bp-idle-id";
        // Fresh: look again when the idle limit would be reached.
        let Due::At(at) = due("bp-idle", id, idle, Instant::now(), false) else { panic!("not due yet") };
        assert!(at > Instant::now() + Duration::from_secs(14 * 60));
        quiet_for("bp-idle", idle + Duration::from_secs(1));
        assert_eq!(due("bp-idle", id, idle, Instant::now(), false), Due::End(IDLE_REASON));
        // Another grant's watcher, or one for a replaced grant, stops.
        assert_eq!(due("bp-idle", "an-older-grant", idle, Instant::now(), false), Due::Gone);
        assert_eq!(due("bp-none", id, idle, Instant::now(), false), Due::Gone);
        forget("bp-idle", "test");
    }

    #[test]
    fn a_browser_call_from_its_agent_keeps_a_grant_alive() {
        let idle = Duration::from_secs(15 * 60);
        put("bp-touch", "naki", &["https://github.com"], 3600);
        quiet_for("bp-touch", idle + Duration::from_secs(1));
        // Another agent's call doesn't count.
        touch("bp-touch", "korp");
        assert_eq!(due("bp-touch", "bp-touch-id", idle, Instant::now(), false), Due::End(IDLE_REASON));
        touch("bp-touch", "Naki");
        assert!(matches!(due("bp-touch", "bp-touch-id", idle, Instant::now(), false), Due::At(_)));
        forget("bp-touch", "test");
    }

    #[test]
    fn waiting_on_the_person_is_not_going_quiet() {
        let idle = Duration::from_secs(15 * 60);
        put("bp-busy", "naki", &["https://github.com"], 3600);
        quiet_for("bp-busy", idle + Duration::from_secs(60));
        // A hand-off is up: the agent waits for the person, it isn't stuck.
        assert!(matches!(due("bp-busy", "bp-busy-id", idle, Instant::now(), true), Due::At(_)));
        // And the wait counts as activity once it's over.
        assert!(matches!(due("bp-busy", "bp-busy-id", idle, Instant::now(), false), Due::At(_)));
        forget("bp-busy", "test");
    }

    #[test]
    fn the_time_limit_ends_a_grant_however_busy() {
        put("bp-time", "naki", &["https://github.com"], 0);
        assert_eq!(due("bp-time", "bp-time-id", Duration::from_secs(900), Instant::now(), true), Due::End(TIME_UP_REASON));
        forget("bp-time", "test");
    }

    #[test]
    fn a_grant_is_for_its_agent_only() {
        put("bp-agent", "naki", &["https://github.com"], 60);
        assert!(active_for("bp-agent", "Naki").is_some());
        assert!(active_for("bp-agent", "korp").is_none());
        forget("bp-agent", "test");
        assert!(active_for("bp-agent", "naki").is_none());
    }

    #[test]
    fn an_expired_grant_is_gone_without_its_timer() {
        put("bp-expired", "naki", &["https://github.com"], 0);
        assert!(active_for("bp-expired", "naki").is_none());
        assert!(!table().grants.contains_key("bp-expired"), "dropped when found expired");
    }

    #[test]
    fn a_grant_covers_only_its_sites() {
        put("bp-sites", "naki", &["https://github.com", "https://*.example.com"], 60);
        let g = active_for("bp-sites", "naki").unwrap();
        assert!(g.covers("https://github.com/example/repo/issues/1"));
        assert!(g.covers("https://docs.example.com/x"));
        assert!(!g.covers("https://accounts.google.com/o/oauth2"));
        assert!(!g.covers("http://github.com/"), "another scheme is another site");
        assert!(!g.covers("about:blank"), "a page with no origin is never covered");
        assert!(!g.covers("data:text/html,hi"));
        forget("bp-sites", "test");
    }

    #[test]
    fn durations_round_down_to_a_choice_and_cap() {
        assert_eq!(clamp_minutes(None), 60);
        assert_eq!(clamp_minutes(Some(15)), 15);
        assert_eq!(clamp_minutes(Some(30)), 15);
        assert_eq!(clamp_minutes(Some(60)), 60);
        assert_eq!(clamp_minutes(Some(90)), 60);
        assert_eq!(clamp_minutes(Some(10_000)), 240);
        assert_eq!(clamp_minutes(Some(1)), 15);
    }

    #[test]
    fn payments_are_recognised() {
        for what in [
            "Click \"Pay now\"",
            "Click \"Buy\"",
            "Click \"Place order\"",
            "Click \"Check out\"",
            "Click \"Checkout\"",
            "Click \"Transfer funds\"",
            "Click \"Confirm payment\"",
            "Click \"Make a payment\"",
            "Click \"Submit payment\"",
            "Click \"Send money\"",
            "Click \"Withdraw\"",
            "Click \"Wire funds\"",
            "Click \"Purchase subscription\"",
            "Click \"Donate\"",
        ] {
            assert!(is_payment(what), "{what}");
        }
        for what in [
            "Click \"Delete draft\"",
            "Click \"Submit\"",
            "Click \"Send\"",
            "Click \"Send message\"",
            "Click \"Reorder list\"",
            "Click \"Payload settings\"",
            "Click \"Delete revision\"",
            "Click \"Confirm\"",
        ] {
            assert!(!is_payment(what), "{what}");
        }
    }

    #[test]
    fn a_no_cools_down() {
        assert!(cooling_down("bp-cool").is_none());
        note_declined("bp-cool");
        assert!(cooling_down("bp-cool").is_some());
        table().declined.insert("bp-cool".into(), Instant::now() - REQUEST_COOLDOWN - Duration::from_secs(1));
        assert!(cooling_down("bp-cool").is_none());
    }
}
