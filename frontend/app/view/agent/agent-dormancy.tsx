// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pane dormancy, routed to the parts of the transcript that do expensive work.
 *
 * WHY THIS EXISTS
 *
 * Keep-alive (`pane-leaf-chrome.tsx`, `isKeepAliveView`) switches pane-stack
 * members by **visibility, not existence** — a backgrounded agent tab stays
 * mounted under `visibility: hidden`. Combined with a stream path that has no
 * visibility gate, every backgrounded-but-streaming agent pane keeps parsing
 * and rendering markdown on the SAME main thread as the pane you are typing
 * into. Work scales with the number of streaming panes, not with how many you
 * can actually see. See
 * `docs/analysis/ANALYSIS_AGENT_PANE_TYPING_UNDER_LOAD_2026_09_22.md` §4.
 *
 * `isBlockDormant(blockId)` (`store/block-component-registry.ts`) already
 * carries exactly this signal — `pane-leaf-chrome.tsx` sets it, and
 * `AgentQuestionPanel`/`useAgentFailure` already pause on it. What was missing
 * is a route from there down to the render path, which is several layers deep
 * (agent-view → AgentDocumentVirtualList → DocumentRow → MarkdownBlock) and
 * passes through virtualization code that is performance-critical and
 * deliberately tuned. Context rather than prop drilling, so that machinery
 * does not have to grow another prop it would only forward.
 *
 * WHAT THIS GATES, AND WHAT IT MUST NOT
 *
 * Rendering only — never data. A dormant pane keeps receiving stream events
 * and keeps its document store current, so nothing is lost, activity
 * indicators outside the pane keep updating, and switching to it is instant.
 * Only the expensive *paint* of content nobody can currently see is deferred.
 *
 * Gating the stream flush instead would be a real regression: it would freeze
 * the store that off-pane indicators read from.
 */

import { createContext, useContext, type Accessor, type JSX } from "solid-js";

/** Defaults to "visible", so any consumer outside an agent pane is unaffected. */
const AgentDormancyContext = createContext<Accessor<boolean>>(() => false);

export function AgentDormancyProvider(props: { dormant: Accessor<boolean>; children: JSX.Element }): JSX.Element {
    return (
        <AgentDormancyContext.Provider value={props.dormant}>{props.children}</AgentDormancyContext.Provider>
    );
}

/**
 * True while this pane is mounted but not visible. Safe to call anywhere —
 * outside a provider it reads `false`.
 */
export function useAgentDormant(): Accessor<boolean> {
    return useContext(AgentDormancyContext);
}
