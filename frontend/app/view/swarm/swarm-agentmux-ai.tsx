// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "AgentMux AI": how the model calls AgentMux makes on its own (session titles,
 * names, prompt suggestions, narration) have gone since its server started, one
 * row per purpose. Shown at the foot of the Swarm once any call has ended; an
 * older srv without `ambient.outcomes` leaves it hidden.
 * docs/specs/SPEC_AMBIENT_SWARM_SUMMARY_HARDENING_2026_10_02.md section 5.8.
 */

import { createMemo, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { summarizeAmbientOutcomes, type AmbientOutcomes } from "@/app/store/ambient-outcomes";
import "./swarm-agentmux-ai.scss";

/** The counters are in-memory on srv and cheap to read. */
const REFRESH_MS = 15_000;

export function AgentMuxAiSection(): JSX.Element {
    const [outcomes, setOutcomes] = createSignal<AmbientOutcomes | null>(null);
    const [collapsed, setCollapsed] = createSignal(true);
    const summary = createMemo(() => summarizeAmbientOutcomes(outcomes()));

    const refresh = (): void => {
        RpcApi.AmbientOutcomesCommand(TabRpcClient, {})
            .then((o) => setOutcomes(o ?? null))
            .catch(() => {});
    };
    onMount(() => {
        refresh();
        const timer = setInterval(refresh, REFRESH_MS);
        onCleanup(() => clearInterval(timer));
    });

    return (
        <Show when={summary()}>
            {(s) => (
                <div class="swarm-agentmux-ai">
                    <button
                        type="button"
                        class="swarm-remote-header swarm-agentmux-ai-header"
                        aria-expanded={!collapsed()}
                        title="Model calls AgentMux makes on its own (session titles, names, prompt suggestions), counted since its server started"
                        onClick={() => {
                            setCollapsed(!collapsed());
                            if (!collapsed()) refresh();
                        }}
                    >
                        <i classList={{ "fa-solid": true, "fa-chevron-down": !collapsed(), "fa-chevron-right": collapsed() }} />
                        <span class="swarm-remote-title">AgentMux AI</span>
                        <span class="swarm-remote-badge">{s().total} calls</span>
                        <Show when={s().unhealthyCount > 0}>
                            <span class="swarm-agentmux-ai-warn">
                                {s().unhealthyCount === 1 ? "1 failing" : `${s().unhealthyCount} failing`}
                            </span>
                        </Show>
                    </button>
                    <Show when={!collapsed()}>
                        <For each={s().rows}>
                            {(row) => (
                                <div class="swarm-agentmux-ai-row" title={row.detail}>
                                    <span class="swarm-agentmux-ai-name">{row.name}</span>
                                    <span
                                        classList={{
                                            "swarm-agentmux-ai-counts": true,
                                            "swarm-agentmux-ai-counts--warn": row.unhealthy,
                                        }}
                                    >
                                        {row.text}
                                    </span>
                                </div>
                            )}
                        </For>
                        <div class="swarm-agentmux-ai-note">Since AgentMux's server started</div>
                    </Show>
                </div>
            )}
        </Show>
    );
}
