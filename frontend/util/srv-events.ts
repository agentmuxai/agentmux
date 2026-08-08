// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E.2c.5b — renderer-side subscriber for srv reducer typed events.
// Phase E.6 — multi-source dispatcher with version tracking + saga buffering.
//
// Mirrors the `launcher-events.ts` pattern from Phase B.7.3.1, but
// for the srv reducer's broadcast bus (workspace / tab / block / window
// / saga lifecycle). The host's CEF JS bridge
// (`agentmux-cef/src/srv_event_bridge.rs`, shipped in Phase E.2c.5a /
// PR #618) calls `window.__agentmux_srv_event(<json>)` once per
// top-level renderer per srv event.
//
// **Phase E.6 additions:** the dispatcher now lives in
// `util/event-buffer.ts::PerSourceTracker`. Behavior:
//   - Per-source version monotonicity check: gaps log a warning + bump
//     `droppedCount`.
//   - Saga buffering: events between `saga_started`/`saga_completed`
//     coalesce into a single SolidJS-effect tick (so atom-router
//     consumers see the saga as one atomic update). Per-event
//     subscriber callbacks still fire in source order.
//   - Stale events (version <= last seen) are dropped.
//
// **Atom routing (still pending — TODO PR after E.6):** we expose a
// `subscribeSrvEvent(cb)` API. The first atom-router consumer ships
// in a follow-up; today the existing RPC + WaveObjUpdate pipeline
// remains authoritative.
//
// See `docs/specs/SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md` §6.6 + §9
// and `docs/retro/next-steps-architecture-completeness-2026-05-01.md`
// step 2 for the broader plan.

import { PerSourceTracker, type EventCallback, type VersionedEvent } from "./event-buffer";

/**
 * Wire-format srv event. Matches the JSON serialization of
 * `agentmux_common::ipc::Event`
 * (`#[serde(tag = "event", rename_all = "snake_case")]`).
 *
 * Discriminator examples (non-exhaustive — see
 * `agentmux-common/src/ipc.rs::Event`):
 *   - `workspace_created` / `workspace_deleted` / `workspace_renamed`
 *   - `tab_created` / `tab_deleted` / `tab_renamed` / `tab_moved` / `tab_reordered`
 *   - `block_created` / `block_deleted` / `block_moved`
 *   - `active_tab_changed`
 *   - `srv_window_opened` / `srv_window_closed` / `srv_window_workspace_changed`
 *   - `tabs_reordered_bulk`
 *   - `workspace_meta_updated` / `tab_meta_updated` / `block_meta_updated`
 *   - `saga_started` / `saga_completed` / `saga_failed`
 */
export type SrvEvent = VersionedEvent;

// No signal setters — nothing in the srv pipe reads a "latest
// event"/"version"/"saw any" signal today (those were dead exports,
// removed; see git history). `PerSourceTracker`'s setters are optional
// for exactly this case: don't pay for signal writes nobody reads.
const tracker = new PerSourceTracker<SrvEvent>({ source: "srv" }, {});

/**
 * Per-event subscriber. Called once per srv event in source order,
 * including events delivered as part of a saga buffer flush. Returns
 * an unsubscribe function.
 *
 * Use this for atom routers — every event matters for correct atom
 * state.
 */
export function subscribeSrvEvent(cb: EventCallback<SrvEvent>): () => void {
    return tracker.subscribe(cb);
}

let installed = false;

/**
 * Register `window.__agentmux_srv_event` as the dispatcher.
 * Idempotent — safe to call multiple times. Called once from
 * `app-init.ts::initApp` BEFORE the first state-needing operation
 * so events that arrive during init aren't dropped.
 */
export function installSrvEventBridge(): void {
    if (installed) return;
    installed = true;
    (window as unknown as { __agentmux_srv_event?: (evt: SrvEvent) => void }).__agentmux_srv_event = (
        evt: SrvEvent,
    ) => tracker.deliver(evt);
    console.log("[srv-events] bridge installed; window.__agentmux_srv_event ready");
}
