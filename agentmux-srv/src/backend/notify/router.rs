// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Notification Router — the side-effecting shell around `policy.rs`
//! (`SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md` §3, §5).
//!
//! Owns: settings resolution, agent-name lookup + sanitization, body
//! redaction (§9), publishing on the mps broker, a 500 ms tick task for
//! debounce release, and the single pending "activate on next window" slot
//! for a click that arrived while no window was open.
//!
//! One Router per broker (i.e. per `AppState`), kept in a small registry
//! keyed by the broker's identity rather than as an `AppState` field: it is
//! created lazily by the first WS connection and outlives any one connection.
//! Keying by broker (not a single global) means a second `AppState` in the
//! same process — tests, or any future multi-state setup — never has its
//! events published onto someone else's bus.

use std::sync::{Arc, Mutex, OnceLock};

use crate::backend::mps::{Broker, MuxEvent};
use crate::backend::obj::Block;
use crate::backend::storage::store::Store;
use crate::backend::wconfig::ConfigState;

use super::policy::{
    Action, Family, FocusReport, Input, NotifyKind, PolicyState, Preview, Request, Settings, TrayState, When,
};

pub const EVENT_NOTIFICATION: &str = "notification";
pub const EVENT_NOTIFICATION_RETRACT: &str = "notification:retract";
pub const EVENT_NOTIFICATION_ACTIVATE: &str = "notification:activate";
/// Tray snapshot (`TrayState`), published on change with persist=1 so a
/// presenter that (re)subscribes gets the current state immediately.
pub const EVENT_NOTIFICATION_STATE: &str = "notification:state";

/// srv-internal sources, fed from places that must not block (the broker's
/// publish path, the jekt delivery path) and drained by the router's task.
enum Internal {
    /// `own_blocks_only`: the source is process-global (the reactive handler
    /// is shared by every AppState), so drop blocks this Router's store
    /// doesn't know.
    Emit { kind: NotifyKind, block_id: String, body: Option<String>, own_blocks_only: bool },
}

const AGENT_NAME_MAX: usize = 32;
const BODY_MAX: usize = 80;

pub struct Router {
    policy: Mutex<PolicyState>,
    broker: Arc<Broker>,
    config: Arc<ConfigState>,
    store: Arc<Store>,
    /// Block to activate once a window connects (click while no window open).
    pending_activation: Mutex<Option<String>>,
    /// block_id → sanitized agent name. Names change rarely; caching keeps
    /// the store (one process-wide SQLite mutex) off the hot path.
    names: Mutex<std::collections::HashMap<String, String>>,
    internal: tokio::sync::mpsc::UnboundedSender<Internal>,
    last_tray: Mutex<Option<TrayState>>,
}

type Registry = Mutex<std::collections::HashMap<usize, Arc<Router>>>;
static ROUTERS: OnceLock<Registry> = OnceLock::new();

fn registry() -> &'static Registry {
    ROUTERS.get_or_init(Default::default)
}

fn key(broker: &Arc<Broker>) -> usize {
    Arc::as_ptr(broker) as usize
}

/// The router for this broker, if a connection has created it yet.
pub fn get(broker: &Arc<Broker>) -> Option<Arc<Router>> {
    registry().lock().unwrap_or_else(|e| e.into_inner()).get(&key(broker)).cloned()
}

/// Get-or-create the router for this broker. Idempotent.
///
/// On creation it also attaches the srv-side sources: an observer on the
/// broker (`agentfailure` → `AgentCrashed`) and the reactive handler's
/// needs-review hook (`ESCALATE=required` jekt → `MessageNeedsReview`).
pub fn init(
    broker: Arc<Broker>,
    config: Arc<ConfigState>,
    store: Arc<Store>,
    reactive: &'static crate::backend::reactive::ReactiveHandler,
) -> Arc<Router> {
    let mut reg = registry().lock().unwrap_or_else(|e| e.into_inner());
    reg.entry(key(&broker))
        .or_insert_with(|| {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            let r = Arc::new(Router {
                policy: Mutex::new(PolicyState::new()),
                broker: broker.clone(),
                config,
                store,
                pending_activation: Mutex::new(None),
                names: Mutex::new(Default::default()),
                internal: tx,
                last_tray: Mutex::new(None),
            });
            spawn_ticker(Arc::downgrade(&r));
            spawn_internal(Arc::downgrade(&r), rx);
            attach_sources(&r, &broker, reactive);
            r
        })
        .clone()
}

