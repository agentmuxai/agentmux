// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentCard — a single entry in the agent picker list.
 *
 * PR 2 of specs/SPEC_CONSOLIDATE_FORGE_IDENTITY_INTO_AGENT_2026_04_13.md
 * adds the ⚙ Forge and 👤 Identity buttons that expand an inline
 * settings panel below the card.
 *
 * Clicking the card body still launches the agent. The buttons call
 * stopPropagation so they never accidentally trigger launch.
 *
 * The card is rendered as a <div role="button"> rather than <button>
 * so nested action buttons are valid HTML (buttons can't contain
 * buttons).
 */

import { Show, type JSX } from "solid-js";

interface AgentCardProps {
    agent: ForgeAgent;
    launching: boolean;
    disabled: boolean;
    onLaunch: (agent: ForgeAgent) => void;
    onOpenForge: (agent: ForgeAgent) => void;
    onOpenIdentity: (agent: ForgeAgent) => void;
}

export const AgentCard = (props: AgentCardProps): JSX.Element => {
    const stopAndRun = (fn: () => void) => (e: MouseEvent) => {
        e.stopPropagation();
        fn();
    };

    const handleCardClick = () => {
        if (!props.disabled) props.onLaunch(props.agent);
    };

    const handleKeyDown = (e: KeyboardEvent) => {
        if (props.disabled) return;
        if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            props.onLaunch(props.agent);
        }
    };

    return (
        <div
            class={`agent-card${props.launching ? " agent-card--launching" : ""}`}
            classList={{ "agent-card--disabled": props.disabled }}
            onClick={handleCardClick}
            onKeyDown={handleKeyDown}
            role="button"
            tabIndex={props.disabled ? -1 : 0}
            aria-disabled={props.disabled}
        >
            <span class="agent-card-icon">{props.agent.icon}</span>
            <span class="agent-card-info">
                <span class="agent-card-name">{props.agent.name}</span>
                <Show when={props.agent.description}>
                    <span class="agent-card-desc">{props.agent.description}</span>
                </Show>
            </span>
            <div class="agent-card-actions">
                <button
                    class="agent-card-action-btn"
                    onClick={stopAndRun(() => props.onOpenForge(props.agent))}
                    title="Configure this agent in the Forge"
                    disabled={props.disabled}
                    type="button"
                >
                    {"\u2699"}
                </button>
                <button
                    class="agent-card-action-btn"
                    onClick={stopAndRun(() => props.onOpenIdentity(props.agent))}
                    title="Manage this agent's identity"
                    disabled={props.disabled}
                    type="button"
                >
                    {"\uD83D\uDC64"}
                </button>
            </div>
            <Show when={props.launching}>
                <span class="agent-card-spinner" />
            </Show>
        </div>
    );
};

AgentCard.displayName = "AgentCard";
