// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The three lookups the agent picker and its neighbours read agent definitions and open panes
// through (SPEC_AGENT_PANE_CLEANUP: out of AgentPicker.tsx). They are three jobs, not copies:
// the definition list, this window's definition -> pane map, and the instance-wide map built
// on that one. AgentPicker re-exports them, so existing importers and test mocks are unchanged.

import { subscribeToPaneLifecycle } from "@/app/store/agent-pane-registration";
import { getOpenDefinitionMap } from "@/app/store/agent-pane-state-store";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import type { AgentDefinition, AgentOpenPane } from "@/app/store/rpc-api";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { windowId } from "@/app/store/window-identity";
import { createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { mergeOpenDefinitions, openAgentLocations, type OpenAgentLocation } from "../open-agent-panes";

/**
 * `next`, but with every definition whose content didn't change replaced by
 * the object already in `prev`. `<For>` keys by object identity, so without
 * this every `agents:changed` refetch — which fires for ANY definition edit,
 * fork or hide app-wide, including other agents calling `agent.define` —
 * remounted every card in every open picker.
 */
function reuseUnchangedById(prev: AgentDefinition[], next: AgentDefinition[]): AgentDefinition[] {
    const prevById = new Map(prev.map((a) => [a.id, a]));
    let changed = prev.length !== next.length;
    const merged = next.map((a, i) => {
        const old = prevById.get(a.id);
        const reused = old != null && JSON.stringify(old) === JSON.stringify(a) ? old : a;
        if (reused !== prev[i]) changed = true;
        return reused;
    });
    return changed ? merged : prev;
}


/**
 * Reactive accessor for the current agent-definition list, plus whether the
 * FIRST fetch is still in flight. Subscribes to `agents:changed` and
 * refetches when that event fires.
 *
 * Returns a tuple `[agents, loading]` (matching `useOpenDefinitionMap`'s own
 * convention below) rather than just the list accessor — `agents` starts as
 * `[]` and stays indistinguishable from "genuinely zero definitions" without
 * a separate loading flag, which is exactly what let AgentPicker's `<Show
 * when={agents().length > 0}>` gate flash its "No definitions configured"
 * empty state on every single mount for users who actually have agents,
 * before the first `ListAgentDefinitionsCommand` response ever arrived (see
 * AgentPicker's own render for the fix). `loading` only ever reflects the
 * FIRST fetch — `agents:changed` refetches don't flip it back to true, so a
 * background refresh never re-triggers whatever a caller gates on it.
 */
export function useAgentDefinitions(): [() => AgentDefinition[], () => boolean] {
    const [agents, setAgents] = createSignal<AgentDefinition[]>([]);
    const [loading, setLoading] = createSignal(true);

    onMount(() => {
        let cancelled = false;
        let firstLoad = true;

        async function load() {
            try {
                const result = await RpcApi.ListAgentDefinitionsCommand(TabRpcClient);
                if (!cancelled) setAgents((prev) => reuseUnchangedById(prev, result ?? []));
            } catch {
                // silently ignore
            } finally {
                if (!cancelled && firstLoad) {
                    firstLoad = false;
                    setLoading(false);
                }
            }
        }

        load();

        const unsub = onMuxEvent(WpsEvent.AgentsChanged, () => load());

        onCleanup(() => {
            cancelled = true;
            unsub();
        });
    });

    return [agents, loading];
}

/**
 * Reactive definitionId → open blockId map. Refreshes on pane register/
 * unregister (`subscribeToPaneLifecycle`) AND on `agents:changed` so newly-
 * forked definitions also appear. Extracted from what was AgentPicker's own
 * inline signal so other consumers (the agent-pane fork tab strip,
 * SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md §4.1) can share it
 * instead of re-deriving the same map.
 */
export function useOpenDefinitionMap(): [() => Map<string, string>, () => void] {
    const [openDefinitions, setOpenDefinitions] = createSignal<Map<string, string>>(new Map());
    const refresh = () => setOpenDefinitions(getOpenDefinitionMap());
    onMount(refresh);
    const unsubAgentsChanged = onMuxEvent(WpsEvent.AgentsChanged, refresh);
    const unsubPaneLifecycle = subscribeToPaneLifecycle(refresh);
    onCleanup(unsubAgentsChanged);
    onCleanup(unsubPaneLifecycle);
    return [openDefinitions, refresh];
}

/**
 * Like `useOpenDefinitionMap`, but instance-wide: an agent open in ANY tab of
 * ANY window — floating ones included — counts as open, via srv's
 * `agent.open-panes`. `useOpenDefinitionMap` only sees this window's panes,
 * which is right for the fork tab strip (its pills switch within the window)
 * but let My agents reattach an agent that was live in a floating window
 * (SPEC_AGENT_SAME_PROCESS_DUPLICATE_PANE_RECOVERY_2026_10_01.md §4.0).
 * Returns `[map, refresh, locations, resolve]`. The map is a cached view for
 * rendering; a decision (reattach or not) must use `resolve`, which answers
 * from a fresh srv read — before the first one lands, or after a failed one,
 * the cached map can't tell "open elsewhere" from "closed" (Codex P1, #4127).
 */
export function useInstanceOpenDefinitions(): [
    () => Map<string, string>,
    () => void,
    () => Map<string, OpenAgentLocation>,
    () => Promise<Map<string, string>>,
] {
    const [local, refreshLocal] = useOpenDefinitionMap();
    const [panes, setPanes] = createSignal<AgentOpenPane[]>([]);
    let fetchSeq = 0;
    let appliedSeq = 0;
    /** `true` once srv answered. A reply older than one already applied is dropped. */
    const fetchPanes = async (): Promise<boolean> => {
        const seq = ++fetchSeq;
        try {
            const r = await RpcApi.AgentOpenPanesCommand(TabRpcClient, {});
            if (seq > appliedSeq) {
                appliedSeq = seq;
                setPanes(r?.panes ?? []);
            }
            return true;
        } catch (e) {
            console.warn("[agent-picker] agent.open-panes failed", String(e));
            return false;
        }
    };
    onMount(() => void fetchPanes());
    const unsub = onMuxEvent(WpsEvent.ProcessBrokerTrackedBlocksChanged, () => void fetchPanes());
    onCleanup(unsub);
    // Tear-off, redock and tab moves keep the pane's controller, so they emit
    // no tracked-blocks event; refetch when the user comes back to this
    // window, which is when a moved pane's location matters (Codex P2, #4127).
    const onFocus = () => void fetchPanes();
    window.addEventListener("focus", onFocus);
    onCleanup(() => window.removeEventListener("focus", onFocus));
    const merged = createMemo(() => mergeOpenDefinitions(local(), panes(), windowId()));
    const locations = createMemo(() => openAgentLocations(local(), panes(), windowId()));
    const refresh = () => {
        refreshLocal();
        void fetchPanes();
    };
    const resolve = async (): Promise<Map<string, string>> => {
        refreshLocal();
        // One retry for a transient failure. If srv still can't answer (an
        // older srv without agent.open-panes), this window's panes are all
        // there is to go on.
        if (!(await fetchPanes())) {
            await new Promise((r) => setTimeout(r, 250));
            await fetchPanes();
        }
        return merged();
    };
    return [merged, refresh, locations, resolve];
}