/// Both hooks run on hot, lock-holding paths (the broker's publish; jekt
/// delivery). They only parse and enqueue — never call back into the broker
/// or touch the store inline.
fn attach_sources(r: &Arc<Router>, broker: &Arc<Broker>, reactive: &'static crate::backend::reactive::ReactiveHandler) {
    let tx = r.internal.clone();
    broker.add_observer(Arc::new(move |ev: &MuxEvent| {
        if ev.event != crate::backend::mps::EVENT_AGENT_FAILURE {
            return;
        }
        let Some(block_id) = ev.scopes.iter().find_map(|s| s.strip_prefix("block:")) else { return };
        let code = ev.data.as_ref().and_then(|d| d.get("code")).and_then(|c| c.as_str()).unwrap_or("");
        if let Some(body) = failure_body(code) {
            let _ = tx.send(Internal::Emit {
                kind: NotifyKind::AgentCrashed,
                block_id: block_id.to_string(),
                body: Some(body.to_string()),
                // Per-broker observer: already scoped to this AppState.
                own_blocks_only: false,
            });
        }
    }));
    let tx = r.internal.clone();
    reactive.add_needs_review_hook(Arc::new(move |block_id: &str| {
        let _ = tx.send(Internal::Emit {
            kind: NotifyKind::MessageNeedsReview,
            block_id: block_id.to_string(),
            // Fixed text only — never the message, sender or trust label
            // (spec §9.1: the toast can only say "go look").
            body: Some("Open AgentMux to see the sender and trust level before it acts.".to_string()),
            own_blocks_only: true,
        });
    }));
}

fn spawn_internal(r: std::sync::Weak<Router>, mut rx: tokio::sync::mpsc::UnboundedReceiver<Internal>) {
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let Some(r) = r.upgrade() else { return };
            match msg {
                Internal::Emit { kind, block_id, body, own_blocks_only } => {
                    // Store lookups — off the async workers.
                    let _ = tokio::task::spawn_blocking(move || {
                        if own_blocks_only && !r.owns_block(&block_id) {
                            return;
                        }
                        r.emit_fixed(kind, &block_id, body)
                    })
                    .await;
                }
            }
        }
    });
}

/// `AgentFailure.code` → the fixed body for its toast; `None` = don't notify.
/// Only `agent_deleted` (a user action) is skipped. `killed` is NOT a user
/// Stop — those go through the controllers' kill arm and are never
/// classified (host_spawn.rs) — it means a signal/137, most often the OOM
/// killer: exactly the "stopped while you were away" case (ReAgent P1 on
/// #3654). Detail/stderr never reach a toast.
pub fn failure_body(code: &str) -> Option<&'static str> {
    Some(match code {
        "rate_limited" => "Hit a rate limit.",
        "overloaded" => "The model provider is overloaded.",
        "usage_limit" => "Reached its usage limit.",
        "auth" => "Needs you to sign in again.",
        "context_exceeded" => "Ran out of context.",
        "max_turns" => "Reached its turn limit.",
        "network" => "Lost its network connection.",
        "spawn_failure" => "Couldn't start.",
        "no_output" | "unknown_non_zero" => "Exited unexpectedly.",
        "killed" => "Was stopped by the system (possibly out of memory).",
        _ => return None,
    })
}

