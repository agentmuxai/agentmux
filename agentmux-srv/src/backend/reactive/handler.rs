// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0


use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::sanitize::{format_injected_message, is_sensitive_message, sanitize_message, validate_agent_id, wrap_jekt_message};
use super::types::*;
use super::{now_unix_millis, sha256_hex, AUDIT_LOG_MAX, RATE_LIMIT_MAX};

// ---- Rate Limiter ----

pub(super) struct RateLimiter {
    tokens: u32,
    max_tokens: u32,
    last_refill: Instant,
}

impl RateLimiter {
    pub(super) fn new(max_tokens: u32) -> Self {
        Self {
            tokens: max_tokens,
            max_tokens,
            last_refill: Instant::now(),
        }
    }

    pub(super) fn check(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill);
        if elapsed >= Duration::from_secs(1) {
            self.tokens = self.max_tokens;
            self.last_refill = now;
        }
        if self.tokens > 0 {
            self.tokens -= 1;
            true
        } else {
            false
        }
    }
}

// ---- Supervisor nudge-ceiling tracking ----

/// Per-target-agent consecutive-nudge tracking for the Warden Supervisor
/// guardrail. Keyed on the target agent's lowercased name in
/// `Handler::nudge_counters`.
struct NudgeCounterState {
    /// The target's `AgentRegistration::registration_nonce` as of the last
    /// nudge — a respawn (new nonce) resets the counter, since "consecutive"
    /// only makes sense within one continuous run. Real nonces are ≥ 1
    /// (persistent-controller spawns only — `persistent::next_registration_
    /// nonce`); PTY/shell and HTTP-register paths always register with 0
    /// ("not recorded"), so nonce alone can't detect a respawn for them —
    /// `block_id` below covers that case instead.
    registration_nonce: u64,
    /// The target's block id as of the last nudge. A PTY/shell-registered
    /// agent that's closed and relaunched gets a fresh block id (a new
    /// pane), which nonce (always 0 for these paths) can't see — comparing
    /// block_id catches that respawn case reagent flagged as a P1 gap
    /// (nonce-only reset never fires for PTY agents). Doesn't help detect a
    /// respawn that reuses the same pane/block id (e.g. a one-shot
    /// SubprocessController that re-registers every turn in-place) — that
    /// case still relies on `NUDGE_COOLDOWN_RESET_MS` alone, same as before.
    block_id: String,
    count: u32,
    last_nudge_at_ms: u64,
}

/// Max consecutive auto-continue nudges a Supervisor may send to the same
/// target agent (within one registration / cooldown window) before
/// `record_supervisor_decision` refuses and forces a decline instead. Bounds
/// a runaway auto-continue loop — see
/// docs/analysis/ANALYSIS_WARDEN_AUTO_CONTROLLER_CONTINUATION_WATCHER_2026_08_12.md.
pub(super) const MAX_CONSECUTIVE_AUTO_CONTINUES: u32 = 5;

/// Gap since the last nudge after which the consecutive counter resets, on
/// the theory that a long-idle target has effectively started a new work
/// session even without a fresh `registration_nonce`.
pub(super) const NUDGE_COOLDOWN_RESET_MS: u64 = 30 * 60 * 1000;

/// The one, fixed message a `SupervisorAction::Nudge` ever delivers.
/// Deliberately not a parameter — see the doc on `record_supervisor_decision`'s
/// `Nudge` arm for why a free-form message defeats the guardrail this
/// exists for.
pub(super) const NUDGE_MESSAGE: &str = "Continue the task you were already doing.";

// ---- Handler ----

/// Core reactive messaging handler.
///
/// Manages agent registrations, rate limiting, message injection,
/// and audit logging.
pub struct Handler {
    // ---- Identity M2 (SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md
    // §4.4): identity is the key, names are typed bindings. ----
    //
    /// The key, when known: the agent's UID (`db_agents.id`) → its block.
    /// One-to-one — a UID registering on a new block evicts its old one
    /// (a respawn). Empty for blocks the server has no row for (quick-launch
    /// panes, PTY shells), which are then reachable by name only.
    uid_to_block: HashMap<String, String>,
    block_to_uid: HashMap<String, String>,
    /// Lowercased name → every block bound to it. A name may hold several
    /// blocks for the first time: two agents whose names collide both stay
    /// registered (today one silently evicts the other), and delivery to
    /// that name is refused with the candidates instead of guessed (§5.2).
    name_to_blocks: HashMap<String, Vec<String>>,
    /// A block's own names, typed — see [`NameBindings`].
    block_names: HashMap<String, NameBindings>,
    /// Keyed by block id (was: by lowercased name). `agent_id` in the record
    /// is the block's *display* binding.
    agent_info: HashMap<String, AgentRegistration>,
    input_sender: Option<InputSender>,
    /// Controller-aware delivery for non-PTY agents (persistent stream-json / ACP).
    /// When set, it is tried before the PTY keystroke path so messages reach (and
    /// steer) agents that have no terminal. See `set_message_sender`.
    message_sender: Option<MessageSender>,
    /// Queries a resolved delivery target's own live identity before
    /// delivering, as a check independent of this handler's own
    /// `agent_to_block` map. See `AgentIdentityConfirmer`'s doc comment and
    /// `set_agent_identity_confirmer`.
    agent_identity_confirmer: Option<AgentIdentityConfirmer>,
    /// Same shape as `agent_identity_confirmer`, but backed by
    /// `Controller::stable_agent_id()` (the frozen `AGENTMUX_AGENT_ID`)
    /// instead of `Controller::agent_id()` (the live, renameable display
    /// name). Checked as an alternative match in `inject_message_inner`'s
    /// #2695 identity check — a jekt addressed to the stable name would
    /// otherwise always fail once the display binding has moved on
    /// post-rename. See `NameBindings::stable`.
    stable_agent_identity_confirmer: Option<AgentIdentityConfirmer>,
    /// Identity M2: backed by `Controller::stable_agent_uid()`. Consulted
    /// ONLY when the target resolved by UID, and `None` is "unverifiable"
    /// (delivery proceeds) — never combined with the two name confirmers
    /// above. A registered-but-unspawned persistent agent has a live name
    /// (`set_agent_id` is called without a spawn) but no UID yet; running a
    /// UID target through the name check would reject it as a mismatch
    /// before the start-on-delivery fall-through could run (spec §4.4.3,
    /// revision 4.0's P0).
    uid_identity_confirmer: Option<AgentIdentityConfirmer>,
    /// Identity M2: is this block still alive (has a controller)? The
    /// garbage collector that replaces name eviction (spec §4.4.4 Q9):
    /// when a name resolves to several blocks, the dead ones are swept
    /// before ambiguity is decided. `None` (tests) treats every block as
    /// live.
    block_liveness: Option<BlockLivenessProbe>,
    audit_log: Vec<AuditLogEntry>,
    rate_limiter: RateLimiter,
    include_source_in_message: bool,
    /// Warden Supervisor consecutive-nudge ceiling state, keyed on the
    /// target agent's lowercased name. In-memory only (same lifecycle as
    /// `audit_log`) — not persisted across a restart.
    nudge_counters: HashMap<String, NudgeCounterState>,
}

/// A block's names, typed (identity M2, spec §4.4.2).
///
/// `display` is the name the block was most recently registered under —
/// the Register-tail and the HTTP presence path supply it every time — and
/// is **replaced** on re-registration, so a retired name does not linger and
/// make a later agent ambiguous. `stable` is the `AGENTMUX_AGENT_ID` parked
/// at spawn (what the alias map used to hold) and is **kept** across display
/// changes: it is the value GitHub PR-body tags embed precisely because it
/// does not change on rename (`INCIDENT_2026_09_09_JEKT_STABLE_ID_ALIAS.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameBindings {
    pub display: String,
    pub stable: Option<String>,
}

