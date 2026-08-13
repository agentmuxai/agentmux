// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Warden — Supervisor section. A control surface only: a per-agent
// auto_continue_enabled toggle list, plus a recent-decisions feed (reusing
// warden-audit-shared.ts, client-filtered to entries with `outcome` set).
// The judgment itself — whether/when to nudge — runs inside an ordinary
// spawned AgentMux agent using GetAgentTranscript + SupervisorNudge (MCP
// tools), not here. See
// docs/analysis/ANALYSIS_WARDEN_AUTO_CONTROLLER_CONTINUATION_WATCHER_2026_08_12.md.
//
// Explicitly out of scope (v1): a "spawn a Supervisor for this agent"
// button, or any liveness check proving a specific running agent IS the
// Supervisor for a given target. The toggle is the source of truth for
// "opted in"; operators spawn/configure the actual watcher agent
// themselves, same as spawning any other agent today.

import { createMemo, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { useTick } from "@/app/hook/useTick";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

import { ageMs, formatAge, WARDEN_REFRESH_MS } from "@/app/view/warden-shared/warden-shared";
import { fetchWardenAudit, type AuditEntry } from "@/app/view/warden-audit/warden-audit-shared";

import "@/app/view/warden-shared/warden-manager-chrome.scss";
import "./warden-supervisor-manager.scss";

export const WardenSupervisorManager = (): JSX.Element => {
    const [agents, setAgents] = createSignal<AgentDefinition[]>([]);
    const [decisions, setDecisions] = createSignal<AuditEntry[]>([]);
    const [error, setError] = createSignal<string | null>(null);
    const [loading, setLoading] = createSignal(true);
    const [savingId, setSavingId] = createSignal<string | null>(null);
    const tick = useTick(1000);
    const now = createMemo(() => (tick(), Date.now()));

    const refreshAgents = async () => {
        try {
            const list = await RpcApi.ListAgentDefinitionsCommand(TabRpcClient, { is_seeded: 0 });
            setAgents(list);
            setError(null);
        } catch (e) {
            setError(String((e as Error).message ?? e));
        } finally {
            setLoading(false);
        }
    };

    const refreshDecisions = async () => {
        try {
            const log = await fetchWardenAudit();
            setDecisions(log.filter((entry) => entry.outcome != null));
        } catch {
            // The Audit tab already surfaces fetch failures for this same
            // endpoint — avoid a second, redundant error banner here.
        }
    };

    const handleToggle = async (agent: AgentDefinition, enabled: boolean) => {
        setSavingId(agent.id);
        try {
            await RpcApi.UpdateAgentDefinitionCommand(TabRpcClient, {
                id: agent.id,
                name: agent.name,
                icon: agent.icon,
                provider: agent.provider,
                description: agent.description,
                working_directory: agent.working_directory,
                shell: agent.shell,
                provider_flags: agent.provider_flags,
                auto_start: agent.auto_start,
                restart_on_crash: agent.restart_on_crash,
                idle_timeout_minutes: agent.idle_timeout_minutes,
                agent_type: agent.agent_type,
                environment: agent.environment,
                agent_bus_id: agent.agent_bus_id,
                auto_continue_enabled: enabled ? 1 : 0,
            });
            setAgents((prev) =>
                prev.map((a) => (a.id === agent.id ? { ...a, auto_continue_enabled: enabled ? 1 : 0 } : a)),
            );
        } catch (e) {
            setError(String((e as Error).message ?? e));
        } finally {
            setSavingId(null);
        }
    };

    onMount(() => {
        void refreshAgents();
        void refreshDecisions();
        const dataTimer = window.setInterval(() => {
            void refreshAgents();
            void refreshDecisions();
        }, WARDEN_REFRESH_MS);
        onCleanup(() => window.clearInterval(dataTimer));
    });

    return (
        <div class="warden-manager-body">
            <p class="warden-manager-summary">Opt-in continuation nudging for stalled agents</p>
            <Show when={error()}>
                <div class="warden-manager-error">⚠ {error()}</div>
            </Show>
            <Show
                when={agents().length > 0}
                fallback={
                    <div class="warden-section-stub">
                        {loading() ? "Loading…" : "No agents yet."}
                    </div>
                }
            >
                <table class="warden-manager-table">
                    <thead>
                        <tr>
                            <th>agent</th>
                            <th>provider</th>
                            <th>auto-continue</th>
                        </tr>
                    </thead>
                    <tbody>
                        <For each={agents()}>
                            {(agent) => (
                                <tr>
                                    <td class="warden-manager-mono">{agent.name}</td>
                                    <td class="warden-manager-dim">{agent.provider}</td>
                                    <td>
                                        <label class="warden-supervisor-toggle-row">
                                            <input
                                                type="checkbox"
                                                checked={!!agent.auto_continue_enabled}
                                                disabled={savingId() === agent.id}
                                                onChange={(e) => void handleToggle(agent, e.currentTarget.checked)}
                                            />
                                            <Show when={savingId() === agent.id}>
                                                <span class="warden-manager-dim">saving…</span>
                                            </Show>
                                        </label>
                                    </td>
                                </tr>
                            )}
                        </For>
                    </tbody>
                </table>
            </Show>

            <p class="warden-manager-summary">Recent Supervisor decisions</p>
            <Show
                when={decisions().length > 0}
                fallback={<div class="warden-section-stub">No Supervisor decisions yet.</div>}
            >
                <ul class="warden-supervisor-decision-feed">
                    <For each={decisions()}>
                        {(entry) => {
                            const statusLabel = () => {
                                if (entry.outcome === "nudge_sent") return "nudged";
                                if (entry.outcome === "nudge_failed") return "nudge failed";
                                return "declined";
                            };
                            return (
                                <li
                                    class="warden-supervisor-decision-row"
                                    data-variant={entry.outcome === "nudge_failed" ? "failed" : "ok"}
                                >
                                    <span class="warden-supervisor-decision-time">
                                        {formatAge(ageMs(entry.timestamp, now()))} ago
                                    </span>
                                    <span class="warden-manager-mono">{entry.target_agent}</span>
                                    <span class="warden-supervisor-decision-status">
                                        {statusLabel()}
                                    </span>
                                    <Show when={entry.reason} fallback={<span class="warden-manager-dim">—</span>}>
                                        <span class="warden-supervisor-decision-reason">{entry.reason}</span>
                                    </Show>
                                </li>
                            );
                        }}
                    </For>
                </ul>
            </Show>

            <div class="warden-manager-footnote">
                Refreshes every {WARDEN_REFRESH_MS / 1000}s. Spawn/configure the
                Supervisor watcher agent itself the same way you'd spawn any
                other agent — this panel only controls which agents it may
                act on.
            </div>
        </div>
    );
};

WardenSupervisorManager.displayName = "WardenSupervisorManager";