fn spawn_ticker(r: std::sync::Weak<Router>) {
    // Only when a Tokio runtime exists (always true in srv; unit tests of the
    // pure policy never reach here).
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    tokio::spawn(async move {
        let mut iv = tokio::time::interval(std::time::Duration::from_millis(500));
        loop {
            iv.tick().await;
            match r.upgrade() {
                Some(r) => r.step(Input::Tick),
                None => return,
            }
        }
    });
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Strip control + bidi-override characters and cap length — the only
/// user-influenced text that reaches a title (§9.1).
pub fn sanitize_name(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| !c.is_control() && !is_bidi_control(*c))
        .collect();
    let trimmed = cleaned.trim();
    let mut out: String = trimmed.chars().take(AGENT_NAME_MAX).collect();
    if out.is_empty() {
        out = "An agent".to_string();
    }
    out
}

fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{200E}' | '\u{200F}' | '\u{061C}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// §9.2 body policy: first line, sanitized, truncated, and replaced wholesale
/// when it trips the same credential/destructive keyword list the jekt tier
/// gate uses.
pub fn redact_body(raw: &str, preview: Preview) -> Option<String> {
    if preview == Preview::None {
        return None;
    }
    let max = if preview == Preview::Full { 200 } else { BODY_MAX };
    let first = raw.lines().map(str::trim).find(|l| !l.is_empty())?;
    if crate::backend::reactive::sanitize::is_sensitive_message(raw) {
        return Some("Contains sensitive content — open AgentMux to view.".to_string());
    }
    let cleaned: String = first.chars().filter(|c| !c.is_control() && !is_bidi_control(*c)).collect();
    let mut out: String = cleaned.chars().take(max).collect();
    if cleaned.chars().count() > max {
        out.push('…');
    }
    Some(out)
}

fn setting_bool(extra: &std::collections::HashMap<String, serde_json::Value>, key: &str, default: bool) -> bool {
    extra.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

fn setting_str<'a>(extra: &'a std::collections::HashMap<String, serde_json::Value>, key: &str) -> Option<&'a str> {
    extra.get(key).and_then(|v| v.as_str())
}

/// Resolve the policy's settings view from `settings.json` (§8).
pub fn settings_from_extra(extra: &std::collections::HashMap<String, serde_json::Value>) -> Settings {
    let mut s = Settings {
        enabled: setting_bool(extra, "notify:os:enabled", true),
        when: match setting_str(extra, "notify:os:when") {
            Some("always") => When::Always,
            Some("never") => When::Never,
            _ => When::Unfocused,
        },
        preview: match setting_str(extra, "notify:os:preview") {
            Some("full") => Preview::Full,
            Some("none") => Preview::None,
            _ => Preview::Redacted,
        },
        kind_enabled: Default::default(),
        pause_until_ms: extra.get("notify:pause:until").and_then(|v| v.as_i64()).unwrap_or(0),
        pause_allow_attention: setting_bool(extra, "notify:pause:allowattention", false),
    };
    for kind in [
        NotifyKind::InputWaiting,
        NotifyKind::TurnCompleted,
        NotifyKind::TurnErrored,
        NotifyKind::AgentCrashed,
        NotifyKind::MessageNeedsReview,
    ] {
        if let Some(sfx) = kind.setting_suffix() {
            s.kind_enabled.insert(kind, setting_bool(extra, &format!("notify:os:{sfx}"), true));
        }
    }
    s
}

impl Router {
    fn settings(&self) -> Settings {
        settings_from_extra(&self.config.get_settings().extra)
    }

    /// Does this Router's store know the block? BLOCKING (SQLite).
    fn owns_block(&self, block_id: &str) -> bool {
        matches!(self.store.get::<Block>(block_id), Ok(Some(_)))
    }

    /// BLOCKING on a cache miss (SQLite behind `Store`'s process-wide mutex).
    /// Only ever reached through `emit`, which callers must run off the async
    /// workers — `notify_handlers.rs` uses `spawn_blocking` (the #1782 failure
    /// class; see websocket.rs's controllerinput note).
    fn agent_name(&self, block_id: &str) -> String {
        if let Some(n) = self.names.lock().unwrap_or_else(|e| e.into_inner()).get(block_id) {
            return n.clone();
        }
        let raw = self
            .store
            .get::<Block>(block_id)
            .ok()
            .flatten()
            .map(|b| {
                let name = crate::backend::obj::meta_get_string(&b.meta, "agentName", "");
                if name.is_empty() {
                    crate::backend::obj::meta_get_string(&b.meta, "agentId", "")
                } else {
                    name
                }
            })
            .unwrap_or_default();
        let name = sanitize_name(&raw);
        let mut cache = self.names.lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() > 1024 {
            cache.clear();
        }
        cache.insert(block_id.to_string(), name.clone());
        name
    }

