// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wave Pub/Sub system: event brokering with scoped subscriptions.
//! Port of Go's pkg/wps/wps.go + wpstypes.go.

//!
//! The Broker supports:
//! - All-scope subscriptions (receive all events of a type)
//! - Exact-scope subscriptions (e.g., "block:uuid")
//! - Star-scope subscriptions (e.g., "block:*")
//! - Event persistence (history/replay)

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

// ---- Event type constants (match Go) ----

#[allow(dead_code)]
pub const EVENT_BLOCK_CLOSE: &str = "blockclose";
#[allow(dead_code)]
pub const EVENT_CONN_CHANGE: &str = "connchange";
pub const EVENT_SYS_INFO: &str = "sysinfo";
pub const EVENT_CONTROLLER_STATUS: &str = "controllerstatus";
pub const EVENT_WAVE_OBJ_UPDATE: &str = "waveobj:update";
pub const EVENT_BLOCK_FILE: &str = "blockfile";
pub const EVENT_INSTALL_PROGRESS: &str = "install_progress";
#[allow(dead_code)]
pub const EVENT_CONFIG: &str = "config";
#[allow(dead_code)]
pub const EVENT_USER_INPUT: &str = "userinput";
/// Fired by `SubprocessController::spawn_turn` when a user message is
/// picked up (either direct-spawn or queue drain). Frontend uses this to
/// promote pending `PendingMessage` entries into the conversation
/// document. Payload: `{ block_id, message_id }`.
pub const EVENT_AGENT_MESSAGE_ACCEPTED: &str = "agent-message-accepted";
#[allow(dead_code)]
pub const EVENT_ROUTE_GONE: &str = "route:gone";
pub const EVENT_BLOCK_STATS: &str = "blockstats";
/// Fired when an agent subprocess exits non-zero (or reports an error on its
/// terminal `result` frame). Carries the classified `AgentFailure` so the pane
/// shows the real cause instead of a bare exit code.
pub const EVENT_AGENT_FAILURE: &str = "agentfailure";
/// Fired by the persistent controller's stale-`--resume` recovery path
/// (`retry_after_resume_failure` / `publish_resume_retry_status`,
/// `docs/status/STATUS_STALE_RESUME_LIVE_REPRO_AND_FIX_PLAN_2026_08_23.md` §6.2)
/// so the pane can show a "Reconnecting…" readout instead of going silent for
/// the ~seconds-to-tens-of-seconds it can take the controller to detect a
/// stale registry `session_id` and respawn against a recovered one.
/// Payload: `{ "status": "retrying", "startedAt": "<rfc3339>" }` or
/// `{ "status": "resolved" }`. `persist: 2` (unlike `compaction_started`'s
/// `persist: 0`) is deliberate: both ends of this signal travel over this
/// same WPS channel (there's no separate out-of-band completion marker the
/// way `compact_boundary` is for compaction), so replaying the latest
/// retrying→resolved pair to a freshly (re)subscribed pane is always the
/// *correct* current state, not a stale echo — see
/// `publish_resume_retry_status`'s own doc comment for why 2, not 1.
pub const EVENT_AGENT_RESUME_RETRY: &str = "agent-resume-retry";
/// Fired by `SubagentWatcher::scan_session_subagents` (the pane-reopen
/// cold-backfill entry point, `subagent_watcher/scan.rs`) so a pane can
/// show its BrainSpinner overlay (`block.tsx`'s `ready()` gate) until its
/// own subagent/dispatch history has actually finished backfilling, instead
/// of exposing the Activity Dock's genuinely-changing intermediate states —
/// see `docs/retro/retro-activity-dock-flicker-survives-debounce-fix-2026-08-24.md`
/// §5 option 1/2. Payload: `{ "status": "started" | "done" }`. `persist: 2`,
/// same rationale as `EVENT_AGENT_RESUME_RETRY` above — both ends travel
/// over this one channel, so a mount-time `EventReadHistoryCommand` read
/// (guarding the same "pane subscribed after the backend already finished"
/// race that event's own hook already handles) always recovers the correct
/// current status rather than a stale one.
pub const EVENT_SUBAGENT_BACKFILL_STATUS: &str = "subagent:backfill_status";
/// Fired by `handle_shell_create` when a persistent shell is launched.
/// Frontend creates the ShellNode row on receipt.
/// Payload: `{ shell_id, cmd, cwd?, title, timestamp }`.
pub const EVENT_SHELL_NODE_CREATE: &str = "shell_node_create";
/// Fired per stdout/stderr line and on process exit by `ShellNodeRunner`.
/// `op: "chunk"` carries `{ shell_id, kind, content, timestamp }`;
/// `op: "exit"` carries `{ shell_id, exit_code, timestamp }`.
pub const EVENT_SHELL_CHUNK: &str = "shell_chunk";
/// Fired by the agent-pane PTY read loop when an OSC 0/2 window-title sequence
/// from Claude Code is extracted. Carries the normalised conversation-topic string
/// so the frontend can surface it as a `term:osc_title` tab label.
/// Payload: `{ "blockId": "...", "activity": "auth refactor" }`.
pub const EVENT_BLOCK_ACTIVITY: &str = "block:activity";
/// Fired whenever a cron job is created, fires, is paused/resumed, or
/// deleted (SPEC_SWARM_LONG_RUNNING_PROCESS_ROWS_2026_07_20 Phase 2).
/// Payload-free, unscoped — the Swarm pane just reloads the full list via
/// `cron.ListActive` on receipt, the same "any event of this type ⇒ refetch"
/// pattern `shell_node_create`/`shell_chunk(op:"exit")` already use.
pub const EVENT_CRON_CHANGED: &str = "cron_changed";
/// Fired by `handle_muxspect_dock_clear` when a `muxspect dock clear`
/// request is served. Scoped `block:<id>`, same convention as
/// `shell_node_create` — only a renderer currently displaying that block
/// receives it. Payload: `{ node_id }`. The receiving renderer flips that
/// one `ToolNode` to `status: "canceled"` via the `ForceCancelToolNode`
/// document-reducer command; a renderer without that node (already
/// resolved, or a different pane's block) no-ops.
/// See `docs/specs/SPEC_MUXSPECT_DOCK_DIAGNOSIS_AND_REMEDIATION_2026_08_06.md` §3.2.
pub const EVENT_DOCK_CLEAR: &str = "dock:clear";

