// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E.2c.5b — renderer-side subscriber for srv reducer typed events.
//
// Mirrors the `launcher-events.ts` pattern from Phase B.7.3.1, but
// for the srv reducer's broadcast bus (workspace / tab / block / window
// / saga lifecycle). The host's CEF JS bridge
// (`agentmux-cef/src/srv_event_bridge.rs`, shipped in Phase E.2c.5a /
// PR #618) calls `window.__agentmux_srv_event(<json>)` once per
// top-level renderer per srv event. We feed those into a SolidJS
// signal so block-level subscribers can `createEffect()` on them.
//
// **Initial scope (this PR — E.2c.5b):** scaffolding only. Install
// the dispatcher, expose the latest event signal, log a one-line
// debug summary on first event arrival. **No atom mutation** — the
// existing RPC + WaveObjUpdate pipeline remains the authoritative
// path for workspace/tab/block atom state. Atom-routing flips to
// event-driven in a follow-up (likely E.6 alongside the per-source
// version tracking + saga buffer).
//
// **Saga events** (`saga_started` / `saga_completed` / `saga_failed`)
// arrive on this channel too. Future renderer-side correlation /
// buffering (E.6 §9.2) consumes them via the same signal.
//
// See `docs/specs/SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md` §6.6 + §9.

import { createSignal } from "solid-js";

/**
 * Wire-format srv event. Matches the JSON serialization of
 * `agentmux_common::ipc::Event`
 * (`#[serde(tag = "event", rename_all = "snake_case")]`).
 *
 * Every event carries `event` (discriminant, snake_case) and
 * `version` (monotonic per srv-process run, used for de-dup /
 * resync ordering). Other fields are variant-specific; downstream
 * subscribers narrow on `event` and read fields by name.
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
export interface SrvEvent {
    event: string;
    version: number;
    [field: string]: unknown;
}

const [latestEvent, setLatestEvent] = createSignal<SrvEvent | null>(null);
const [eventVersion, setEventVersion] = createSignal<number>(0);
const [seenAnyEvent, setSeenAnyEvent] = createSignal<boolean>(false);

/** Latest typed event delivered by the srv pipe. Null until the first event. */
export const srvEvent = latestEvent;

/** Monotonic version of the most recent event. 0 until the first event. */
export const srvEventVersion = eventVersion;

/**
 * True once we've received at least one srv typed event. Mirrors
 * `launcherEventsActive` — useful to E.6's resync flow to detect
 * "have we seen at least one srv event?" vs "fresh srv pipe."
 */
export const srvEventsActive = seenAnyEvent;

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
    ) => {
        if (!evt || typeof evt.version !== "number" || typeof evt.event !== "string") {
            console.warn("[srv-events] received malformed event", evt);
            return;
        }
        setLatestEvent(evt);
        setEventVersion(evt.version);
        if (!seenAnyEvent()) {
            setSeenAnyEvent(true);
            console.log("[srv-events] first event received", { event: evt.event, version: evt.version });
        }
    };
    console.log("[srv-events] bridge installed; window.__agentmux_srv_event ready");
}