    fn step(&self, input: Input) {
        let settings = self.settings();
        let now = now_ms();
        let (actions, tray) = {
            let mut p = self.policy.lock().unwrap_or_else(|e| e.into_inner());
            let actions = p.step(input, now, &settings);
            (actions, p.tray_state(&settings, now))
        };
        for a in actions {
            self.publish(a);
        }
        self.publish_tray_if_changed(tray);
    }

    fn publish_tray_if_changed(&self, tray: TrayState) {
        {
            let mut last = self.last_tray.lock().unwrap_or_else(|e| e.into_inner());
            if last.as_ref() == Some(&tray) {
                return;
            }
            *last = Some(tray.clone());
        }
        self.broker.publish(MuxEvent {
            event: EVENT_NOTIFICATION_STATE.to_string(),
            scopes: vec![],
            sender: String::new(),
            persist: 1,
            data: serde_json::to_value(&tray).ok(),
        });
    }

    fn publish(&self, action: Action) {
        let (event, data) = match action {
            Action::Show(n) => (EVENT_NOTIFICATION, serde_json::to_value(&n).ok()),
            Action::Retract { id, tag } => (EVENT_NOTIFICATION_RETRACT, Some(serde_json::json!({ "id": id, "tag": tag }))),
            Action::Activate { id, block_id } => {
                if !self.has_window() {
                    *self.pending_activation.lock().unwrap_or_else(|e| e.into_inner()) = Some(block_id.clone());
                }
                (
                    EVENT_NOTIFICATION_ACTIVATE,
                    Some(serde_json::json!({ "id": id, "block_id": block_id, "at_ms": now_ms() })),
                )
            }
        };
        tracing::info!(event, "notify: publish");
        self.broker.publish(MuxEvent {
            event: event.to_string(),
            scopes: vec![],
            sender: String::new(),
            persist: 0,
            data,
        });
    }

    // ── Source entry points ───────────────────────────────────────────────

    /// A frontend reports a pane event. `question` is only used for
    /// `InputWaiting` and goes through `redact_body`.
    ///
    /// BLOCKING (agent-name lookup may hit the store) — call from
    /// `spawn_blocking`, never inline on an async worker.
    pub fn emit(&self, kind: NotifyKind, block_id: &str, question: Option<&str>) {
        let preview = self.settings().preview;
        let body = match kind {
            NotifyKind::InputWaiting => question.and_then(|q| redact_body(q, preview)),
            _ => None,
        };
        self.step(Input::Emit(Request { kind, block_id: block_id.to_string(), agent_name: self.agent_name(block_id), body }));
    }

    /// srv-internal sources: the body is app-controlled fixed text, so it is
    /// not redacted — only the preview=none setting strips it (in policy).
    /// BLOCKING on a name-cache miss, like `emit`.
    pub fn emit_fixed(&self, kind: NotifyKind, block_id: &str, body: Option<String>) {
        self.step(Input::Emit(Request { kind, block_id: block_id.to_string(), agent_name: self.agent_name(block_id), body }));
    }

    /// Re-evaluate without an event (e.g. pause settings changed) so the tray
    /// snapshot follows promptly.
    pub fn refresh(&self) {
        self.step(Input::Tick);
    }

    pub fn resolve(&self, block_id: &str, family: Family) {
        self.step(Input::Resolve { block_id: block_id.to_string(), family });
    }

    pub fn test(&self) {
        self.step(Input::Emit(Request {
            kind: NotifyKind::Test,
            block_id: String::new(),
            agent_name: String::new(),
            body: Some("You'll see alerts like this when an agent needs you.".to_string()),
        }));
    }