// File operation constants
#[allow(dead_code)]
pub const FILE_OP_CREATE: &str = "create";
#[allow(dead_code)]
pub const FILE_OP_DELETE: &str = "delete";
pub const FILE_OP_APPEND: &str = "append";
pub const FILE_OP_TRUNCATE: &str = "truncate";
#[allow(dead_code)]
pub const FILE_OP_INVALIDATE: &str = "invalidate";

const MAX_PERSIST: usize = 4096;
const REMAKE_ARR_THRESHOLD: usize = 10 * 1024;

// ---- Types ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaveEvent {
    pub event: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub scopes: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub sender: String,
    #[serde(skip_serializing_if = "is_zero", default)]
    pub persist: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

fn is_zero(v: &usize) -> bool {
    *v == 0
}

impl WaveEvent {
    #[allow(dead_code)]
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionRequest {
    pub event: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub allscopes: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WSFileEventData {
    pub zoneid: String,
    pub filename: String,
    pub fileop: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub data64: String,
    /// File size immediately before this append (the chunk spans
    /// `[offset, offset + data.len())`). Only populated for filestore-
    /// write-through-backed appends (`handle_append_block_file`); absent
    /// means "no offset info available" — consumers should treat that as
    /// "always new" (the pre-existing, always-write behavior).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub offset: Option<u64>,
}

// ---- Client trait ----

/// Trait for event delivery to connected clients.
pub trait WpsClient: Send + Sync {
    fn send_event(&self, route_id: &str, event: WaveEvent);
}

// ---- Subscription internals ----

#[derive(Default)]
struct BrokerSubscription {
    /// Route IDs subscribed to all scopes for this event.
    all_subs: Vec<String>,
    /// Exact scope → route IDs.
    scope_subs: HashMap<String, Vec<String>>,
    /// Star/wildcard scope → route IDs.
    star_subs: HashMap<String, Vec<String>>,
}

impl BrokerSubscription {
    fn is_empty(&self) -> bool {
        self.all_subs.is_empty() && self.scope_subs.is_empty() && self.star_subs.is_empty()
    }
}

#[derive(Hash, Eq, PartialEq, Clone)]
struct PersistKey {
    event: String,
    scope: String,
}

struct PersistEventWrap {
    arr_total_adds: usize,
    events: Vec<WaveEvent>,
}

// ---- Broker ----

/// The central pub/sub broker for WaveEvents.
pub struct Broker {
    inner: Mutex<BrokerInner>,
}

/// Tracks `(route_id, event_name, scope)` tuples whose persisted
/// history has already been replayed to a given route. Skipping
/// replay on resubscribe prevents the frontend's `eventsub`
/// flushes (sent on every listener add/remove, once per conn_id)
/// from re-emitting completed bash logs on every pane mount or
/// tab switch. Codex P2 on PR #817; route keying updated PR #1418.
type ReplayKey = (String, String, String);

struct BrokerInner {
    client: Option<Box<dyn WpsClient>>,
    sub_map: HashMap<String, BrokerSubscription>,
    persist_map: HashMap<PersistKey, PersistEventWrap>,
    replayed: HashSet<ReplayKey>,
}

impl Broker {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(BrokerInner {
                client: None,
                sub_map: HashMap::new(),
                persist_map: HashMap::new(),
                replayed: HashSet::new(),
            }),
        }
    }

