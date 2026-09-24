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
//! Process-global (`OnceLock`) rather than an `AppState` field: it is
//! initialized from the first WS connection's `AppState` and is independent
//! of any one connection's lifetime.

use std::sync::{Arc, Mutex, OnceLock};

use crate::backend::mps::{Broker, MuxEvent};
use crate::backend::obj::Block;
use crate::backend::storage::store::Store;
use crate::backend::wconfig::ConfigState;

use super::policy::{Action, Family, FocusReport, Input, NotifyKind, PolicyState, Preview, Request, Settings, When};

pub const EVENT_NOTIFICATION: &str = "notification";
pub const EVENT_NOTIFICATION_RETRACT: &str = "notification:retract";
pub const EVENT_NOTIFICATION_ACTIVATE: &str = "notification:activate";

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
}

static ROUTER: OnceLock<Arc<Router>> = OnceLock::new();

/// The router, if any connection has initialized it yet.
pub fn get() -> Option<Arc<Router>> {
    ROUTER.get().cloned()
}

/// Get-or-init the global router. Idempotent; the first caller's handles win
/// (they are process-wide singletons in `AppState` anyway).
pub fn init(broker: Arc<Broker>, config: Arc<ConfigState>, store: Arc<Store>) -> Arc<Router> {
    ROUTER
        .get_or_init(|| {
            let r = Arc::new(Router {
                policy: Mutex::new(PolicyState::new()),
                broker,
                config,
                store,
                pending_activation: Mutex::new(None),
                names: Mutex::new(Default::default()),
            });
            spawn_ticker(Arc::downgrade(&r));
            r
        })
        .clone()
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
    };
    for kind in [NotifyKind::InputWaiting, NotifyKind::TurnCompleted, NotifyKind::TurnErrored] {
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
        let actions = {
            let mut p = self.policy.lock().unwrap_or_else(|e| e.into_inner());
            p.step(input, now_ms(), &settings)
        };
        for a in actions {
            self.publish(a);
        }
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
        let s = settings_from_extra(&extra);
        assert!(!s.enabled);
        assert_eq!(s.when, When::Never);
        assert_eq!(s.preview, Preview::None);
        assert_eq!(s.kind_enabled.get(&NotifyKind::TurnCompleted), Some(&false));
        assert_eq!(s.kind_enabled.get(&NotifyKind::InputWaiting), Some(&true));
    }
}