    pub fn focus(&self, conn_id: &str, report: FocusReport) {
        self.step(Input::Focus { conn_id: conn_id.to_string(), report });
    }

    pub fn disconnect(&self, conn_id: &str) {
        self.step(Input::Disconnect { conn_id: conn_id.to_string() });
    }

    /// Presenter feedback. Returns whether a window is connected so a
    /// click-while-no-window caller knows to open one.
    pub fn ack(&self, id: &str, clicked: bool) -> bool {
        self.step(Input::Ack { id: id.to_string(), clicked });
        self.has_window()
    }

    pub fn has_window(&self) -> bool {
        self.policy.lock().unwrap_or_else(|e| e.into_inner()).has_window()
    }

    /// One-shot: the block a click asked for before any window existed.
    pub fn take_pending_activation(&self) -> Option<String> {
        self.pending_activation.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_sanitization_strips_bidi_and_controls_and_caps() {
        assert_eq!(sanitize_name("lark"), "lark");
        assert_eq!(sanitize_name("evil\u{202E}txt.exe"), "eviltxt.exe");
        assert_eq!(sanitize_name("a\nb\tc"), "abc");
        assert_eq!(sanitize_name("   "), "An agent");
        assert_eq!(sanitize_name(&"x".repeat(100)).chars().count(), AGENT_NAME_MAX);
    }

    #[test]
    fn failure_classes_map_to_fixed_bodies_and_skip_user_deletes() {
        assert_eq!(failure_body("auth"), Some("Needs you to sign in again."));
        assert_eq!(failure_body("unknown_non_zero"), Some("Exited unexpectedly."));
        assert_eq!(failure_body("killed"), Some("Was stopped by the system (possibly out of memory)."));
        assert_eq!(failure_body("agent_deleted"), None);
        assert_eq!(failure_body("<script>"), None);
    }

    #[test]
    fn body_redaction() {
        assert_eq!(redact_body("\n  Which branch?\nmore", Preview::Redacted).as_deref(), Some("Which branch?"));
        assert_eq!(redact_body("Paste your API token here", Preview::Redacted).as_deref(),
            Some("Contains sensitive content — open AgentMux to view."));
        assert_eq!(redact_body("ok", Preview::None), None);
        let long = "y".repeat(300);
        assert_eq!(redact_body(&long, Preview::Redacted).unwrap().chars().count(), BODY_MAX + 1);
        assert_eq!(redact_body(&long, Preview::Full).unwrap().chars().count(), 201);
        assert_eq!(redact_body("   \n  ", Preview::Redacted), None);
    }

    #[test]
    fn settings_defaults_and_overrides() {
        let mut extra = std::collections::HashMap::new();
        let s = settings_from_extra(&extra);
        assert!(s.enabled);
        assert_eq!(s.when, When::Unfocused);
        assert_eq!(s.preview, Preview::Redacted);
        extra.insert("notify:os:when".into(), serde_json::json!("never"));
        extra.insert("notify:os:preview".into(), serde_json::json!("none"));
        extra.insert("notify:os:turncompleted".into(), serde_json::json!(false));
        extra.insert("notify:os:enabled".into(), serde_json::json!(false));
        extra.insert("notify:pause:until".into(), serde_json::json!(1234));
        extra.insert("notify:os:agentcrashed".into(), serde_json::json!(false));
        let s = settings_from_extra(&extra);
        assert_eq!(s.pause_until_ms, 1234);
        assert!(!s.pause_allow_attention);
        assert_eq!(s.kind_enabled.get(&NotifyKind::AgentCrashed), Some(&false));
        assert_eq!(s.kind_enabled.get(&NotifyKind::MessageNeedsReview), Some(&true));
        assert!(!s.enabled);
        assert_eq!(s.when, When::Never);
        assert_eq!(s.preview, Preview::None);
        assert_eq!(s.kind_enabled.get(&NotifyKind::TurnCompleted), Some(&false));
        assert_eq!(s.kind_enabled.get(&NotifyKind::InputWaiting), Some(&true));
    }
}
