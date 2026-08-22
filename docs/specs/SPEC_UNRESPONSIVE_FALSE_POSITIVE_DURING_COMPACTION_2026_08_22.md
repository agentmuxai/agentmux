# Spec: Stop "Agent unresponsive" From Firing During Legitimate Context Compaction

**Date:** 2026-08-22
**Repo:** agentmuxai/agentmux
**Trigger:** operator report — the "Agent unresponsive" banner (with its "Restart" recovery action) appears in the agent pane during context compaction, and "other times it's not supposed to."

**Implementation status:** Not started. This document proposes the design; it does not change code.

---

## 1. Problem

`HealthMonitor` (`agentmux-srv/src/backend/blockcontroller/health.rs`) classifies an active turn as `Dead` — and publishes an `AgentFailure` with `code: "unresponsive"`, `title: "Agent unresponsive"` (`publish_unresponsive_failure`) — whenever no "meaningful" NDJSON line has arrived from the CLI subprocess for `DEAD_SECS` (120s). The frontend (`useAgentFailure.ts` → `failure-accessory.ts`) renders this as a dismissible banner offering a **Restart** action that kills and respawns the controller process.

This detector was built (`docs/reports/REPORT_WORKING_STATE_REGRESSION_AND_STUCK_QUESTION_PANEL_2026_07_27.md` §2.7/§4, 2026-07-27) specifically to catch a **genuinely wedged CLI subprocess** — one that has stopped producing any output at all while still holding the process alive. It is purely a silence timer: it has no concept of *why* the process is silent, only *how long*.

Three weeks later, `docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md` shipped real compaction visibility (Tiers 1-3, most recently 2026-08-22) — a `PreCompact` hook fires the instant Claude Code begins compacting, and a `compact_boundary` NDJSON frame arrives once it finishes, carrying the real elapsed duration. **The spec's own captured real-world example records a compaction that took 231.6 seconds** — very nearly double `DEAD_SECS`. During that entire window, Claude Code emits **zero** NDJSON output (confirmed by the same spec's own Tier 4 section: "Compaction is a single opaque LLM call producing a summary; there is no intermediate progress event at the protocol level ... Claude Code's own interactive TUI shows only a spinner + elapsed time during compaction, never a percentage").

These two features were built independently, three weeks apart, and never made aware of each other:

- The frontend's own pane state (`agent-pane-state/reducer.ts`) tracks a `compacting` flag and shows a "Compacting… Ns" readout in `AgentComposerStrip.tsx` — it correctly knows nothing is wrong.
- `HealthMonitor`, running in the same `agentmux-srv` process, has **no visibility into that flag at all**. It only watches raw stdout bytes. Once `last_meaningful_ts` is more than 120s stale — which any auto-compaction on a large context routinely exceeds — it transitions to `Dead` and fires the failure banner, **while the pane is simultaneously showing "Compacting…" right above it.**

This is a straightforward, structural gap, not a mystery: the health monitor was never told compaction is a valid reason for extended silence.

### 1.1 "Other times it's not supposed to" — scope of this document