impl NameBindings {
    /// Every lowercased key this block is bound under.
    fn keys(&self) -> Vec<String> {
        let mut keys = vec![self.display.to_lowercase()];
        if let Some(s) = &self.stable {
            let k = s.to_lowercase();
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
        keys
    }
}

/// What a by-name unregister did (identity M2, spec §4.4.4 Q7). A name held
/// by several live blocks is refused, never silently no-op'd as success.
#[derive(Debug, Clone)]
pub enum UnregisterOutcome {
    Removed {
        block_id: String,
        /// Every name the block was bound under, so the caller can tear
        /// down name-keyed side state (file registry, cloud subscription)
        /// for each of them, not just the display name.
        names: Vec<String>,
    },
    NotFound,
    Ambiguous(Vec<AgentRegistration>),
}

/// How `inject_message_inner` resolved its target (spec §4.4.3).
enum TargetResolution {
    Uid(String),
    Name(String),
    Ambiguous(Vec<AgentRegistration>),
    NotFound,
}

/// Human-readable candidate list for an ambiguity refusal — the §5.2 shape:
/// enough for a model to retry by UID and for a human to act on.
fn describe_candidates(candidates: &[AgentRegistration]) -> String {
    candidates
        .iter()
        .map(|c| {
            format!(
                "uid={} block={} name={}",
                c.uid.as_deref().unwrap_or("none"),
                c.block_id,
                c.agent_id
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

impl Handler {
    /// Create a new handler without an input sender.
    /// Call `set_input_sender` before injecting messages.
    pub fn new() -> Self {
        Self {
            uid_to_block: HashMap::new(),
            block_to_uid: HashMap::new(),
            name_to_blocks: HashMap::new(),
            block_names: HashMap::new(),
            agent_info: HashMap::new(),
            input_sender: None,
            message_sender: None,
            agent_identity_confirmer: None,
            stable_agent_identity_confirmer: None,
            uid_identity_confirmer: None,
            block_liveness: None,
            audit_log: Vec::with_capacity(AUDIT_LOG_MAX),
            rate_limiter: RateLimiter::new(RATE_LIMIT_MAX),
            include_source_in_message: false,
            nudge_counters: HashMap::new(),
        }
    }

    /// Set the input sender function for message injection.
    pub fn set_input_sender(&mut self, sender: InputSender) {
        self.input_sender = Some(sender);
    }

    /// Set the controller-aware message sender. When present, `inject_message`
    /// tries it first: persistent stream-json and ACP agents receive a structured
    /// message on their live channel (mid-turn steering); PTY-based agents report
    /// back so injection falls through to the keystroke path.
    pub fn set_message_sender(&mut self, sender: MessageSender) {
        self.message_sender = Some(sender);
    }

    /// Set the agent-identity confirmer used by `inject_message_inner` to
    /// double-check a resolved delivery target against its own live,
    /// spawn-time-captured identity before delivering. Optional — if never
    /// set, delivery proceeds exactly as before this check existed (fail-open).
    pub fn set_agent_identity_confirmer(&mut self, confirmer: AgentIdentityConfirmer) {
        self.agent_identity_confirmer = Some(confirmer);
    }

    /// See `stable_agent_identity_confirmer`'s field doc comment.
    pub fn set_stable_agent_identity_confirmer(&mut self, confirmer: AgentIdentityConfirmer) {
        self.stable_agent_identity_confirmer = Some(confirmer);
    }

    /// See `uid_identity_confirmer`'s field doc comment (identity M2).
    pub fn set_uid_identity_confirmer(&mut self, confirmer: AgentIdentityConfirmer) {
        self.uid_identity_confirmer = Some(confirmer);
    }

    /// See `block_liveness`'s field doc comment (identity M2).
    pub fn set_block_liveness(&mut self, probe: BlockLivenessProbe) {
        self.block_liveness = Some(probe);
    }

    /// Set whether to include source agent prefix in injected messages.
    #[allow(dead_code)]
    pub fn set_include_source(&mut self, include: bool) {
        self.include_source_in_message = include;
    }

    /// Register an agent with a block.
    /// Name-only registration. Every production site now goes through
    /// [`register_agent_full`] with its UID and counter site; this is the
    /// test-facing form (and the shape the pre-M2 API had).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn register_agent(
        &mut self,
        agent_id: &str,
        block_id: &str,
        tab_id: Option<&str>,
    ) -> Result<(), String> {
        self.register_agent_full(
            agent_id,
            block_id,
            tab_id,
            0,
            None,
            None,
            "registration.no_uid.unspecified",
        )
    }

    /// [`register_agent`], recording the registering persistent-controller
    /// spawn's process-wide registration nonce
    /// (`AgentRegistration::registration_nonce`) so its own exit-handler
    /// can later compare-and-remove ([`unregister_block_if_nonce`])
    /// instead of blindly wiping a fallback respawn's fresh registration
    /// (issue #2363).
    ///
    /// `alias`, if given, becomes the block's *stable* name binding
    /// ([`NameBindings::stable`]): kept when a later call replaces the
    /// display binding, so a jekt addressed to `AGENTMUX_AGENT_ID` still
    /// resolves after the display name has moved on post-rename.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn register_agent_with_nonce(
        &mut self,
        agent_id: &str,
        block_id: &str,
        tab_id: Option<&str>,
        registration_nonce: u64,
        alias: Option<&str>,
    ) -> Result<(), String> {
        self.register_agent_full(
            agent_id,
            block_id,
            tab_id,
            registration_nonce,
            alias,
            None,
            "registration.no_uid.unspecified",
        )
    }

    /// The full registration (identity M2, spec §4.4.2): `display` is the
    /// block's display binding, `stable` its stable binding (set or kept),
    /// `uid` its identity when the caller knows it, and `no_uid_counter` the
    /// §9.2 counter to bump when it does not.
    ///
    /// **Eviction is by identity, never by name:**
    /// - `uid` already registered on another block → that block is evicted
    ///   entirely (a respawn).
    /// - a name already held by another block is left alone **unless**
    ///   neither block has a UID — that case keeps today's name eviction
    ///   exactly, and is counted.
    /// - a block that has a UID keeps it when re-registered without one
    ///   (sticky — a transient row miss on the presence path must not drop a
    ///   live block out of `uid_to_block`); a block without one is upgraded
    ///   in place when a later registration supplies one.
    // Eight parameters because the three identity facets (display, stable,
    // uid) plus the counter site are each optional independently; bundling
    // them would move the same eight names into a struct literal at every
    // call site without making any of them clearer.
    #[allow(clippy::too_many_arguments)]
    pub fn register_agent_full(
        &mut self,
        display: &str,
        block_id: &str,
        tab_id: Option<&str>,
        registration_nonce: u64,
        stable: Option<&str>,
        uid: Option<&str>,
        no_uid_counter: &'static str,
    ) -> Result<(), String> {
        if !validate_agent_id(display) {
            return Err(format!("invalid agent ID: {}", display));
        }
        let stable = stable.filter(|s| validate_agent_id(s));
        let uid = uid
            .map(str::trim)
            .filter(|u| !u.is_empty() && validate_agent_id(u));

        let mut evicted_block: Option<String> = None;
        let mut evicted_agent: Option<String> = None;

        // 1. Identity eviction: this UID on a different block is a respawn.
        if let Some(uid) = uid {
            if let Some(old) = self.uid_to_block.get(uid).cloned() {
                if old != block_id {
                    evicted_agent = self.block_names.get(&old).map(|n| n.display.clone());
                    self.remove_block_bindings(&old);
                    evicted_block = Some(old);
                }
            }
        }

        // 2. This block's effective UID: supplied, else whatever it already
        //    had (sticky). A block that changes UID — rare: a reconfigured
        //    pane, or a pane reused for a different agent whose spawn env
        //    now carries the new row's id — drops its OLD `uid_to_block`
        //    entry here, otherwise a delivery addressed to the old UID would
        //    still resolve to this block and then fail the UID confirmer as
        //    a spurious mismatch instead of a clean not-found (ReAgent P2 on
        //    PR #3560).
        let effective_uid: Option<String> = uid
            .map(str::to_string)
            .or_else(|| self.block_to_uid.get(block_id).cloned());
        if let (Some(new_uid), Some(old_uid)) = (uid, self.block_to_uid.get(block_id).cloned()) {
            if old_uid != new_uid {
                tracing::info!(
                    block_id = %block_id,
                    old_uid = %old_uid,
                    new_uid = %new_uid,
                    "reactive: block re-registered under a different uid — dropping the old uid mapping (identity M2)"
                );
                if self.uid_to_block.get(&old_uid).is_some_and(|b| b == block_id) {
                    self.uid_to_block.remove(&old_uid);
                }
            }
        }
        if effective_uid.is_none() {
            crate::backend::agent_resolve::record_uid_fallback(no_uid_counter);
        }

        // 3. Name eviction — only when neither side has a UID (today's
        //    semantics, counted). Two identified agents sharing a name both
        //    stay; delivery to that name is then refused with candidates.
        let prev = self.block_names.get(block_id).cloned();
        let new_keys: Vec<String> = {
            let mut k = vec![display.to_lowercase()];
            if let Some(s) = stable {
                let s = s.to_lowercase();
                if !k.contains(&s) {
                    k.push(s);
                }
            }
            k
        };
        if effective_uid.is_none() {
            for key in &new_keys {
                let others: Vec<String> = self
                    .name_to_blocks
                    .get(key)
                    .map(|v| v.iter().filter(|b| b.as_str() != block_id).cloned().collect())
                    .unwrap_or_default();
                for other in others {
                    if self.block_to_uid.contains_key(&other) {
                        continue;
                    }
                    if evicted_agent.is_none() {
                        evicted_agent = self.block_names.get(&other).map(|n| n.display.clone());
                    }
                    self.remove_block_bindings(&other);
                    if evicted_block.is_none() {
                        evicted_block = Some(other);
                    }
                    crate::backend::agent_resolve::record_uid_fallback("registration.name_evicted_no_uid");
                }
            }
        }

        // 4. Replace the display binding; set or keep the stable one.
        let stable_final: Option<String> = stable
            .map(str::to_string)
            .or_else(|| prev.as_ref().and_then(|p| p.stable.clone()));
        if let Some(prev) = &prev {
            for key in prev.keys() {
                self.unbind_name(&key, block_id);
            }
            if prev.display.to_lowercase() != display.to_lowercase() && evicted_agent.is_none() {
                evicted_agent = Some(prev.display.clone());
            }
        }
        let bindings = NameBindings {
            display: display.to_string(),
            stable: stable_final,
        };
        for key in bindings.keys() {
            self.bind_name(&key, block_id);
        }
        self.block_names.insert(block_id.to_string(), bindings);
        if let Some(u) = &effective_uid {
            self.uid_to_block.insert(u.clone(), block_id.to_string());
            self.block_to_uid.insert(block_id.to_string(), u.clone());
        }

        let now = now_unix_millis();
        self.agent_info.insert(
            block_id.to_string(),
            AgentRegistration {
                agent_id: display.to_string(),
                block_id: block_id.to_string(),
                tab_id: tab_id.map(|s| s.to_string()),
                registered_at: now,
                last_seen: now,
                registration_nonce,
                uid: effective_uid,
            },
        );

        self.log_audit_registration(
            "register",
            display,
            block_id,
            evicted_block.as_deref(),
            evicted_agent.as_deref(),
        );

        Ok(())
    }

    fn bind_name(&mut self, key: &str, block_id: &str) {
        let blocks = self.name_to_blocks.entry(key.to_string()).or_default();
        if !blocks.iter().any(|b| b == block_id) {
            blocks.push(block_id.to_string());
        }
    }

    fn unbind_name(&mut self, key: &str, block_id: &str) {
        if let Some(blocks) = self.name_to_blocks.get_mut(key) {
            blocks.retain(|b| b != block_id);
            if blocks.is_empty() {
                self.name_to_blocks.remove(key);
            }
        }
    }

    /// Drop every trace of `block_id` — names, UID, record. No audit entry;
    /// callers log with the context they have.
    fn remove_block_bindings(&mut self, block_id: &str) {
        if let Some(names) = self.block_names.remove(block_id) {
            for key in names.keys() {
                self.unbind_name(&key, block_id);
            }
        }
        if let Some(uid) = self.block_to_uid.remove(block_id) {
            if self.uid_to_block.get(&uid).is_some_and(|b| b == block_id) {
                self.uid_to_block.remove(&uid);
            }
        }
        self.agent_info.remove(block_id);
    }

    /// Blocks bound to `key` (lowercased name) that are still alive, per the
    /// liveness probe. Read-only: does not sweep. `None` probe = all live.
    fn live_blocks_for_key(&self, key: &str) -> Vec<String> {
        let Some(blocks) = self.name_to_blocks.get(key) else {
            return Vec::new();
        };
        match &self.block_liveness {
            Some(probe) => blocks.iter().filter(|b| probe(b)).cloned().collect(),
            None => blocks.clone(),
        }
    }

    /// Resolve a delivery target (spec §4.4.3): UID first, then name among
    /// live blocks — sweeping dead ones on the way (§4.4.4 Q9) — refusing an
    /// ambiguous name with its candidates rather than guessing.
    fn resolve_target(&mut self, target: &str) -> TargetResolution {
        if let Some(block) = self
            .uid_to_block
            .get(target)
            .or_else(|| self.uid_to_block.get(&target.to_lowercase()))
        {
            return TargetResolution::Uid(block.clone());
        }
        let key = target.to_lowercase();
        let blocks = self.name_to_blocks.get(&key).cloned().unwrap_or_default();
        if blocks.is_empty() {
            return TargetResolution::NotFound;
        }
        let mut live = Vec::with_capacity(blocks.len());
        for block in blocks {
            let alive = self
                .block_liveness
                .as_ref()
                .is_none_or(|probe| probe(&block));
            if alive {
                live.push(block);
            } else {
                let display_name = self
                    .block_names
                    .get(&block)
                    .map(|n| n.display.clone())
                    .unwrap_or_default();
                tracing::info!(
                    block_id = %block,
                    name = %display_name,
                    "reactive: sweeping dead registration while resolving a name (identity M2)"
                );
                self.remove_block_bindings(&block);
                self.log_audit_registration("swept", &display_name, &block, None, None);
                crate::backend::agent_resolve::record_uid_fallback("registry.swept_dead");
            }
        }
        match live.len() {
            0 => TargetResolution::NotFound,
            1 => {
                crate::backend::agent_resolve::record_uid_fallback("delivery.resolved_by_name");
                TargetResolution::Name(live.remove(0))
            }
            _ => {
                crate::backend::agent_resolve::record_uid_fallback("delivery.ambiguous");
                let candidates = live
                    .iter()
                    .filter_map(|b| self.agent_info.get(b).cloned())
                    .collect();
                TargetResolution::Ambiguous(candidates)
            }
        }
    }

    /// Unregister by name (identity M2, spec §4.4.4 Q7). One live block →
    /// removed. Several → nothing is removed and the candidates are
    /// returned; the caller must not report success. None → `NotFound`.
    pub fn unregister_agent(&mut self, agent_id: &str) -> UnregisterOutcome {
        let key = agent_id.to_lowercase();
        let live = self.live_blocks_for_key(&key);
        match live.len() {
            0 => UnregisterOutcome::NotFound,
            1 => {
                let block_id = live[0].clone();
                let names = self.unregister_block(&block_id);
                UnregisterOutcome::Removed { block_id, names }
            }
            _ => {
                crate::backend::agent_resolve::record_uid_fallback(
                    "unregister.ambiguous_without_block",
                );
                let candidates = live
                    .iter()
                    .filter_map(|b| self.agent_info.get(b).cloned())
                    .collect::<Vec<_>>();
                tracing::warn!(
                    name = %agent_id,
                    candidates = %describe_candidates(&candidates),
                    "reactive: refusing to unregister an ambiguous name without a block id"
                );
                UnregisterOutcome::Ambiguous(candidates)
            }
        }
    }

    /// Unregister by block ID. Returns every name the block was bound under
    /// (empty if it was not registered) so name-keyed side state can be
    /// torn down for each.
    pub fn unregister_block(&mut self, block_id: &str) -> Vec<String> {
        let Some(names) = self.block_names.get(block_id).cloned() else {
            return Vec::new();
        };
        let display = names.display.clone();
        self.remove_block_bindings(block_id);
        self.log_audit_registration("unregister", &display, block_id, None, None);
        let mut out = vec![display];
        if let Some(s) = names.stable {
            if !out.iter().any(|n| n.eq_ignore_ascii_case(&s)) {
                out.push(s);
            }
        }
        out
    }

    /// Unregister by block ID **only if** the current registration was
    /// written by the spawn with `expected_nonce` — a
    /// compare-and-remove for persistent-controller exit-handlers (issue
    /// #2363: the handler's `is_current_generation` gate is read once,
    /// while a fallback respawn re-registers on a parallel task; an
    /// unconditional [`unregister_block`] here could wipe the NEW spawn's
    /// registration, leaving the live agent invisible to Tier-1 delivery
    /// with nothing left to re-register it). The nonce is process-wide
    /// unique, so the guard also holds across controller replacement
    /// (`resync_controller` — codex P1 on PR #2500), where a
    /// controller-local generation would restart at 1 and collide. Runs
    /// atomically under the handler's own lock (via the outer wrapper).
    /// A registration with no recorded nonce (0 — HTTP/PTY paths) is
    /// never removed by this variant: leaving a stale entry to the TTL
    /// sweep is strictly safer than deleting a live one.
    ///
    /// Returns true if the registration was ours and was removed.
    pub fn unregister_block_if_nonce(&mut self, block_id: &str, expected_nonce: u64) -> bool {
        let Some(info) = self.agent_info.get(block_id) else {
            return false;
        };
        let matches = expected_nonce != 0 && info.registration_nonce == expected_nonce;
        if !matches {
            tracing::info!(
                block_id = %block_id,
                expected_nonce = expected_nonce,
                "reactive: registration changed hands since this spawn registered — skipping unregister"
            );
            return false;
        }
        self.unregister_block(block_id);
        true
    }

    /// Update the last_seen timestamp for an agent.
    #[allow(dead_code)]
    pub fn update_last_seen(&mut self, agent_id: &str) {
        let now = now_unix_millis();
        for block in self.live_blocks_for_key(&agent_id.to_lowercase()) {
            if let Some(info) = self.agent_info.get_mut(&block) {
                info.last_seen = now;
            }
        }
    }

    /// Get agent registration by agent ID — a UID, or a name held by
    /// exactly one live block. A name held by several is `None`, counted
    /// (`lookup.ambiguous_name`): callers that authorize on it fail closed,
    /// and callers that display on it show nothing rather than the wrong one.
    pub fn get_agent(&self, agent_id: &str) -> Option<&AgentRegistration> {
        if let Some(block) = self
            .uid_to_block
            .get(agent_id)
            .or_else(|| self.uid_to_block.get(&agent_id.to_lowercase()))
        {
            return self.agent_info.get(block);
        }
        let live = self.live_blocks_for_key(&agent_id.to_lowercase());
        match live.len() {
            1 => self.agent_info.get(&live[0]),
            0 => None,
            _ => {
                crate::backend::agent_resolve::record_uid_fallback("lookup.ambiguous_name");
                None
            }
        }
    }

    /// Get agent registration by block ID.
    #[allow(dead_code)]
    pub fn get_agent_by_block(&self, block_id: &str) -> Option<&AgentRegistration> {
        self.agent_info.get(block_id)
    }

    /// Every name `block_id` is bound under (display first, then stable).
    /// Production teardown gets the same list back from [`unregister_block`];
    /// this read-only form is for tests and diagnostics.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn names_for_block(&self, block_id: &str) -> Vec<String> {
        let Some(names) = self.block_names.get(block_id) else {
            return Vec::new();
        };
        let mut out = vec![names.display.clone()];
        if let Some(s) = &names.stable {
            if !out.iter().any(|n| n.eq_ignore_ascii_case(s)) {
                out.push(s.clone());
            }
        }
        out
    }

    /// List all registered agents.
    pub fn list_agents(&self) -> Vec<AgentRegistration> {
        self.agent_info.values().cloned().collect()
    }

    /// Inject a message into an agent's terminal.
    ///
    /// Sends `message\r` as a single payload (required for text display),
    /// then spawns 3 delayed `\r` sends at 200ms intervals as separate
    /// PTY writes to ensure submission. See `docs/specs/jekt-inject-timing.md`.
    pub fn inject_message(&mut self, req: InjectionRequest) -> InjectionResponse {
        self.inject_message_inner(req, None, None, None)
    }

    /// Shared delivery path behind both `inject_message` (ordinary jekt,
    /// every `outcome`/`reason` param always `None`) and
    /// `record_supervisor_decision`'s `Nudge` arm (`outcome_on_success:
    /// Some("nudge_sent")`, `outcome_on_failure: Some("nudge_failed")`,
    /// `reason` the Supervisor's stated reasoning) — every audit-log write
    /// below carries these through instead of the two call sites
    /// duplicating sanitize/deliver logic. Two separate outcome params
    /// (rather than one applied uniformly) so a failed delivery is audited
    /// distinctly from a successful one (reagentx P2 on PR #2557 — the
    /// Supervisor UI's decision feed must not show "nudged" for a delivery
    /// that actually failed).
    fn inject_message_inner(
        &mut self,
        mut req: InjectionRequest,
        outcome_on_success: Option<&str>,
        outcome_on_failure: Option<&str>,
        reason: Option<&str>,
    ) -> InjectionResponse {
        let now = now_unix_millis();

        // Generate request ID if missing
        if req.request_id.is_none() || req.request_id.as_deref() == Some("") {
            req.request_id = Some(uuid::Uuid::new_v4().to_string());
        }
        let request_id = req.request_id.clone().unwrap_or_default();

        // Rate limit check
        if !self.rate_limiter.check() {
            return InjectionResponse {
                success: false,
                request_id,
                block_id: None,
                error: Some("rate limit exceeded".to_string()),
                timestamp: now,
                effective_tier: None,
                requires_stop: None,
                channel_verified: None,
            };
        }

        // Validate agent ID
        if !validate_agent_id(&req.target_agent) {
            return InjectionResponse {
                success: false,
                request_id,
                block_id: None,
                error: Some(format!("invalid agent ID: {}", req.target_agent)),
                timestamp: now,
                effective_tier: None,
                requires_stop: None,
                channel_verified: None,
            };
        }

        // Sanitize message
        let sanitized = sanitize_message(&req.message);

        // Resolve the target (identity M2, spec §4.4.3): UID first, then a
        // name held by exactly one live block. A name held by several is
        // refused WITH the candidates — the only place a collision is
        // visible is the only place it can be resolved (§5.2). The error
        // string deliberately does not begin with "agent not found": that
        // prefix is what `handle_reactive_inject` forwards to other
        // instances, and a local collision must not be forwarded.
        let (block_id, resolved_by_uid) = match self.resolve_target(&req.target_agent) {
            TargetResolution::Uid(b) => (b, true),
            TargetResolution::Name(b) => (b, false),
            TargetResolution::Ambiguous(candidates) => {
                let err = format!(
                    "ambiguous agent name: '{}' is held by {} live agents — address one by uid: {}",
                    req.target_agent,
                    candidates.len(),
                    describe_candidates(&candidates)
                );
                self.log_audit(
                    req.source_agent.as_deref(),
                    &req.target_agent,
                    "",
                    &sanitized,
                    false,
                    Some(&err),
                    &request_id,
                    outcome_on_failure,
                    reason,
                );
                return InjectionResponse {
                    success: false,
                    request_id,
                    block_id: None,
                    error: Some(err),
                    timestamp: now,
                    effective_tier: None,
                    requires_stop: None,
                    channel_verified: None,
                };
            }
            TargetResolution::NotFound => {
                let err = format!("agent not found: {}", req.target_agent);
                self.log_audit(
                    req.source_agent.as_deref(),
                    &req.target_agent,
                    "",
                    &sanitized,
                    false,
                    Some(&err),
                    &request_id,
                    outcome_on_failure,
                    reason,
                );
                return InjectionResponse {
                    success: false,
                    request_id,
                    block_id: None,
                    error: Some(err),
                    timestamp: now,
                    effective_tier: None,
                    requires_stop: None,
                    channel_verified: None,
                };
            }
        };

        // Identity M2: a target that resolved by UID is confirmed by UID
        // only (`uid_identity_confirmer`); `None` is unverifiable and
        // delivery proceeds — see that field's doc comment for the
        // registered-but-unspawned case this protects. The name checks
        // below apply only to targets that resolved by name.
        if resolved_by_uid {
            let own_uid = self
                .uid_identity_confirmer
                .as_ref()
                .and_then(|c| c(&block_id));
            if let Some(own_uid) = own_uid {
                if !own_uid.eq_ignore_ascii_case(&req.target_agent) {
                    let err = format!(
                        "identity mismatch: block {} resolved for uid '{}' but its own uid is '{}'",
                        block_id, req.target_agent, own_uid
                    );
                    tracing::error!(
                        target = %req.target_agent,
                        block_id = %block_id,
                        own_uid = %own_uid,
                        "reactive inject: recipient uid mismatch — rejecting delivery"
                    );
                    self.log_audit(
                        req.source_agent.as_deref(),
                        &req.target_agent,
                        &block_id,
                        &sanitized,
                        false,
                        Some(&err),
                        &request_id,
                        Some("identity-mismatch"),
                        reason,
                    );
                    return InjectionResponse {
                        success: false,
                        request_id,
                        block_id: Some(block_id),
                        error: Some(err),
                        timestamp: now,
                        effective_tier: None,
                        requires_stop: None,
                        channel_verified: None,
                    };
                }
            }
        }

        // Recipient-identity check (issue #2695): before delivering, compare
        // the resolved block's own live, spawn-time-captured identity
        // (queried via `agent_identity_confirmer`, which goes straight to
        // the controller — independent of this handler's own agent_to_block
        // map) against who this jekt is addressed to. Checking agent_to_block
        // against itself would be a tautology and catch nothing; this is a
        // genuine second, independently-written source of truth that can
        // drift from the registry (e.g. a stale registry entry survived a
        // respawn, or a resync overwrote registration without updating the
        // block's own field).
        //
        // `None` (no confirmer configured, or this controller type doesn't
        // implement `agent_id()`) is "unverifiable," not "confirmed absent" —
        // delivery proceeds exactly as before this check existed. Matches
        // this codebase's existing jekt trust philosophy elsewhere (see the
        // TRUST=/ESCALATE= rules): only an ACTIVE mismatch is a red flag,
        // mere absence of proof is not.
        //
        // Checked against BOTH the live identity (`agent_identity_confirmer`)
        // and the stable one (`stable_agent_identity_confirmer`) — a target
        // resolved via the alias registry is legitimately the stable ID, not
        // the live display name, and would otherwise always fail this check
        // once the primary key has moved on post-rename. Either source
        // reporting a match is sufficient; both reporting `None` is
        // "unverifiable" exactly as before.
        if !resolved_by_uid
            && (self.agent_identity_confirmer.is_some()
                || self.stable_agent_identity_confirmer.is_some())
        {
            let live_agent_id = self
                .agent_identity_confirmer
                .as_ref()
                .and_then(|c| c(&block_id));
            let stable_agent_id = self
                .stable_agent_identity_confirmer
                .as_ref()
                .and_then(|c| c(&block_id));
            // Both `None` is "unverifiable" (see above) — only proceed to
            // the check below if at least one source reported an identity.
            if live_agent_id.is_some() || stable_agent_id.is_some() {
                let target_lower = req.target_agent.to_lowercase();
                let matches = |id: &Option<String>| {
                    id.as_deref().is_some_and(|id| id.to_lowercase() == target_lower)
                };
                if !matches(&live_agent_id) && !matches(&stable_agent_id) {
                    let err = format!(
                        "identity mismatch: block {} resolved for target '{}' but its own identity is '{}'",
                        block_id,
                        req.target_agent,
                        live_agent_id
                            .as_deref()
                            .or(stable_agent_id.as_deref())
                            .unwrap_or("<unknown>")
                    );
                    tracing::error!(
                        target = %req.target_agent,
                        block_id = %block_id,
                        live_agent_id = ?live_agent_id,
                        stable_agent_id = ?stable_agent_id,
                        "reactive inject: recipient identity mismatch — rejecting delivery"
                    );
                    self.log_audit(
                        req.source_agent.as_deref(),
                        &req.target_agent,
                        &block_id,
                        &sanitized,
                        false,
                        Some(&err),
                        &request_id,
                        Some("identity-mismatch"),
                        reason,
                    );
                    return InjectionResponse {
                        success: false,
                        request_id,
                        block_id: Some(block_id),
                        error: Some(err),
                        timestamp: now,
                        effective_tier: None,
                        requires_stop: None,
                        channel_verified: None,
                    };
                }
            }
        }

        // Determine effective jekt tier.
        // Escalation rules (spec §5.2, extended by
        // SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md §2.2, by
        // SPEC_JEKT_REAGENT_TRUST_RELAXATION_2026_08_14.md §1 for the
        // network-tier exception below, and NARROWED by
        // SPEC_JEKT_SENSITIVE_TIER_NARROWING_2026_08_15.md — repo-owner-
        // confirmed directly in a live conversation, not a jekt/muxbus claim):
        //   1. WAN or LAN delivery, sender's identity check ACTIVELY FAILED
        //      (a reagent_sig was present but didn't verify — SIG=invalid) →
        //      always SENSITIVE, regardless of declared tier or keyword
        //      content. A real red flag: someone tried to forge a signature.
        //      Mere ABSENCE of any signature attempt (no reagent_sig sent at
        //      all, or one verified only under the known-exposed dev key) is
        //      NOT this case — that's rule 5's default, same as any other
        //      self-declared sender. LAN never carries a signature attempt
        //      at all, so it's never eligible for this rule either — see
        //      rules 3/4 for what still catches a malicious LAN jekt.
        //   1b. WAN delivery, sender verified via reagent's pinned Ed25519
        //      key (`reagent_verified == Some(true)`) → NOT forced to
        //      SENSITIVE by delivery tier alone. As of the 2026-08-15
        //      narrowing this is NOT a distinct check anymore — it's simply
        //      rule 1 not matching (`Some(true)` isn't `Some(false)`), so a
        //      message verified under the trusted `reagent-v1` key and one
        //      verified only under the known-exposed `reagent-v1-dev`
        //      placeholder now get IDENTICAL tier treatment: neither is
        //      forced sensitive. `is_reagent_trusted_signing_key`
        //      (agentmux-common::jekt_sign) is NOT consulted here at all —
        //      unlike before this narrowing, key trust no longer gates
        //      TIER in any way; it still exists for other verification
        //      bookkeeping, just not this decision. Rules 3/4 below still
        //      apply on top: a verified reagent message that declares
        //      SENSITIVE or matches the keyword scan still escalates.
        //   2. Host delivery, sender identity checkable but signature missing
        //      or wrong → always SENSITIVE (host-tier senders can now be
        //      verified when the claimed source_agent has a signing key —
        //      see `sig_verified`'s doc comment on `InjectionRequest` for
        //      exactly when this applies vs. is skipped). Same "an active
        //      verification FAILURE is the red flag, not mere absence of
        //      one" logic as rule 1 — this rule was already scoped that way.
        //   3. Declared SENSITIVE (any tier) → SENSITIVE.
        //   4. Keyword match (any tier) → SENSITIVE.
        //   5. Otherwise → use declared tier (default: coord). This is now
        //      reachable by ordinary unverified LAN/WAN traffic with clean
        //      content — `TRUST` in the marker is UNCHANGED by this
        //      narrowing (still reads `network-claimed`, still exactly as
        //      forgeable as ever); only whether that lack of proof alone
        //      is sufficient grounds to interrupt the human has changed.
        let declared_tier = req.jekt_tier.as_ref();
        let delivery_tier = req.delivery_tier.as_deref().unwrap_or("host");
        let is_network_tier = delivery_tier == "wan" || delivery_tier == "lan";
        // A reagent_sig that was PRESENT but didn't verify — someone tried to
        // forge it. `reagent_verified` is WAN-only by construction
        // (`sync_agent_reactive`/`verify_reagent_signature` never compute it
        // off the WAN tier, so it's always `None` for LAN — reagent is a
        // WAN-only service sender, this never applied to LAN) — absence of a
        // signature attempt (`None`), or a signature that verified but only
        // under the known-exposed dev key, is NOT this case; both fall
        // through to rule 5 like any other self-declared sender.
        let is_network_tier_sig_invalid = is_network_tier && req.reagent_verified == Some(false);
        // A lan_sig that was PRESENT, whose claimed sender's public key WAS
        // found, but didn't cryptographically verify — a specific agent's
        // identity was actively forged, not merely unproven. Scoped to LAN
        // only, same reasoning as reagent's WAN scoping above — see
        // docs/specs/SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md §2.4/§2.5.
        let is_lan_sig_invalid = delivery_tier == "lan" && req.lan_verified == Some(false);
        let is_unverified_sender = req.sig_verified == Some(false);
        // SPEC_JEKT_TRANSCRIPT_REQUEST_TIER_RULES_2026_08_22.md rule 1: a
        // transcript_request (muxspect Phase B/C's LAN/WAN conversation-
        // visibility protocol) is forced sensitive unconditionally, on any
        // tier, regardless of trust — the existing credential/destructive-
        // keyword scan (is_sensitive_message) doesn't catch a content-
        // disclosure *request* at all. `req.is_transcript_request` is
        // server-computed in `handle_reactive_inject` by re-parsing
        // `message` itself (never attacker-settable — see its own doc
        // comment), so this can't be spoofed by a client claiming/denying it.
        let is_sensitive = is_network_tier_sig_invalid
            || is_lan_sig_invalid
            || is_unverified_sender
            || matches!(declared_tier, Some(super::types::JektTier::Sensitive))
            || is_sensitive_message(&sanitized)
            || req.is_transcript_request;
        let effective_tier = if is_sensitive { "sensitive" } else {
            declared_tier.map_or("coord", |t| match t {
                super::types::JektTier::Info => "info",
                super::types::JektTier::Coord => "coord",
                super::types::JektTier::Sensitive => "sensitive",
            })
        };
        // Whether TIER=sensitive should actually STOP the receiving agent and
        // require human confirmation, vs. just carry the tag for visual
        // indication (SPEC_JEKT_SENSITIVE_TIER_VERIFIED_SENDER_NO_STOP_2026_08_17.md,
        // repo-owner-confirmed directly in a live conversation, same channel
        // this policy's own STOP rule already treats as authoritative).
        //
        // Cryptographic proof of identity is exactly what the STOP rule was
        // protecting against the ABSENCE of (the spoofed-jekt-then-spoofed-
        // muxbus-confirmation incident this whole tiering system exists to
        // stop). Once a sender is actually verified — `sig_verified`,
        // `reagent_verified`, or `lan_verified` all being `Some(true)` — that
        // specific attack is no longer possible for this message, regardless
        // of WHY it was marked sensitive (self-declared, or a keyword match
        // on content a genuinely-signed sender is allowed to legitimately
        // discuss, e.g. a code review of credential-handling code).
        //
        // This can never accidentally cover an active-forgery case: the three
        // rules above that force `is_sensitive` from a signature actively
        // failing (`is_network_tier_sig_invalid`, `is_lan_sig_invalid`,
        // `is_unverified_sender`) are each keyed on the SAME field this
        // checks for `Some(true)` reading `Some(false)` instead — the two can
        // never both be true for the same field at once, so a message that
        // reaches STOP-required via one of those three rules is, by
        // construction, never simultaneously "verified" on that same tier.
        //
        // `channel_verified` (SPEC_JEKT_CROSS_CHANNEL_TRUST_2026_09_02.md
        // §D3, Phase B) joins the list on the same terms: a cross-channel
        // signature that verified against the sender's published public
        // key is proof of exactly who sent it. Its `Some(false)` is NOT yet
        // among the forcing rules above — Phase C, deliberately held back
        // until published keys have propagated (spec §6/§10) — so today
        // this field can only ever relax, never escalate.
        let is_cryptographically_verified = req.sig_verified == Some(true)
            || req.reagent_verified == Some(true)
            || req.lan_verified == Some(true)
            || req.channel_verified == Some(true);
        // SPEC_JEKT_TRANSCRIPT_REQUEST_TIER_RULES_2026_08_22.md rule 2: the
        // ONE named exception to the verified-sender relaxation above. A
        // transcript_request's ESCALATE=required is not relaxed by a
        // verified sender when the RESPONDING agent's own
        // conversation_visibility is `ask` (or `trusted_peers` with a
        // non-allow-listed requester) — `req.transcript_request_escalate_forced`,
        // resolved server-side by `resolve_transcript_request_tier_fields`
        // against that agent's own setting, called from both delivery paths
        // that reach `Handler` (`handle_reactive_inject` for
        // host/cross-channel/LAN, `muxbus::cloud_subscriber::sync_agent_reactive`
        // for WAN) — meaningless/always-false unless `is_transcript_request`
        // is also true. A valid signature answers
        // *who is asking*, not *whether this content should be disclosed* —
        // identity proof alone doesn't relax that question, unlike every
        // other TIER=sensitive case above.
        let requires_stop = is_sensitive
            && (!is_cryptographically_verified || req.transcript_request_escalate_forced);
        let priority = req.priority.as_deref().unwrap_or("normal");

        // Wrap in JEKT marker block (structured tag + human-readable header).
        // Note: `req.sig_verified` (three-state) is passed through as-is for
        // the marker's TRUST label — `is_unverified_sender` above only
        // captures the `Some(false)` case (the one that forces SENSITIVE);
        // the marker itself also needs to distinguish `None` ("never
        // checked," e.g. a Slack-bridge message) from `Some(true)`
        // ("actually verified") rather than collapsing both to the same
        // label — see `wrap_jekt_message`'s doc comment.
        let wrapped = wrap_jekt_message(
            &sanitized,
            req.source_agent.as_deref(),
            &req.target_agent,
            effective_tier,
            delivery_tier,
            req.sig_verified,
            req.reagent_verified,
            req.lan_verified,
            req.channel_verified,
            requires_stop,
            &request_id,
            priority,
        );

        // Legacy source-prefix format preserved for PTY controllers that don't
        // parse the JEKT block — the wrap_jekt_message output already includes
        // the source in the human-readable header so format_injected_message
        // is called with include_source=false here.
        let final_msg = format_injected_message(
            &wrapped,
            req.source_agent.as_deref(),
            false,
        );

        // Controller-aware delivery (SPEC_AGENT_CONTROL_PROTOCOL §6 / Phase 3).
        // Persistent (stream-json) and ACP agents have no PTY — their inbox is a
        // structured channel (live stdin NDJSON / `session/prompt`). Delivering there
        // also lands the message mid-turn (steering) instead of waiting for idle.
        // PTY-based shell/term agents report back so we fall through to keystrokes.
        if let Some(ref deliver) = self.message_sender {
            match deliver(&block_id, &final_msg) {
                Ok(true) => {
                    tracing::info!(
                        target_agent = %req.target_agent,
                        block_id = %block_id,
                        "inject: structured delivery to non-PTY controller (mid-turn steer)"
                    );
                    self.log_audit(
                        req.source_agent.as_deref(),
                        &req.target_agent,
                        &block_id,
                        &sanitized,
                        true,
                        None,
                        &request_id,
                        outcome_on_success,
                        reason,
                    );
                    return InjectionResponse {
                        success: true,
                        request_id,
                        block_id: Some(block_id),
                        error: None,
                        timestamp: now,
                        effective_tier: Some(effective_tier.to_string()),
                        requires_stop: Some(requires_stop),
                        channel_verified: req.channel_verified,
                    };
                }
                Ok(false) => {
                    // PTY-based controller — fall through to keystroke injection.
                }
                Err(e) => {
                    // Structured controller but delivery failed (e.g. persistent
                    // process not running). Do NOT fall back to PTY keystrokes — the
                    // persistent controller rejects raw input. Surface the error.
                    tracing::warn!(
                        target_agent = %req.target_agent,
                        block_id = %block_id,
                        error = %e,
                        "inject: structured delivery failed"
                    );
                    self.log_audit(
                        req.source_agent.as_deref(),
                        &req.target_agent,
                        &block_id,
                        &sanitized,
                        false,
                        Some(&e),
                        &request_id,
                        outcome_on_failure,
                        reason,
                    );
                    return InjectionResponse {
                        success: false,
                        request_id,
                        block_id: Some(block_id),
                        error: Some(e),
                        timestamp: now,
                        effective_tier: Some(effective_tier.to_string()),
                        requires_stop: Some(requires_stop),
                        channel_verified: req.channel_verified,
                    };
                }
            }
        }

        // Send message via input sender
        let sender = match &self.input_sender {
            Some(s) => s.clone(),
            None => {
                let err = "input sender not configured".to_string();
                self.log_audit(
                    req.source_agent.as_deref(),
                    &req.target_agent,
                    &block_id,
                    &sanitized,
                    false,
                    Some(&err),
                    &request_id,
                    outcome_on_failure,
                    reason,
                );
                return InjectionResponse {
                    success: false,
                    request_id,
                    block_id: Some(block_id),
                    error: Some(err),
                    timestamp: now,
                    effective_tier: Some(effective_tier.to_string()),
                    requires_stop: Some(requires_stop),
                    channel_verified: req.channel_verified,
                };
            }
        };

        // Jekt inject sequence (see docs/specs/jekt-inject-timing.md):
        // 1. \r to clear any partial input on the line
        // 2. message\r as single payload (proven to display text — v0.31.122/125)
        // 3. Three delayed \r at 200ms intervals as separate PTY writes to submit
        let _ = sender(&block_id, b"\r");
        let payload = format!("{}\r", final_msg);
        tracing::info!(
            target_agent = %req.target_agent,
            block_id = %block_id,
            msg_len = payload.len(),
            "inject: sending payload to PTY"
        );
        if let Err(e) = sender(&block_id, payload.as_bytes()) {
            tracing::error!(
                target_agent = %req.target_agent,
                block_id = %block_id,
                error = %e,
                "inject: sender failed"
            );
            self.log_audit(
                req.source_agent.as_deref(),
                &req.target_agent,
                &block_id,
                &sanitized,
                false,
                Some(&e),
                &request_id,
                outcome_on_failure,
                reason,
            );
            return InjectionResponse {
                success: false,
                request_id,
                block_id: Some(block_id),
                error: Some(e),
                timestamp: now,
                effective_tier: Some(effective_tier.to_string()),
                requires_stop: Some(requires_stop),
                channel_verified: req.channel_verified,
            };
        }

        // Spawn 3 delayed \r sends as separate PTY events to ensure submission.
        let sender_enter = sender.clone();
        let block_id_enter = block_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let _ = sender_enter(&block_id_enter, b"\r");
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let _ = sender_enter(&block_id_enter, b"\r");
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let _ = sender_enter(&block_id_enter, b"\r");
        });

        // Success
        self.log_audit(
            req.source_agent.as_deref(),
            &req.target_agent,
            &block_id,
            &sanitized,
            true,
            None,
            &request_id,
            outcome_on_success,
            reason,
        );

        InjectionResponse {
            success: true,
            request_id,
            block_id: Some(block_id),
            error: None,
            timestamp: now,
            effective_tier: Some(effective_tier.to_string()),
            requires_stop: Some(requires_stop),
            channel_verified: req.channel_verified,
        }
    }

