// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Warden — Host section. Lifted out of the original monolithic warden.tsx
// (Phase 2 of specs/SPEC_WARDEN_WIDGET_2026-05-25.md) into its own
// rail-switchable manager. Behavior unchanged: fetches the agent list from
// /agentmux/reactive/agents on mount and refreshes every 5s. The audit feed
// that used to render below this table now lives in its own "Audit" rail
// section (warden-audit-manager.tsx) instead of being bolted on here.

import { createMemo, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { useTick } from "@/app/hook/useTick";

import { getWebServerEndpoint } from "@/util/endpoints";
import { authedHeaders, ageMs, formatAge, WARDEN_REFRESH_MS } from "@/app/view/warden-shared/warden-shared";

import "@/app/view/warden-shared/warden-manager-chrome.scss";
import "./warden-host-manager.scss";

interface HostAgent {
    agent_id: string;
    block_id: string;
    tab_id?: string;
    registered_at: number;
    last_seen: number;
}

// Last-seen newer than this counts as "active"; older is "idle".
const ACTIVE_THRESHOLD_MS = 30_000;

async function fetchHostAgents(): Promise<HostAgent[]> {
    const resp = await fetch(
        getWebServerEndpoint() + "/agentmux/reactive/agents",
        { headers: authedHeaders() },
    );
    if (!resp.ok) {
        throw new Error(`warden: GET /agentmux/reactive/agents → ${resp.status}`);
    }
    const data = await resp.json();
    return Array.isArray(data) ? (data as HostAgent[]) : [];
}

/**
 * Deregister an agent from the local ReactiveHandler. This is a *soft*
 * enforcement action — it removes the agent's routing entry so future
 * jekts return "agent not found", but does NOT kill the underlying PTY
 * process (that's owned by the pane / block controller). The agent's
 * shell auto-register hook may re-register on its next heartbeat.
 *
 * Hard kill (PTY termination) lives outside Warden — see future PR.
 */
async function deregisterAgent(agentId: string): Promise<void> {
    const resp = await fetch(
        getWebServerEndpoint() + "/agentmux/reactive/unregister",
        {
            method: "POST",
            headers: { ...authedHeaders(), "Content-Type": "application/json" },
            body: JSON.stringify({ agent_id: agentId }),
        },
    );
    if (!resp.ok) {
        throw new Error(`warden: POST /agentmux/reactive/unregister → ${resp.status}`);
    }
}

function agentState(agent: HostAgent, now: number): "active" | "idle" {
    return ageMs(agent.last_seen, now) < ACTIVE_THRESHOLD_MS ? "active" : "idle";
}

export const WardenHostManager = (): JSX.Element => {
    const [agents, setAgents] = createSignal<HostAgent[]>([]);
    const [error, setError] = createSignal<string | null>(null);
    const [loading, setLoading] = createSignal(true);
    const tick = useTick(1000);
    const now = createMemo(() => (tick(), Date.now()));

    const refresh = async () => {
        try {
            const agentList = await fetchHostAgents();
            setAgents(agentList);
            setError(null);
        } catch (e) {
            setError(String(e));
        } finally {
            setLoading(false);
        }
    };

    const handleDeregister = async (agentId: string) => {
        const confirmed = globalThis.window?.confirm(
            `Deregister agent "${agentId}"?\n\nThis removes its routing entry so future jekts return "agent not found". The underlying process keeps running and may re-register on its next heartbeat.`,
        );
        if (!confirmed) return;
        try {
            await deregisterAgent(agentId);
            void refresh();
        } catch (e) {
            setError(String(e));
        }
    };

    onMount(() => {
        void refresh();
        const dataTimer = window.setInterval(() => void refresh(), WARDEN_REFRESH_MS);
        onCleanup(() => window.clearInterval(dataTimer));
    });

    return (
        <div class="warden-manager-body">
            <p class="warden-manager-summary">This AgentMux process · jekt tiers 1–2 · &lt; 1 ms</p>
            <Show when={error()}>
                <div class="warden-manager-error">⚠ {error()}</div>
            </Show>
            <Show
                when={agents().length > 0}
                fallback={
                    <div class="warden-section-stub">
                        {loading() ? "Loading…" : "No agents registered on this host."}
                    </div>
                }
            >
                <table class="warden-manager-table">
                    <thead>
                        <tr>
                            <th>agent</th>
                            <th>block</th>
                            <th>last seen</th>
                            <th>state</th>
                            <th></th>
                        </tr>
                    </thead>
                    <tbody>
                        <For each={agents()}>
                            {(a) => {
                                const state = () => agentState(a, now());
                                return (
                                    <tr data-state={state()}>
                                        <td class="warden-manager-mono">{a.agent_id}</td>
                                        <td class="warden-manager-mono warden-manager-dim">
                                            {a.block_id.slice(0, 8)}
                                        </td>
                                        <td>{formatAge(ageMs(a.last_seen, now()))} ago</td>
                                        <td>
                                            <span class={`warden-host-state warden-host-state--${state()}`}>
                                                {state()}
                                            </span>
                                        </td>
                                        <td class="warden-host-actions">
                                            <button
                                                class="warden-host-deregister"
                                                title="Deregister (soft kill — removes from jekt routing, leaves process running)"
                                                onClick={() => void handleDeregister(a.agent_id)}
                                            >
                                                ×
                                            </button>
                                        </td>
                                    </tr>
                                );
                            }}
                        </For>
                    </tbody>
                </table>
            </Show>
            <div class="warden-manager-footnote">
                Refreshes every {WARDEN_REFRESH_MS / 1000}s. Identity, capabilities,
                and jekt/min counters are coming in a follow-up PR.
            </div>
        </div>
    );
};

WardenHostManager.displayName = "WardenHostManager";