The operator's report also names other, unspecified occasions. Investigation for this spec found **no second confirmed cause** — no other code path in this repo currently tells `HealthMonitor` about a legitimate long-silence operation, so nothing else *could* be suppressing a false positive today. The most likely additional candidate, based on what's currently in the repo, is a long-running `Task`/subagent dispatch (`docs/specs/SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md` / `docs/reports/REPORT_AGENT_TRANSCRIPT_DISPATCH_CARDS_2026_08_19.md`, merged 2026-08-20) — a parent turn blocked on a `Task` tool call could plausibly also go silent on its own stdout for an extended stretch while the sub-dispatch runs. **This is speculative, not verified** — unlike compaction, there is no captured real-world duration and no structural reason yet to believe a hung `Task` tool call should be treated differently from a hung `Bash` call (the original detector's whole purpose). Recommend confirming with `muxlog srv grep "agent health transition"` whether `Dead` transitions correlate with `Task` dispatches before deciding whether they need the same treatment, or whether flagging a slow dispatch really is correct behavior. §4 below is deliberately scoped to the confirmed compaction cause only, but designed so a second confirmed cause is cheap to add later without redesigning the mechanism.

## 2. Root cause, precisely

`compute_health()` (`health.rs`):

```rust
// Check output silence
let silence = inner.last_meaningful_ts.elapsed();
if silence > Duration::from_secs(Self::DEAD_SECS) {      // 120s
    return AgentHealth::Dead;
}
if silence > Duration::from_secs(Self::STALL_SECS) {     // 30s
    return AgentHealth::Stalled;
}
```

`last_meaningful_ts` only advances when `record_output(meaningful: true)` is called, which only happens when a line actually arrives on stdout. During compaction, no line arrives — `PreCompact` fires over a **separate channel** (an HTTP POST from the standalone `agentmux-bashwrap precompact` process to `/agentmux/wps/publish`, event `compaction_started`; see `agentmux-bashwrap/src/precompact.rs` / `wps_client.rs`), and `compact_boundary` arrives on stdout **only after compaction finishes** — i.e. after the false positive would already have fired.

Concretely, on a session whose auto-compaction takes ~230s (the spec's own real example): `Healthy → Stalled` at +30s, `Stalled → Dead` at +120s (publishing "Agent unresponsive" / Restart), then ~110s later the real `compact_boundary` frame lands, is classified as "meaningful" by `classify_output_line`'s default arm (it doesn't special-case `type: "system"`), and `record_output(true)` silently self-heals `Dead → Healthy` (`clear_unresponsive_failure`, per the existing self-heal handling built for a *different* reason in the 07-27 report). **The banner appears and then disappears on its own** roughly 90-100 seconds later, mid-compaction, for no actual problem — exactly matching the report.

## 3. Design

### 3.1 New primitive: a suppressible/extendable silence timer, not a compaction special case

Add a `compacting: bool` (plus `compacting_started_ts: Option<Instant>`) to `HealthMonitorInner`, and a new public method:

```rust
/// Suspend the normal Stalled/Dead silence thresholds — a known-legitimate
/// long-silence operation (currently: context compaction) is in progress.
/// A separate, more generous ceiling (`COMPACTING_DEAD_SECS`) still applies,
/// so a compaction that itself hangs is eventually caught.
pub fn set_compacting(&self, compacting: bool) { ... }
```

`compute_health()` gains one branch, evaluated before the existing `STALL_SECS`/`DEAD_SECS` checks:

```rust
if inner.active_turn {
    if inner.compacting {
        let compacting_silence = inner.compacting_started_ts
            .map(|t| t.elapsed())
            .unwrap_or_default();
        if compacting_silence > Duration::from_secs(Self::COMPACTING_DEAD_SECS) {
            return AgentHealth::Dead; // compaction itself has hung — still worth flagging
        }
        return AgentHealth::Healthy;
    }
    // ... existing STALL_SECS / DEAD_SECS checks, unchanged
}
```

Named and scoped as a general "suppress the silence timer for a confirmed reason" primitive (not `set_compacting_because_reasons`) so a second confirmed cause (§1.1) is a second call site, not a redesign. Deliberately **not** introducing a new `AgentHealth::Compacting` enum variant for v1 — `AgentHealthEvent`'s own comment already notes it "has no frontend subscriber" today, so there's no consumer that benefits from a finer-grained state, and it avoids touching the `AgentHealth::as_str()`/`make_detail()` match arms and any downstream type unions for zero user-visible gain. Revisit only if `agenthealth` gets a real subscriber later.

**`COMPACTING_DEAD_SECS` — open question for the operator to confirm before implementation.** Proposed default: 600s (10 minutes) — roughly 2.6× the one real captured duration (231.6s), generous enough to comfortably cover a much larger context's compaction without being effectively infinite.

### 3.2 Wiring "compaction started" → `set_compacting(true)`

`agentmux-bashwrap precompact` already POSTs `{event: "compaction_started", scopes: ["block:<id>"], persist: 0, data: {...}}` to `/agentmux/wps/publish`, handled generically by `handle_wps_publish` (`agentmux-srv/src/server/mod.rs:743`), which today just forwards to `state.broker.publish(...)` with no per-event special-casing.

Add a narrow special case in that handler: when `req.event == "compaction_started"`, parse the block id out of `req.scopes` (`"block:<id>"`, the same convention `health.rs` itself publishes under), look it up via the existing global registry `blockcontroller::get_controller(block_id)` (`agentmux-srv/src/backend/blockcontroller/mod.rs:264`), and call into its health monitor. This requires one small addition to the `Controller` trait:

```rust
/// This controller's health monitor, if it owns one. Default `None` —
/// only controller types wired to a `HealthMonitor` override this
/// (persistent, host_spawn/subprocess, container_spawn). Mirrors the
/// existing `agent_id()` default-None pattern on this trait.
fn health_monitor(&self) -> Option<Arc<HealthMonitor>> {
    None
}
```

The generic publish path stays generic and cheap for every other event (a string compare + a registry lookup only for this one event name); nothing about the WPS publish/broadcast behavior itself changes, so the frontend's own `compaction_started` subscription (`useCompactionStream.ts`) is unaffected.

### 3.3 Wiring "compaction ended" → `set_compacting(false)`

`compact_boundary` arrives as a normal line on the same stdout stream `classify_output_line`/`record_output` already read from. Three call sites already parse each raw JSON line for exactly this purpose and would each gain one check immediately alongside the existing one:

- `agentmux-srv/src/backend/blockcontroller/persistent.rs:2581-2582`
- `agentmux-srv/src/backend/blockcontroller/subprocess/host_spawn.rs:317-318`
- `agentmux-srv/src/backend/blockcontroller/subprocess/container_spawn.rs:470-471`

```rust
let (meaningful, _error) = classify_output_line(&parsed);
if parsed.get("type").and_then(|v| v.as_str()) == Some("system")
    && parsed.get("subtype").and_then(|v| v.as_str()) == Some("compact_boundary")
{
    health_read.set_compacting(false);
}
health_read.record_output(meaningful);
```

`acp.rs` is deliberately **not** touched — it never classifies raw JSON at all (`record_output(true)` unconditionally per its own comment) and ACP is not the Claude Code stream-json path the compaction feature targets, matching the original compaction spec's own explicit provider scoping (§5: "Doesn't fix: other providers").

**Safety net if `compact_boundary` never arrives** (compaction hook fired but the process then genuinely died, or the CLI crashed mid-compaction): covered by §3.1's `COMPACTING_DEAD_SECS` ceiling — `compacting` never gets cleared, so after 10 minutes it still correctly reaches `Dead`, unlike leaving `compacting` permanently sticky.

### 3.4 What does NOT change

- `STALL_SECS`/`DEAD_SECS` (30s/120s) for the non-compacting case — untouched. This is not a general loosening of the unresponsive detector, only a scoped exemption for a specific, protocol-confirmed legitimate cause.
- The frontend's existing `compacting` pane flag / "Compacting… Ns" UX (§4.2-4.3 of the compaction spec) — untouched; this fix operates one layer down, in the backend health monitor, and the two states simply stop contradicting each other.
- `classify_output_line`'s own classification of `compact_boundary` as "meaningful" (§2's self-heal path) — still correct and still fires; it's just no longer the *only* thing standing between a long compaction and a spurious banner.

## 4. Rollout shape

Additive and low-risk:
- New `HealthMonitorInner` fields default `false`/`None` — a controller that never calls `set_compacting` behaves byte-for-byte as today.
- New `Controller::health_monitor()` trait method has a default (`None`) — every controller type that doesn't override it is unaffected; adding the override to three concrete types is mechanical.
- `handle_wps_publish`'s new special case only touches behavior for `req.event == "compaction_started"`; every other event's publish path is unchanged.

## 5. Test plan (for whoever implements this)

- `health.rs` unit tests (existing style, `Instant`-based, no real sleeps): `compute_health` returns `Healthy` while `compacting=true` even with `last_meaningful_ts` far in the past; returns `Dead` once `compacting_started_ts` exceeds `COMPACTING_DEAD_SECS` even while still `compacting=true`; `set_compacting(false)` immediately re-arms the normal `STALL_SECS`/`DEAD_SECS` checks against whatever `last_meaningful_ts` already is (don't silently reset it — a compaction that ends into an *already*-stale stream should still be judged on its own silence, not get a fresh 120s grace period for free).
- Integration-shaped test on one of the three wired controllers (mirroring `dead_transition_publishes_unresponsive_failure`'s style): feed a synthetic `compact_boundary` line through the stdout-handling path and assert `set_compacting(false)` fires (i.e. the controller's health monitor no longer reports `compacting`).
- `handle_wps_publish`: a `compaction_started` publish for a `block_id` with a registered controller reaches that controller's `HealthMonitor::set_compacting(true)`; a `compaction_started` publish for an unknown/unregistered block id is a no-op, not an error (mirrors the hook's own "never fail the operation" philosophy) — `handle_wps_publish` must still `200 OK` either way, since this is best-effort observability wiring, not a contract the hook depends on.
- Manual/live verification: trigger `/compact` on a real Claude Code session with a large enough context that compaction visibly runs past 120s (or temporarily lower `COMPACTING_DEAD_SECS` for testing), confirm the pane shows only "Compacting… Ns" throughout with no "Agent unresponsive" banner, and confirm a genuinely killed process during that window still eventually surfaces the banner once `COMPACTING_DEAD_SECS` elapses.

## 6. Open questions for the operator

1. Confirm or adjust the proposed `COMPACTING_DEAD_SECS` = 600s ceiling (§3.1).
2. Whether to investigate the `Task`/subagent-dispatch hypothesis (§1.1) now, or wait and see if it recurs after the compaction fix ships — since compaction may well have been the entirety of "other times."
3. Whether a real `AgentHealth::Compacting` state (vs. reusing `Healthy`) is wanted for future observability, given `agenthealth`'s current "no frontend subscriber" status (§3.1).

---

## Sources

- `agentmux-srv/src/backend/blockcontroller/health.rs` — the detector itself
- `docs/reports/REPORT_WORKING_STATE_REGRESSION_AND_STUCK_QUESTION_PANEL_2026_07_27.md` §2.7/§4 — original design intent and incident history for the Unresponsive/Restart feature
- `docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md` — compaction visibility feature this spec closes a gap against, including the real 231.6s captured duration and the "no intermediate progress signal" finding (§7 Tier 4)
- `agentmux-bashwrap/src/precompact.rs`, `agentmux-bashwrap/src/wps_client.rs` — `PreCompact` hook → `compaction_started` WPS publish
- `agentmux-srv/src/server/mod.rs` (`handle_wps_publish`), `agentmux-srv/src/backend/blockcontroller/mod.rs` (`get_controller`, `Controller` trait) — proposed wiring points
- `agentmux-srv/src/backend/blockcontroller/persistent.rs`, `subprocess/host_spawn.rs`, `subprocess/container_spawn.rs` — existing `classify_output_line`/`record_output` call sites, proposed `compact_boundary` interception points
- `frontend/app/view/agent/hooks/useAgentFailure.ts`, `frontend/app/view/agent/failure/failure-accessory.ts` — where "Agent unresponsive" is actually rendered