    /// Record a Warden Supervisor watcher agent's decision about
    /// `target_agent`. A `Nudge` is delivered through the same path
    /// `inject_message` uses (`inject_message_inner`) and audited with
    /// `outcome: "nudge_sent"`; a `Decline` sends nothing and is audited
    /// directly with `outcome: "nudge_declined"`.
    ///
    /// Enforces the consecutive-nudge ceiling
    /// (`MAX_CONSECUTIVE_AUTO_CONTINUES`): if this nudge would exceed it,
    /// the decision is forced to a decline (audited with
    /// `outcome: "nudge_declined"`, `reason: "consecutive-nudge ceiling
    /// reached"`) and `Err` is returned so the calling agent's MCP
    /// tool-call result surfaces the refusal directly — a signal for it to
    /// stop nudging and escalate to a human via an ordinary jekt instead of
    /// retrying. The counter resets when the target's `registration_nonce`
    /// or `block_id` changes (a respawn — "consecutive" only makes sense
    /// within one continuous run; see `NudgeCounterState`'s field docs for
    /// why both signals are needed) or after `NUDGE_COOLDOWN_RESET_MS` of
    /// inactivity.
    ///
    /// Does NOT check the target's `auto_continue_enabled` opt-in itself —
    /// `Handler` has no `Store` access by design (this module doesn't
    /// depend on `backend::storage`). That gate lives at the HTTP boundary,
    /// in `handle_reactive_supervisor_decision`
    /// (`server/reactive.rs`), which has `AppState::mstore`. Any other
    /// caller of this method directly is responsible for its own
    /// entitlement check first.
    pub fn record_supervisor_decision(
        &mut self,
        target_agent: &str,
        action: SupervisorAction,
        reason: &str,
        request_id: &str,
        source_agent: Option<&str>,
    ) -> Result<InjectionResponse, String> {
        let now = now_unix_millis();
        let target_key = target_agent.to_lowercase();
        // Same resolution as delivery (identity M2, §10 constraint 4 — this
        // is the site #3520 converted the others around and missed). An
        // ambiguous or unknown target leaves `block_id` empty exactly as an
        // unknown one did before.
        let block_id = match self.resolve_target(target_agent) {
            TargetResolution::Uid(b) | TargetResolution::Name(b) => b,
            TargetResolution::Ambiguous(_) | TargetResolution::NotFound => String::new(),
        };

        match action {
            SupervisorAction::Decline => {
                self.log_audit(
                    source_agent,
                    target_agent,
                    &block_id,
                    "",
                    true,
                    None,
                    request_id,
                    Some("nudge_declined"),
                    Some(reason),
                );
                Ok(InjectionResponse {
                    success: true,
                    request_id: request_id.to_string(),
                    block_id: if block_id.is_empty() { None } else { Some(block_id) },
                    error: None,
                    timestamp: now,
                    effective_tier: None,
                    requires_stop: None,
                    channel_verified: None,
                })
            }
            SupervisorAction::Nudge => {
                // Bound `nudge_counters`' growth (reagentx P2 on PR #2557):
                // an entry idle past the cooldown window is about to be
                // treated as stale on next use anyway, so dropping it here
                // is behavior-neutral — just reclaims memory instead of
                // accumulating one entry per distinct target agent forever.
                self.nudge_counters
                    .retain(|_, v| now.saturating_sub(v.last_nudge_at_ms) <= NUDGE_COOLDOWN_RESET_MS);

                let current_nonce = self
                    .agent_info
                    .get(&block_id)
                    .map(|info| info.registration_nonce)
                    .unwrap_or(0);

                // Scoped borrow: check/reset staleness and read the
                // pre-delivery count, then drop the borrow before calling
                // `inject_message_inner` (which needs `&mut self` too).
                let count_before = {
                    let entry = self.nudge_counters.entry(target_key.clone()).or_insert(NudgeCounterState {
                        registration_nonce: current_nonce,
                        block_id: block_id.clone(),
                        count: 0,
                        last_nudge_at_ms: 0,
                    });
                    // registration_nonce only distinguishes a respawn for
                    // persistent-controller agents (real nonces, ≥ 1);
                    // PTY/shell/HTTP-register paths always register with 0,
                    // so block_id is the fallback signal for those
                    // (reagentx P1 on PR #2557 — nonce alone never fired
                    // for them, leaving a respawned PTY agent stuck behind
                    // its prior run's exhausted ceiling for up to the full
                    // cooldown window).
                    let stale = entry.registration_nonce != current_nonce
                        || entry.block_id != block_id
                        || now.saturating_sub(entry.last_nudge_at_ms) > NUDGE_COOLDOWN_RESET_MS;
                    if stale {
                        entry.registration_nonce = current_nonce;
                        entry.block_id = block_id.clone();
                        entry.count = 0;
                    }
                    entry.count
                };

                if count_before >= MAX_CONSECUTIVE_AUTO_CONTINUES {
                    let ceiling_reason = "consecutive-nudge ceiling reached".to_string();
                    self.log_audit(
                        source_agent,
                        target_agent,
                        &block_id,
                        "",
                        true,
                        None,
                        request_id,
                        Some("nudge_declined"),
                        Some(&ceiling_reason),
                    );
                    return Err(ceiling_reason);
                }

                // Fixed, narrow continuation template — deliberately NOT
                // free-form text composed by the calling Supervisor agent.
                // ANALYSIS_WARDEN_AUTO_CONTROLLER_CONTINUATION_WATCHER_
                // 2026_08_12.md §4.3: "The nudge text should be a fixed,
                // narrow template... not a free-form instruction the
                // watcher composes per-situation — this is the direct
                // mitigation for consent-chain degradation." (reagentx P1
                // on PR #2557 — SupervisorNudge used to accept arbitrary
                // `message` text.) `reason` (the Supervisor's own
                // reasoning) still travels separately, into the audit log
                // only — never into what's delivered to the target.
                let req = InjectionRequest {
                    target_agent: target_agent.to_string(),
                    message: NUDGE_MESSAGE.to_string(),
                    source_agent: source_agent.map(|s| s.to_string()),
                    request_id: Some(request_id.to_string()),
                    priority: Some("normal".to_string()),
                    wait_for_idle: false,
                    jekt_tier: Some(super::types::JektTier::Coord),
                    delivery_tier: Some("host".to_string()),
                    forward_hops: 0,
                    ..Default::default()
                };
                let resp = self.inject_message_inner(
                    req,
                    Some("nudge_sent"),
                    Some("nudge_failed"),
                    Some(reason),
                );

                // Only a successful delivery consumes the ceiling
                // (reagentx P2 on PR #2557) — a rate-limited/unavailable-
                // controller failure shouldn't cost the Supervisor one of
                // its 5 consecutive attempts for this target.
                if resp.success {
                    if let Some(entry) = self.nudge_counters.get_mut(&target_key) {
                        entry.count += 1;
                        entry.last_nudge_at_ms = now;
                    }
                }

                Ok(resp)
            }
        }
    }

