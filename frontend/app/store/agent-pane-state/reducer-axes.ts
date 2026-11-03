// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { AgentPaneCommand, AgentPaneState, ReducerResult } from "./types";

/** `DetailsToggle`. */
export function onDetailsToggle(state: AgentPaneState): ReducerResult {
    return {
        state: { ...state, detailsOpen: !state.detailsOpen },
        events: [],
    };
}

/** `DetailsExpand`. */
export function onDetailsExpand(state: AgentPaneState): ReducerResult {
    if (state.detailsOpen) return { state, events: [] };
    return {
        state: { ...state, detailsOpen: true },
        events: [],
    };
}

/** `DetailsCollapse`. */
export function onDetailsCollapse(state: AgentPaneState): ReducerResult {
    if (!state.detailsOpen) return { state, events: [] };
    return {
        state: { ...state, detailsOpen: false },
        events: [],
    };
}

// ── Attached task axis (SPEC_ATTACHED_TASK_STATUS_AXIS_2026_08_02.md) ──
export function onAttachedTaskObserved(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "AttachedTaskObserved" }>): ReducerResult {
    if (state.attachedTask) return { state, events: [] };
    return {
        state: { ...state, attachedTask: { since: command.at } },
        events: [{ type: "attached-task-observed", at: command.at }],
    };
}

/** `AttachedTaskCleared`. */
export function onAttachedTaskCleared(state: AgentPaneState): ReducerResult {
    if (!state.attachedTask) return { state, events: [] };
    return {
        state: { ...state, attachedTask: null },
        events: [{ type: "attached-task-cleared" }],
    };
}

// ── Registry-derived attached-task floor (Phase C of
// SPEC_BACKGROUND_TASK_DASHBOARD_INTELLIGENCE_2026_08_20.md) — see
// registryAttachedTaskSince's doc comment in types.ts for why this
// is a separate axis, exclusively owned by these two commands. ──
export function onRegistryAttachedTaskObserved(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "RegistryAttachedTaskObserved" }>): ReducerResult {
    if (state.registryAttachedTaskSince === command.at) return { state, events: [] };
    return {
        state: { ...state, registryAttachedTaskSince: command.at },
        events: [{ type: "registry-attached-task-observed", at: command.at }],
    };
}

/** `RegistryAttachedTaskCleared`. */
export function onRegistryAttachedTaskCleared(state: AgentPaneState): ReducerResult {
    if (state.registryAttachedTaskSince == null) return { state, events: [] };
    return {
        state: { ...state, registryAttachedTaskSince: null },
        events: [{ type: "registry-attached-task-cleared" }],
    };
}

// ── Stale-`--resume` reconnect axis
// (STATUS_STALE_RESUME_LIVE_REPRO_AND_FIX_PLAN_2026_08_23.md §6.2) ──
export function onResumeRetryStarted(state: AgentPaneState, command: Extract<AgentPaneCommand, { type: "ResumeRetryStarted" }>): ReducerResult {
    if (state.reconnecting) return { state, events: [] };
    return {
        state: { ...state, reconnecting: { startedAt: command.at } },
        events: [{ type: "resume-retry-started", at: command.at }],
    };
}

/** `ResumeRetryResolved`. */
export function onResumeRetryResolved(state: AgentPaneState): ReducerResult {
    if (!state.reconnecting) return { state, events: [] };
    return {
        state: { ...state, reconnecting: null },
        events: [{ type: "resume-retry-resolved" }],
    };
}
