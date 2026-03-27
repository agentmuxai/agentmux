// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal } from "solid-js";
import type { JSX } from "solid-js";
import { Show } from "solid-js";
import type { ForgeViewModel } from "../forge-model";
import { PROVIDERS } from "../forge-constants";

export function ForgeAgentCard(props: { agent: ForgeAgent; model: ForgeViewModel }): JSX.Element {
    const [confirming, setConfirming] = createSignal(false);

    const handleDelete = async () => {
        if (!confirming()) {
            setConfirming(true);
            return;
        }
        setConfirming(false);
        await props.model.deleteAgent(props.agent.id);
    };

    const handleClick = () => {
        props.model.openDetail(props.agent);
    };

    const handleEdit = (e: MouseEvent) => {
        e.stopPropagation();
        props.model.startEdit(props.agent);
    };

    const handleDeleteClick = (e: MouseEvent) => {
        e.stopPropagation();
        handleDelete();
    };

    const providerLabel = () => PROVIDERS.find((p) => p.id === props.agent.provider)?.label ?? props.agent.provider;
    const typeBadge = () => {
        if (props.agent.agent_type === "host") return "HOST";
        if (props.agent.agent_type === "container") return "CONTAINER";
        return null;
    };

    return (
        <div class="forge-card" onClick={handleClick}>
            <span class="forge-card-icon">{props.agent.icon}</span>
            <div class="forge-card-info">
                <div class="forge-card-name-row">
                    <span class="forge-card-name">{props.agent.name}</span>
                    <Show when={typeBadge()}>
                        <span class={`forge-agent-type-badge forge-agent-type-${props.agent.agent_type}`}>{typeBadge()}</span>
                    </Show>
                </div>
                <span class="forge-card-provider">{providerLabel()}</span>
                <Show when={props.agent.description}>
                    <span class="forge-card-desc">{props.agent.description}</span>
                </Show>
            </div>
            <div class="forge-card-actions">
                <button class="forge-card-btn" onClick={handleEdit} title="Edit">
                    Edit
                </button>
                <button
                    class={`forge-card-btn forge-card-btn-delete${confirming() ? " confirming" : ""}`}
                    onClick={handleDeleteClick}
                    onBlur={() => setConfirming(false)}
                    title={confirming() ? "Click again to confirm" : "Delete"}
                >
                    {confirming() ? "Sure?" : "\u2715"}
                </button>
            </div>
        </div>
    );
}
