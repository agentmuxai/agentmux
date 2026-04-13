// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentCard — a single entry in the agent picker list.
 *
 * PR 1 of specs/SPEC_CONSOLIDATE_FORGE_IDENTITY_INTO_AGENT_2026_04_13.md.
 *
 * For now this is a pure extraction of the inline button JSX that lived
 * inside AgentPicker's <For>. Later PRs add the per-agent Forge /
 * Identity buttons and the inline settings slide-over.
 */

import { Show, type JSX } from "solid-js";

interface AgentCardProps {
    agent: ForgeAgent;
    launching: boolean;
    disabled: boolean;
    onLaunch: (agent: ForgeAgent) => void;
}

export const AgentCard = (props: AgentCardProps): JSX.Element => {
    return (
        <button
            class={`agent-card${props.launching ? " agent-card--launching" : ""}`}
            onClick={() => props.onLaunch(props.agent)}
            disabled={props.disabled}
        >
            <span class="agent-card-icon">{props.agent.icon}</span>
            <span class="agent-card-info">
                <span class="agent-card-name">{props.agent.name}</span>
                <Show when={props.agent.description}>
                    <span class="agent-card-desc">{props.agent.description}</span>
                </Show>
            </span>
            <Show when={props.launching}>
                <span class="agent-card-spinner" />
            </Show>
        </button>
    );
};

AgentCard.displayName = "AgentCard";