    pub fn set_client(&self, client: Box<dyn WpsClient>) {
        let mut inner = self.inner.lock().unwrap();
        inner.client = Some(client);
    }

    /// Subscribe a route to an event, optionally scoped.
    ///
    /// **Replay-on-subscribe**: after registering the route, immediately
    /// deliver any persisted events that match the subscription. Lets
    /// late subscribers catch up on the most recent state without
    /// waiting for the next publish — closes the race for live-log
    /// streaming where the frontend learns the tool_use_id only after
    /// the wrapper has already finished publishing.
    pub fn subscribe(&self, route_id: &str, sub: SubscriptionRequest) {
        if sub.event.is_empty() {
            return;
        }
        let mut inner = self.inner.lock().unwrap();
        // Remove existing subscription first (re-subscribe)
        Self::unsubscribe_nolock(&mut inner, route_id, &sub.event);

        let bs = inner
            .sub_map
            .entry(sub.event.clone())
            .or_default();

        if sub.allscopes {
            add_unique(&mut bs.all_subs, route_id);
        } else {
            for scope in &sub.scopes {
                if scope_has_star(scope) {
                    add_to_scope_map(&mut bs.star_subs, scope, route_id);
                } else {
                    add_to_scope_map(&mut bs.scope_subs, scope, route_id);
                }
            }
        }

        Self::replay_to_route(&mut inner, route_id, &sub);
    }

    /// Deliver any persisted events matching `sub` to `route_id`.
    /// Called inside `subscribe` so replay happens atomically under
    /// the broker lock — no live event published mid-replay can
    /// interleave.
    ///
    /// **Once-per-(route, event, scope).** The frontend
    /// (`frontend/app/store/wps.ts`) flushes `eventsub` on every
    /// listener add/remove; each WebSocket connection has its own
    /// `conn_id` as the route key (PR #1418). Replaying persisted
    /// history on each of those flushes would re-emit completed bash
    /// logs every pane mount / tab switch / sibling subscription. The
    /// `replayed` set tracks tuples that already received their backfill
    /// and short-circuits subsequent resubscribes. Cleared per-route in
    /// `unsubscribe_all` so a true reconnect (route dropped +
    /// re-registered) gets a fresh replay.
    ///
    /// Star-scope replay is intentionally not implemented (rare,
    /// requires scanning every persist key; can add later if needed).
    fn replay_to_route(
        inner: &mut BrokerInner,
        route_id: &str,
        sub: &SubscriptionRequest,
    ) {
        let client = match &inner.client {
            Some(c) => c,
            None => return,
        };

        let mut scopes_to_deliver: Vec<String> = Vec::new();
        if sub.allscopes {
            // "" key holds the global history per persist_event's scope_set.
            scopes_to_deliver.push(String::new());
        } else {
            for scope in &sub.scopes {
                if !scope_has_star(scope) {
                    scopes_to_deliver.push(scope.clone());
                }
            }
        }

        let mut to_send: Vec<WaveEvent> = Vec::new();
        for scope in scopes_to_deliver {
            let key = (route_id.to_string(), sub.event.clone(), scope.clone());
            if inner.replayed.contains(&key) {
                continue;
            }
            let pkey = PersistKey {
                event: sub.event.clone(),
                scope: scope.clone(),
            };
            if let Some(pe) = inner.persist_map.get(&pkey) {
                for event in &pe.events {
                    to_send.push(event.clone());
                }
            }
            inner.replayed.insert(key);
        }
        for event in to_send {
            client.send_event(route_id, event);
        }
    }

    /// Unsubscribe a route from a specific event.
    pub fn unsubscribe(&self, route_id: &str, event_name: &str) {
        let mut inner = self.inner.lock().unwrap();
        Self::unsubscribe_nolock(&mut inner, route_id, event_name);
    }