    /// Get audit log entries, most recent first.
    pub fn get_audit_log(&self, limit: usize) -> Vec<AuditLogEntry> {
        let start = if self.audit_log.len() > limit {
            self.audit_log.len() - limit
        } else {
            0
        };
        let mut entries: Vec<_> = self.audit_log[start..].to_vec();
        entries.reverse();
        entries
    }

    /// Add an entry to the audit ring buffer. `outcome`/`reason` are `None`
    /// for every ordinary jekt injection (all current call sites) — only
    /// Warden Supervisor decisions (see `record_supervisor_decision`) ever
    /// set them.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn log_audit(
        &mut self,
        source_agent: Option<&str>,
        target_agent: &str,
        block_id: &str,
        message: &str,
        success: bool,
        error_message: Option<&str>,
        request_id: &str,
        outcome: Option<&str>,
        reason: Option<&str>,
    ) {
        let entry = AuditLogEntry {
            timestamp: now_unix_millis(),
            source_agent: source_agent.map(|s| s.to_string()),
            target_agent: target_agent.to_string(),
            block_id: block_id.to_string(),
            message_hash: sha256_hex(message),
            message_length: message.len(),
            success,
            error_message: error_message.map(|s| s.to_string()),
            request_id: request_id.to_string(),
            outcome: outcome.map(|s| s.to_string()),
            reason: reason.map(|s| s.to_string()),
            event_kind: "delivery".to_string(),
            evicted_block: None,
            evicted_agent: None,
        };

        if self.audit_log.len() >= AUDIT_LOG_MAX {
            self.audit_log.remove(0);
        }
        self.audit_log.push(entry);
    }

    /// Record a registration/eviction event in the audit log — the
    /// counterpart to [`log_audit`] (which only ever recorded delivery
    /// attempts). `evicted_block`/`evicted_agent` capture whichever prior
    /// mapping a "register" call displaced, so a same-host identity
    /// collision (two panes racing to register the same agent_id) is
    /// reconstructable after the fact instead of leaving no trace.
    pub(super) fn log_audit_registration(
        &mut self,
        event_kind: &str,
        agent_id: &str,
        block_id: &str,
        evicted_block: Option<&str>,
        evicted_agent: Option<&str>,
    ) {
        let entry = AuditLogEntry {
            timestamp: now_unix_millis(),
            source_agent: None,
            target_agent: agent_id.to_string(),
            block_id: block_id.to_string(),
            message_hash: String::new(),
            message_length: 0,
            success: true,
            error_message: None,
            request_id: String::new(),
            outcome: None,
            reason: None,
            event_kind: event_kind.to_string(),
            evicted_block: evicted_block.map(|s| s.to_string()),
            evicted_agent: evicted_agent.map(|s| s.to_string()),
        };

        if self.audit_log.len() >= AUDIT_LOG_MAX {
            self.audit_log.remove(0);
        }
        self.audit_log.push(entry);
    }
}