    /// Unsubscribe a route from all events.
    ///
    /// Also clears the `replayed` tracker for this route so a future
    /// reconnect (route registers again from scratch) gets a fresh
    /// replay of persisted history. Without this, a transient
    /// WebSocket drop would silently lose all subsequent replay.
    pub fn unsubscribe_all(&self, route_id: &str) {
        let mut inner = self.inner.lock().unwrap();
        let events: Vec<String> = inner.sub_map.keys().cloned().collect();
        for event in events {
            Self::unsubscribe_nolock(&mut inner, route_id, &event);
        }
        inner.replayed.retain(|(r, _, _)| r != route_id);
    }

    /// Purge all persisted history for a scope (e.g. `block:<id>` or
    /// `shell:<id>`), regardless of event name. Call when the underlying
    /// block/shell is deleted — `persist_map`'s key set is otherwise never
    /// pruned (each key's event Vec is capped, but the map only grows over
    /// a session's cumulative block/shell count). Also drops matching
    /// `replayed` entries so a stale scope can't linger across route
    /// reconnects.
    pub fn purge_scope(&self, scope: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.persist_map.retain(|k, _| k.scope != scope);
        inner.replayed.retain(|(_, _, s)| s != scope);
    }

    fn unsubscribe_nolock(inner: &mut BrokerInner, route_id: &str, event_name: &str) {
        let bs = match inner.sub_map.get_mut(event_name) {
            Some(bs) => bs,
            None => return,
        };
        bs.all_subs.retain(|s| s != route_id);
        remove_from_all_scopes(&mut bs.scope_subs, route_id);
        remove_from_all_scopes(&mut bs.star_subs, route_id);
        if bs.is_empty() {
            inner.sub_map.remove(event_name);
        }
    }

    /// Publish an event to all matching subscribers.
    pub fn publish(&self, event: WaveEvent) {
        let mut inner = self.inner.lock().unwrap();

        // Persist if requested
        if event.persist > 0 {
            Self::persist_event(&mut inner, &event);
        }

        let client = match &inner.client {
            Some(c) => c,
            None => return,
        };

        let route_ids = Self::get_matching_routes(&inner, &event);
        for route_id in route_ids {
            client.send_event(&route_id, event.clone());
        }
    }

    /// Read persisted event history.
    pub fn read_event_history(
        &self,
        event_type: &str,
        scope: &str,
        max_items: usize,
    ) -> Vec<WaveEvent> {
        if max_items == 0 {
            return Vec::new();
        }
        let inner = self.inner.lock().unwrap();
        let key = PersistKey {
            event: event_type.to_string(),
            scope: scope.to_string(),
        };
        match inner.persist_map.get(&key) {
            Some(pe) if !pe.events.is_empty() => {
                let n = max_items.min(pe.events.len());
                pe.events[pe.events.len() - n..].to_vec()
            }
            _ => Vec::new(),
        }
    }

    fn persist_event(inner: &mut BrokerInner, event: &WaveEvent) {
        let num_persist = event.persist.min(MAX_PERSIST);
        let mut scope_set: Vec<String> = event.scopes.clone();
        scope_set.push(String::new()); // "" scope for global persistence

        for scope in scope_set {
            let key = PersistKey {
                event: event.event.clone(),
                scope,
            };
            let pe = inner.persist_map.entry(key).or_insert_with(|| {
                PersistEventWrap {
                    arr_total_adds: 0,
                    events: Vec::with_capacity(num_persist),
                }
            });
            pe.events.push(event.clone());
            pe.arr_total_adds += 1;
            // Trim to max persist
            if pe.events.len() > num_persist {
                pe.events.drain(..pe.events.len() - num_persist);
            }
            // Compact if too many additions (reduce memory fragmentation)
            if pe.arr_total_adds > REMAKE_ARR_THRESHOLD {
                let compacted: Vec<WaveEvent> = pe.events.drain(..).collect();
                pe.events = compacted;
                pe.arr_total_adds = pe.events.len();
            }
        }
    }

    fn get_matching_routes(inner: &BrokerInner, event: &WaveEvent) -> Vec<String> {
        let bs = match inner.sub_map.get(&event.event) {
            Some(bs) => bs,
            None => return Vec::new(),
        };

        let mut route_ids: HashMap<&str, ()> = HashMap::new();

        // All-scope subscribers
        for route_id in &bs.all_subs {
            route_ids.insert(route_id, ());
        }

        // Exact-scope subscribers
        for scope in &event.scopes {
            if let Some(routes) = bs.scope_subs.get(scope) {
                for route_id in routes {
                    route_ids.insert(route_id, ());
                }
            }
            // Star-scope subscribers
            for (star_scope, routes) in &bs.star_subs {
                if star_match(star_scope, scope, ":") {
                    for route_id in routes {
                        route_ids.insert(route_id, ());
                    }
                }
            }
        }

        route_ids.keys().map(|s| s.to_string()).collect()
    }
}