impl Default for Handler {
    fn default() -> Self {
        Self::new()
    }
}

// ---- Thread-safe wrapper ----

/// Thread-safe wrapper around Handler.
pub struct ReactiveHandler {
    inner: Mutex<Handler>,
}

impl ReactiveHandler {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Handler::new()),
        }
    }

    pub fn set_input_sender(&self, sender: InputSender) {
        self.inner.lock().unwrap().set_input_sender(sender);
    }

    pub fn set_message_sender(&self, sender: MessageSender) {
        self.inner.lock().unwrap().set_message_sender(sender);
    }

    pub fn set_agent_identity_confirmer(&self, confirmer: AgentIdentityConfirmer) {
        self.inner
            .lock()
            .unwrap()
            .set_agent_identity_confirmer(confirmer);
    }

    /// See the inner [`Handler::set_stable_agent_identity_confirmer`].
    pub fn set_stable_agent_identity_confirmer(&self, confirmer: AgentIdentityConfirmer) {
        self.inner
            .lock()
            .unwrap()
            .set_stable_agent_identity_confirmer(confirmer);
    }

    /// See the inner [`Handler::set_uid_identity_confirmer`] (identity M2).
    pub fn set_uid_identity_confirmer(&self, confirmer: AgentIdentityConfirmer) {
        self.inner
            .lock()
            .unwrap()
            .set_uid_identity_confirmer(confirmer);
    }

    /// See the inner [`Handler::set_block_liveness`] (identity M2).
    pub fn set_block_liveness(&self, probe: BlockLivenessProbe) {
        self.inner.lock().unwrap().set_block_liveness(probe);
    }

    /// See the inner [`Handler::register_agent_full`] (identity M2).
    #[allow(clippy::too_many_arguments)]
    pub fn register_agent_full(
        &self,
        display: &str,
        block_id: &str,
        tab_id: Option<&str>,
        registration_nonce: u64,
        stable: Option<&str>,
        uid: Option<&str>,
        no_uid_counter: &'static str,
    ) -> Result<(), String> {
        self.inner.lock().unwrap().register_agent_full(
            display,
            block_id,
            tab_id,
            registration_nonce,
            stable,
            uid,
            no_uid_counter,
        )
    }

    /// [`try_register_agent_with_nonce`]'s full-identity form — same
    /// non-blocking, bounded-retry lock discipline, for the spawn path.
    #[allow(clippy::too_many_arguments)]
    pub fn try_register_agent_full(
        &self,
        display: &str,
        block_id: &str,
        tab_id: Option<&str>,
        registration_nonce: u64,
        stable: Option<&str>,
        uid: Option<&str>,
        no_uid_counter: &'static str,
    ) -> Result<(), String> {
        let mut guard = self.try_lock_bounded()?;
        guard.register_agent_full(
            display,
            block_id,
            tab_id,
            registration_nonce,
            stable,
            uid,
            no_uid_counter,
        )
    }

    /// Every name `block_id` is bound under — see [`Handler::names_for_block`].
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn names_for_block(&self, block_id: &str) -> Vec<String> {
        self.inner.lock().unwrap().names_for_block(block_id)
    }

    /// Bounded `try_lock`, shared by the spawn-path entry points — see
    /// [`try_register_agent_with_nonce`]'s doc comment for the reasoning.
    fn try_lock_bounded(&self) -> Result<std::sync::MutexGuard<'_, Handler>, String> {
        const MAX_ATTEMPTS: u32 = 5;
        const RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(2);
        for attempt in 1..=MAX_ATTEMPTS {
            match self.inner.try_lock() {
                Ok(guard) => return Ok(guard),
                Err(std::sync::TryLockError::WouldBlock) => {
                    if attempt == MAX_ATTEMPTS {
                        return Err(
                            "reactive handler lock busy after retrying — skipping \
                             registration (either a same-thread reentrant call, \
                             redundant by construction on that path, or contention \
                             that outlasted the retry budget; see \
                             INCIDENT_2026_09_07_BACKEND_UPTIME_TIMER_FROZEN.md)"
                                .to_string(),
                        );
                    }
                    std::thread::sleep(RETRY_DELAY);
                }
                Err(std::sync::TryLockError::Poisoned(e)) => {
                    return Err(format!("reactive handler mutex poisoned: {e}"));
                }
            }
        }
        unreachable!("loop always returns on its final attempt");
    }

    #[allow(dead_code)]
    pub fn set_include_source(&self, include: bool) {
        self.inner.lock().unwrap().set_include_source(include);
    }

    /// Test-facing name-only form — see [`Handler::register_agent`].
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn register_agent(
        &self,
        agent_id: &str,
        block_id: &str,
        tab_id: Option<&str>,
    ) -> Result<(), String> {
        self.inner
            .lock()
            .unwrap()
            .register_agent(agent_id, block_id, tab_id)
    }

    /// See the inner [`Handler::register_agent_with_nonce`].
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn register_agent_with_nonce(
        &self,
        agent_id: &str,
        block_id: &str,
        tab_id: Option<&str>,
        registration_nonce: u64,
        alias: Option<&str>,
    ) -> Result<(), String> {
        self.inner
            .lock()
            .unwrap()
            .register_agent_with_nonce(agent_id, block_id, tab_id, registration_nonce, alias)
    }

    /// Like [`register_agent_with_nonce`], but never blocks: if the lock is
    /// already held, this returns an error immediately instead of waiting
    /// for it.
    ///
    /// The case that matters is this exact call running on the same OS
    /// thread as an in-flight `inject_message` — the reactive-delivery spawn
    /// fallback (`bootstrap.rs`'s `install_agent_turn_delivery`) runs a
    /// respawn synchronously on the injecting thread via `block_in_place`,
    /// and that respawn's own auto-registration (this method's caller,
    /// `PersistentSubprocessController::spawn_process`) used to re-lock this
    /// same non-reentrant `Mutex<Handler>` two modules away from the
    /// call site `TurnRegistration::Skip` actually guards — permanently
    /// deadlocking the reactive handler process-wide (every persistent
    /// agent's first message after an idle period, or after any srv
    /// restart). See `docs/incident/INCIDENT_2026_09_07_BACKEND_UPTIME_TIMER_FROZEN.md`.
    ///
    /// Skipping in that case loses nothing: `inject_message` only reaches
    /// the spawn fallback after resolving `target_agent` through
    /// `agent_to_block`, so this exact agent/block pair is registered BY
    /// CONSTRUCTION before this call is ever reached on that path — the
    /// same "redundant, not just deadlock-prone" reasoning documented on
    /// `TurnRegistration::Skip` for the sibling re-lock it guards.
    ///
    /// Genuine cross-thread contention is a real, separate risk this
    /// method does not get to treat as harmless: `respawn_once_for_
    /// leftover_queue` and other internal callers reach `spawn_process`
    /// directly, with no `run_agent_turn` `Register`-tail behind them to
    /// repair a skip — a losing race there can leave a live respawned
    /// agent unregistered and unreachable until a human intervenes via
    /// the UI (reagent P1 on PR #3084's review). The retry loop below
    /// exists for exactly that case: a same-thread reentrant call can
    /// never succeed no matter how many times it retries (nothing on
    /// that call stack can release the lock), but ordinary cross-thread
    /// contention — a few HashMap reads under the same lock elsewhere,
    /// typically microseconds — very likely clears within the retry
    /// budget. See the loop's own comment for the full reasoning.
    ///
    /// A skip's caller (`spawn_process`) is also responsible for not
    /// trusting `registration_nonce` as this spawn's exit-time cleanup
    /// key when the call returns `Err` here — the registration this
    /// skip left in place (if any) still belongs to whichever nonce is
    /// actually on record, not to this spawn's own unwritten one. See
    /// `spawn_process`'s handling of this method's `Err` arm.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn try_register_agent_with_nonce(
        &self,
        agent_id: &str,
        block_id: &str,
        tab_id: Option<&str>,
        registration_nonce: u64,
        alias: Option<&str>,
    ) -> Result<(), String> {
        // Bounded retry, not a single attempt (reagent P1 on PR #3084 —
        // review of INCIDENT_2026_09_07_BACKEND_UPTIME_TIMER_FROZEN.md's
        // fix): this method has exactly one call site
        // (`PersistentSubprocessController::spawn_process`'s
        // auto-registration), shared by every caller that spawns a
        // process — including `respawn_once_for_leftover_queue`, which
        // calls `spawn_process` directly with no `run_agent_turn`
        // `Register`-tail to repair a skip. A single `try_lock` cannot
        // tell "the SAME thread already holds this lock" (the reentrant
        // deadlock this method exists to avoid — retrying never helps,
        // nothing on this call stack can release it) apart from "a
        // DIFFERENT thread holds it briefly for an unrelated op" (e.g.
        // `list_agents()` — a few HashMap reads, typically microseconds).
        // Retrying costs the reentrant case a few bounded, wasted
        // milliseconds and still correctly returns `Err`; it costs the
        // ordinary-contention case nothing in the near-universal case
        // where the next attempt lands after the brief holder is done —
        // "bounded stalling beats silent loss", the same tradeoff
        // `bootstrap.rs`'s `block_in_place` fallback already makes.
        let mut guard = self.try_lock_bounded()?;
        guard.register_agent_with_nonce(agent_id, block_id, tab_id, registration_nonce, alias)
    }

    /// See the inner [`Handler::unregister_agent`] — by name, refusing an
    /// ambiguous one (identity M2).
    pub fn unregister_agent(&self, agent_id: &str) -> UnregisterOutcome {
        self.inner.lock().unwrap().unregister_agent(agent_id)
    }

    /// See the inner [`Handler::unregister_block`]. Returns the names the
    /// block was bound under.
    pub fn unregister_block(&self, block_id: &str) -> Vec<String> {
        self.inner.lock().unwrap().unregister_block(block_id)
    }

    /// See the inner [`Handler::unregister_block_if_nonce`] — atomic
    /// compare-and-remove under the handler lock (issue #2363).
    pub fn unregister_block_if_nonce(&self, block_id: &str, expected_nonce: u64) -> bool {
        self.inner
            .lock()
            .unwrap()
            .unregister_block_if_nonce(block_id, expected_nonce)
    }

    #[allow(dead_code)]
    pub fn update_last_seen(&self, agent_id: &str) {
        self.inner.lock().unwrap().update_last_seen(agent_id);
    }

    pub fn get_agent(&self, agent_id: &str) -> Option<AgentRegistration> {
        self.inner.lock().unwrap().get_agent(agent_id).cloned()
    }

    /// Used by `server/app_api/fleet.rs`'s ordinary (non-reentrant) request
    /// handlers — no longer dead code, but NOT safe to call from
    /// `spawn_process`'s skip-arm; see [`try_get_agent_by_block`] for why.
    pub fn get_agent_by_block(&self, block_id: &str) -> Option<AgentRegistration> {
        self.inner
            .lock()
            .unwrap()
            .get_agent_by_block(block_id)
            .cloned()
    }

    /// Non-blocking counterpart to [`get_agent_by_block`], for the one
    /// caller that can be running on a thread already holding this lock:
    /// `spawn_process`'s handling of a skipped registration
    /// (`try_register_agent_with_nonce` returning `Err`).
    ///
    /// reagent P0 on PR #3084 (caught twice — the fix for the P1 nonce-leak
    /// finding introduced this exact regression on its first attempt): that
    /// fix originally called the plain, blocking [`get_agent_by_block`]
    /// from inside the skip arm. In the routine reentrant case — the same
    /// thread already holds `self.inner` via an in-flight `inject_message`
    /// — that blocking `.lock()` reproduces INCIDENT_2026_09_07's exact
    /// permanent self-deadlock, on the very thread `try_register_agent_
    /// with_nonce`'s own retry-then-fail was designed to protect. Same
    /// bounded-retry reasoning as that method: a same-thread reentrant call
    /// can never succeed (nothing on this call stack releases the lock) and
    /// correctly returns `None` after exhausting the budget — the caller's
    /// nonce-leak mitigation simply doesn't apply in that sub-case, which is
    /// a strict improvement on the pre-this-PR baseline (no cleanup, but no
    /// deadlock either), not a regression. Ordinary cross-thread contention
    /// gets a real chance to succeed, same as the sibling method.
    pub fn try_get_agent_by_block(&self, block_id: &str) -> Option<AgentRegistration> {
        const MAX_ATTEMPTS: u32 = 5;
        const RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(2);
        for attempt in 1..=MAX_ATTEMPTS {
            match self.inner.try_lock() {
                Ok(guard) => return guard.get_agent_by_block(block_id).cloned(),
                Err(std::sync::TryLockError::WouldBlock) => {
                    if attempt == MAX_ATTEMPTS {
                        return None;
                    }
                    std::thread::sleep(RETRY_DELAY);
                }
                Err(std::sync::TryLockError::Poisoned(_)) => return None,
            }
        }
        unreachable!("loop always returns on its final attempt");
    }

    pub fn list_agents(&self) -> Vec<AgentRegistration> {
        self.inner.lock().unwrap().list_agents()
    }

    pub fn inject_message(&self, req: InjectionRequest) -> InjectionResponse {
        self.inner.lock().unwrap().inject_message(req)
    }

    pub fn get_audit_log(&self, limit: usize) -> Vec<AuditLogEntry> {
        self.inner.lock().unwrap().get_audit_log(limit)
    }

    /// Records an action that isn't a jekt injection (fleet bulk-stop,
    /// pane-lifecycle close/etc. — `agentmux-srv/src/server/app_api/{fleet,
    /// pane}.rs`) into the SAME audit ring buffer, so it shows up in
    /// Warden's Audit tab exactly like an ordinary injection
    /// (`SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md` §6 — fleet actions get
    /// visibility there without Warden owning any new code). `action` fills
    /// the slot `log_audit` normally uses for the injected message text
    /// (e.g. `"fleet.bulk-stop"`, `"pane.close"`); `target_agent` is the
    /// resolved agent name for `block_id` when known, else `block_id` itself
    /// (an unregistered/already-stopped block has no agent to name).
    ///
    /// `reason`: an optional caller-supplied free-text note (e.g.
    /// `ClosePane`'s `reason` field, SPEC_AGENT_PANE_LIFECYCLE_CONTROL_2026_09_10.md
    /// §5.2). `None` for every pre-existing caller (`FleetBulkStop` has no
    /// such field today) — purely additive, no behavior change for them.
    #[allow(clippy::too_many_arguments)]
    pub fn log_fleet_action_audit(
        &self,
        source_agent: Option<&str>,
        target_agent: &str,
        block_id: &str,
        action: &str,
        success: bool,
        error_message: Option<&str>,
        request_id: &str,
        reason: Option<&str>,
    ) {
        self.inner.lock().unwrap().log_audit(
            source_agent,
            target_agent,
            block_id,
            action,
            success,
            error_message,
            request_id,
            None,
            reason,
        );
    }

    /// See the inner [`Handler::record_supervisor_decision`].
    pub fn record_supervisor_decision(
        &self,
        target_agent: &str,
        action: SupervisorAction,
        reason: &str,
        request_id: &str,
        source_agent: Option<&str>,
    ) -> Result<InjectionResponse, String> {
        self.inner
            .lock()
            .unwrap()
            .record_supervisor_decision(target_agent, action, reason, request_id, source_agent)
    }
}

impl Default for ReactiveHandler {
    fn default() -> Self {
        Self::new()
    }
}

/// Global reactive handler singleton.
static GLOBAL_HANDLER: OnceLock<ReactiveHandler> = OnceLock::new();

/// Get or initialize the global reactive handler.
pub fn get_global_handler() -> &'static ReactiveHandler {
    GLOBAL_HANDLER.get_or_init(ReactiveHandler::new)
}