impl Default for Broker {
    fn default() -> Self {
        Self::new()
    }
}

// ---- Helpers ----

fn scope_has_star(scope: &str) -> bool {
    scope.split(':').any(|part| part == "*" || part == "**")
}

/// Simple star matching: each segment separated by `sep` is compared.
/// "*" matches any single segment, "**" matches any remaining segments.
fn star_match(pattern: &str, value: &str, sep: &str) -> bool {
    let pat_parts: Vec<&str> = pattern.split(sep).collect();
    let val_parts: Vec<&str> = value.split(sep).collect();

    let mut pi = 0;
    let mut vi = 0;
    while pi < pat_parts.len() && vi < val_parts.len() {
        if pat_parts[pi] == "**" {
            return true; // matches everything remaining
        }
        if pat_parts[pi] != "*" && pat_parts[pi] != val_parts[vi] {
            return false;
        }
        pi += 1;
        vi += 1;
    }
    pi == pat_parts.len() && vi == val_parts.len()
}

fn add_unique(vec: &mut Vec<String>, val: &str) {
    if !vec.iter().any(|s| s == val) {
        vec.push(val.to_string());
    }
}

fn add_to_scope_map(map: &mut HashMap<String, Vec<String>>, scope: &str, route_id: &str) {
    let entry = map.entry(scope.to_string()).or_default();
    add_unique(entry, route_id);
}

fn remove_from_all_scopes(map: &mut HashMap<String, Vec<String>>, route_id: &str) {
    let empty_scopes: Vec<String> = map
        .iter_mut()
        .filter_map(|(scope, routes)| {
            routes.retain(|r| r != route_id);
            if routes.is_empty() {
                Some(scope.clone())
            } else {
                None
            }
        })
        .collect();
    for scope in empty_scopes {
        map.remove(&scope);
    }
}

/// Publish a single install-progress line to the frontend for a given block.
/// The frontend subscribes to `install_progress` events scoped to `block:{block_id}`
/// and displays each message as a log line in the agent presentation view.
pub fn publish_install_progress(broker: &Broker, block_id: &str, message: &str) {
    let scope = format!("block:{}", block_id);
    broker.publish(WaveEvent {
        event: EVENT_INSTALL_PROGRESS.to_string(),
        scopes: vec![scope],
        sender: String::new(),
        persist: 0,
        data: Some(serde_json::json!({ "message": message })),
    });
}

/// Publish a Claude Code OSC window-title activity string to the frontend
/// for a given agent-pane block. Frontend subscribes to `block:activity`
/// events scoped to `block:{block_id}` and writes the payload to
/// `term:osc_title` block metadata, which the tab label reads.
pub fn publish_block_activity(broker: &Broker, block_id: &str, activity: &str) {
    let scope = format!("block:{}", block_id);
    broker.publish(WaveEvent {
        event: EVENT_BLOCK_ACTIVITY.to_string(),
        scopes: vec![scope],
        sender: String::new(),
        persist: 0,
        data: Some(serde_json::json!({ "blockId": block_id, "activity": activity })),
    });
}

// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct TestClient {
        events: Mutex<Vec<(String, WaveEvent)>>,
    }

    impl TestClient {
        fn new() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
            }
        }

        fn received_events(&self) -> Vec<(String, WaveEvent)> {
            self.events.lock().unwrap().clone()
        }
    }

    impl WpsClient for TestClient {
        fn send_event(&self, route_id: &str, event: WaveEvent) {
            self.events
                .lock()
                .unwrap()
                .push((route_id.to_string(), event));
        }
    }

    impl WpsClient for Arc<TestClient> {
        fn send_event(&self, route_id: &str, event: WaveEvent) {
            self.events
                .lock()
                .unwrap()
                .push((route_id.to_string(), event));
        }
    }

    #[test]
    fn test_subscribe_all_scopes() {
        let broker = Broker::new();
        let client = Arc::new(TestClient::new());
        broker.set_client(Box::new(Arc::clone(&client)));

        broker.subscribe(
            "route-1",
            SubscriptionRequest {
                event: EVENT_WAVE_OBJ_UPDATE.to_string(),
                scopes: vec![],
                allscopes: true,
            },
        );

        broker.publish(WaveEvent {
            event: EVENT_WAVE_OBJ_UPDATE.to_string(),
            scopes: vec!["block:abc".to_string()],
            sender: String::new(),
            persist: 0,
            data: None,
        });

        let events = client.received_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "route-1");
    }

    #[test]
    fn test_subscribe_exact_scope() {
        let broker = Broker::new();
        let client = Arc::new(TestClient::new());
        broker.set_client(Box::new(Arc::clone(&client)));

        broker.subscribe(
            "route-1",
            SubscriptionRequest {
                event: EVENT_WAVE_OBJ_UPDATE.to_string(),
                scopes: vec!["block:abc".to_string()],
                allscopes: false,
            },
        );

        // Should match
        broker.publish(WaveEvent {
            event: EVENT_WAVE_OBJ_UPDATE.to_string(),
            scopes: vec!["block:abc".to_string()],
            sender: String::new(),
            persist: 0,
            data: None,
        });

        // Should NOT match
        broker.publish(WaveEvent {
            event: EVENT_WAVE_OBJ_UPDATE.to_string(),
            scopes: vec!["block:xyz".to_string()],
            sender: String::new(),
            persist: 0,
            data: None,
        });

        let events = client.received_events();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn test_subscribe_star_scope() {
        let broker = Broker::new();
        let client = Arc::new(TestClient::new());
        broker.set_client(Box::new(Arc::clone(&client)));

        broker.subscribe(
            "route-1",
            SubscriptionRequest {
                event: EVENT_WAVE_OBJ_UPDATE.to_string(),
                scopes: vec!["block:*".to_string()],
                allscopes: false,
            },
        );

        broker.publish(WaveEvent {
            event: EVENT_WAVE_OBJ_UPDATE.to_string(),
            scopes: vec!["block:abc".to_string()],
            sender: String::new(),
            persist: 0,
            data: None,
        });

        broker.publish(WaveEvent {
            event: EVENT_WAVE_OBJ_UPDATE.to_string(),
            scopes: vec!["tab:xyz".to_string()],
            sender: String::new(),
            persist: 0,
            data: None,
        });

        let events = client.received_events();
        assert_eq!(events.len(), 1); // only block:* matches block:abc
    }

    #[test]
    fn test_unsubscribe() {
        let broker = Broker::new();
        let client = Arc::new(TestClient::new());
        broker.set_client(Box::new(Arc::clone(&client)));

        broker.subscribe(
            "route-1",
            SubscriptionRequest {
                event: EVENT_BLOCK_CLOSE.to_string(),
                scopes: vec![],
                allscopes: true,
            },
        );

        broker.unsubscribe("route-1", EVENT_BLOCK_CLOSE);

        broker.publish(WaveEvent {
            event: EVENT_BLOCK_CLOSE.to_string(),
            scopes: vec![],
            sender: String::new(),
            persist: 0,
            data: None,
        });

        assert!(client.received_events().is_empty());
    }

    #[test]
    fn test_unsubscribe_all() {
        let broker = Broker::new();
        let client = Arc::new(TestClient::new());
        broker.set_client(Box::new(Arc::clone(&client)));

        broker.subscribe(
            "route-1",
            SubscriptionRequest {
                event: EVENT_BLOCK_CLOSE.to_string(),
                scopes: vec![],
                allscopes: true,
            },
        );
        broker.subscribe(
            "route-1",
            SubscriptionRequest {
                event: EVENT_CONFIG.to_string(),
                scopes: vec![],
                allscopes: true,
            },
        );

        broker.unsubscribe_all("route-1");

        broker.publish(WaveEvent {
            event: EVENT_BLOCK_CLOSE.to_string(),
            scopes: vec![],
            sender: String::new(),
            persist: 0,
            data: None,
        });
        broker.publish(WaveEvent {
            event: EVENT_CONFIG.to_string(),
            scopes: vec![],
            sender: String::new(),
            persist: 0,
            data: None,
        });

        assert!(client.received_events().is_empty());
    }

    /// Regression: replay-on-subscribe delivers persisted events that
    /// were published BEFORE the route subscribed. This closes the
    /// late-subscribe race for tool_chunk streaming.
    #[test]
    fn test_replay_on_subscribe_exact_scope() {
        let broker = Broker::new();
        let client = Arc::new(TestClient::new());
        broker.set_client(Box::new(Arc::clone(&client)));

        // Publish 5 persisted events BEFORE any subscriber exists.
        for i in 0..5 {
            broker.publish(WaveEvent {
                event: "tool_chunk".to_string(),
                scopes: vec!["block:abc".to_string()],
                sender: String::new(),
                persist: 10,
                data: Some(serde_json::json!({"tool_id": "t1", "n": i})),
            });
        }
        assert!(
            client.received_events().is_empty(),
            "no subscriber yet, no delivery"
        );

        // Subscribe to the matching scope — replay should fire.
        broker.subscribe(
            "route-1",
            SubscriptionRequest {
                event: "tool_chunk".to_string(),
                scopes: vec!["block:abc".to_string()],
                allscopes: false,
            },
        );

        let events = client.received_events();
        assert_eq!(events.len(), 5, "all 5 persisted events replayed");
        assert_eq!(events[0].1.data, Some(serde_json::json!({"tool_id": "t1", "n": 0})));
        assert_eq!(events[4].1.data, Some(serde_json::json!({"tool_id": "t1", "n": 4})));
    }

    /// Regression: re-subscribing the SAME route to the SAME
    /// (event, scope) does NOT replay again. The frontend's
    /// `eventsub` flushes happen on every listener add/remove, so
    /// the broker has to be idempotent across resubscribe calls.
    /// Codex P2 on PR #817.
    #[test]
    fn test_replay_on_resubscribe_is_idempotent() {
        let broker = Broker::new();
        let client = Arc::new(TestClient::new());
        broker.set_client(Box::new(Arc::clone(&client)));

        for i in 0..3 {
            broker.publish(WaveEvent {
                event: "tool_chunk".to_string(),
                scopes: vec!["block:abc".to_string()],
                sender: String::new(),
                persist: 10,
                data: Some(serde_json::json!({"n": i})),
            });
        }

        let sub = SubscriptionRequest {
            event: "tool_chunk".to_string(),
            scopes: vec!["block:abc".to_string()],
            allscopes: false,
        };

        broker.subscribe("route-1", sub.clone());
        assert_eq!(
            client.received_events().len(),
            3,
            "first subscribe replays all 3 persisted events"
        );

        // Re-subscribe (same route, same event+scope) — must not
        // replay a second time.
        broker.subscribe("route-1", sub.clone());
        assert_eq!(
            client.received_events().len(),
            3,
            "re-subscribe is a no-op for replay; received count stays at 3"
        );

        // Live publish after re-subscribe still delivers.
        broker.publish(WaveEvent {
            event: "tool_chunk".to_string(),
            scopes: vec!["block:abc".to_string()],
            sender: String::new(),
            persist: 10,
            data: Some(serde_json::json!({"n": "live"})),
        });
        assert_eq!(
            client.received_events().len(),
            4,
            "live publish after resubscribe is delivered exactly once"
        );

        // Disconnect (unsubscribe_all) + reconnect — fresh replay.
        broker.unsubscribe_all("route-1");
        broker.subscribe("route-1", sub.clone());
        assert_eq!(
            client.received_events().len(),
            8,
            "reconnect after unsubscribe_all clears the replayed tracker; \
             gets all 4 persisted events again"
        );
    }

    /// Regression: replay does NOT cross-pollute scopes. Subscriber to
    /// `block:abc` must not receive events persisted for `block:xyz`.
    #[test]
    fn test_replay_on_subscribe_scope_isolation() {
        let broker = Broker::new();
        let client = Arc::new(TestClient::new());
        broker.set_client(Box::new(Arc::clone(&client)));

        broker.publish(WaveEvent {
            event: "tool_chunk".to_string(),
            scopes: vec!["block:xyz".to_string()],
            sender: String::new(),
            persist: 10,
            data: Some(serde_json::json!({"tool_id": "other"})),
        });

        broker.subscribe(
            "route-1",
            SubscriptionRequest {
                event: "tool_chunk".to_string(),
                scopes: vec!["block:abc".to_string()],
                allscopes: false,
            },
        );

        let events = client.received_events();
        assert_eq!(events.len(), 0, "scope:abc must not get block:xyz events");
    }

    #[test]
    fn test_event_persistence() {
        let broker = Broker::new();
        let client = Arc::new(TestClient::new());
        broker.set_client(Box::new(Arc::clone(&client)));

        // Subscribe so events are dispatched
        broker.subscribe(
            "route-1",
            SubscriptionRequest {
                event: EVENT_SYS_INFO.to_string(),
                scopes: vec![],
                allscopes: true,
            },
        );

        // Publish persistent events
        for i in 0..5 {
            broker.publish(WaveEvent {
                event: EVENT_SYS_INFO.to_string(),
                scopes: vec!["cpu".to_string()],
                sender: String::new(),
                persist: 3, // keep last 3
                data: Some(serde_json::json!(i)),
            });
        }

        // Read history (global scope "")
        let history = broker.read_event_history(EVENT_SYS_INFO, "", 10);
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].data, Some(serde_json::json!(2)));
        assert_eq!(history[2].data, Some(serde_json::json!(4)));

        // Read scoped history
        let scoped = broker.read_event_history(EVENT_SYS_INFO, "cpu", 2);
        assert_eq!(scoped.len(), 2);
    }

    #[test]
    fn test_purge_scope_removes_only_matching_scope() {
        let broker = Broker::new();

        // Two different event names persisted under the same scope
        // (block:abc), plus one persisted under a different scope
        // (block:xyz) that must survive the purge.
        broker.publish(WaveEvent {
            event: "install_progress".to_string(),
            scopes: vec!["block:abc".to_string()],
            sender: String::new(),
            persist: 5,
            data: Some(serde_json::json!("a")),
        });
        broker.publish(WaveEvent {
            event: EVENT_BLOCK_ACTIVITY.to_string(),
            scopes: vec!["block:abc".to_string()],
            sender: String::new(),
            persist: 5,
            data: Some(serde_json::json!("b")),
        });
        broker.publish(WaveEvent {
            event: "install_progress".to_string(),
            scopes: vec!["block:xyz".to_string()],
            sender: String::new(),
            persist: 5,
            data: Some(serde_json::json!("c")),
        });

        assert_eq!(broker.read_event_history("install_progress", "block:abc", 10).len(), 1);
        assert_eq!(broker.read_event_history(EVENT_BLOCK_ACTIVITY, "block:abc", 10).len(), 1);
        assert_eq!(broker.read_event_history("install_progress", "block:xyz", 10).len(), 1);

        broker.purge_scope("block:abc");

        // Both event names under the purged scope are gone...
        assert_eq!(broker.read_event_history("install_progress", "block:abc", 10).len(), 0);
        assert_eq!(broker.read_event_history(EVENT_BLOCK_ACTIVITY, "block:abc", 10).len(), 0);
        // ...but the other scope is untouched.
        assert_eq!(broker.read_event_history("install_progress", "block:xyz", 10).len(), 1);
    }

    #[test]
    fn test_purge_scope_on_unknown_scope_is_noop() {
        // Purging a scope with no persisted entries must not panic.
        let broker = Broker::new();
        broker.purge_scope("block:never-existed");
    }

    #[test]
    fn test_star_match() {
        assert!(star_match("block:*", "block:abc", ":"));
        assert!(star_match("*:abc", "block:abc", ":"));
        assert!(!star_match("block:*", "tab:abc", ":"));
        assert!(star_match("**", "block:abc:xyz", ":"));
        assert!(!star_match("block:*", "block:abc:xyz", ":")); // * matches one segment only
    }

    #[test]
    fn test_wave_event_serialization() {
        let event = WaveEvent {
            event: "test".to_string(),
            scopes: vec!["scope1".to_string()],
            sender: String::new(),
            persist: 0,
            data: Some(serde_json::json!({"key": "value"})),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: WaveEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.event, "test");
        assert_eq!(parsed.scopes, vec!["scope1"]);
        // Empty sender and zero persist should be omitted
        assert!(!json.contains("\"sender\""));
        assert!(!json.contains("\"persist\""));
    }

    #[test]
    fn test_subscription_request_serialization() {
        let req = SubscriptionRequest {
            event: "blockclose".to_string(),
            scopes: vec!["block:123".to_string()],
            allscopes: false,
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: SubscriptionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.event, "blockclose");
    }

    #[test]
    fn test_no_client_publish_does_not_panic() {
        let broker = Broker::new();
        // No client set — should not panic
        broker.publish(WaveEvent {
            event: "test".to_string(),
            scopes: vec![],
            sender: String::new(),
            persist: 0,
            data: None,
        });
    }

    #[test]
    fn test_double_star_scope() {
        let broker = Broker::new();
        let client = Arc::new(TestClient::new());
        broker.set_client(Box::new(Arc::clone(&client)));

        broker.subscribe(
            "route-1",
            SubscriptionRequest {
                event: EVENT_WAVE_OBJ_UPDATE.to_string(),
                scopes: vec!["**".to_string()],
                allscopes: false,
            },
        );

        broker.publish(WaveEvent {
            event: EVENT_WAVE_OBJ_UPDATE.to_string(),
            scopes: vec!["block:abc:def".to_string()],
            sender: String::new(),
            persist: 0,
            data: None,
        });

        assert_eq!(client.received_events().len(), 1);
    }
}
