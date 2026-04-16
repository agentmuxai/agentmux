// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentFocusedPanel — half-pane overlay shown when the user clicks
 * ✏ Rename, ⚙ Forge, or 👤 Identity in the presentation-view title bar.
 *
 * Reuses AgentCardSettingsPanel (forge + identity tabs) and an inline
 * rename input. No new panels — just new wiring.
 *
 * Spec: docs/specs/agent-pane-title-buttons.md
 */

import { createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import type { BlockNodeModel } from "@/app/block/blocktypes";
import { AgentCardSettingsPanel } from "./AgentCardSettingsPanel";
import type { OverlayTab } from "../agent-model";

interface AgentFocusedPanelProps {
    blockId: string;
    nodeModel: BlockNodeModel;
    agent: ForgeAgent;
    initialTab: OverlayTab;
    onClose: () => void;
}

export const AgentFocusedPanel = (props: AgentFocusedPanelProps): JSX.Element => {
    const [editName, setEditName] = createSignal(props.agent.name);
    const [renameError, setRenameError] = createSignal<string | null>(null);
    const [saving, setSaving] = createSignal(false);

    onMount(() => {
        const handleKey = (e: KeyboardEvent) => {
            if (e.key === "Escape") {
                e.preventDefault();
                props.onClose();
            }
        };
        window.addEventListener("keydown", handleKey);
        onCleanup(() => window.removeEventListener("keydown", handleKey));
    });

    const commitRename = async () => {
        const newName = editName().trim();
        if (!newName) {
            setRenameError("Name cannot be empty");
            return;
        }
        if (newName.length > 64) {
            setRenameError("Name must be 64 characters or fewer");
            return;
        }
        if (newName === props.agent.name) {
            props.onClose();
            return;
        }
        setSaving(true);
        try {
            await RpcApi.UpdateForgeAgentCommand(TabRpcClient, {
                id: props.agent.id,
                name: newName,
                icon: props.agent.icon,
                provider: props.agent.provider,
                description: props.agent.description,
                working_directory: props.agent.working_directory,
                shell: props.agent.shell,
                provider_flags: props.agent.provider_flags,
                auto_start: props.agent.auto_start,
                restart_on_crash: props.agent.restart_on_crash,
                idle_timeout_minutes: props.agent.idle_timeout_minutes,
                agent_type: props.agent.agent_type,
                environment: props.agent.environment,
                agent_bus_id: props.agent.agent_bus_id,
            });
            props.onClose();
        } catch (e: any) {
            setRenameError(String(e?.message ?? e));
        } finally {
            setSaving(false);
        }
    };

    const handleInputKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Enter") {
            e.preventDefault();
            void commitRename();
        } else if (e.key === "Escape") {
            e.preventDefault();
            props.onClose();
        }
    };

    // Click-outside: close when clicking the backdrop, not the panel itself.
    const handleBackdropClick = (e: MouseEvent) => {
        if (e.target === e.currentTarget) {
            props.onClose();
        }
    };

    return (
        <div class="agent-focused-overlay" onClick={handleBackdropClick}>
            <div class="agent-focused-panel">
                <Show when={props.initialTab === "rename"}>
                    <div class="agent-focused-panel-header">
                        <span class="agent-focused-panel-title">Rename agent</span>
                        <button
                            class="agent-focused-panel-close"
                            onClick={() => props.onClose()}
                            title="Close (Esc)"
                        >
                            {"\u2715"}
                        </button>
                    </div>
                    <div class="agent-focused-panel-body agent-focused-rename">
                        <input
                            class={`agent-card-rename-input${renameError() ? " agent-card-rename-input--error" : ""}`}
                            type="text"
                            value={editName()}
                            onInput={(e) => {
                                setEditName(e.currentTarget.value);
                                setRenameError(null);
                            }}
                            onKeyDown={handleInputKeyDown}
                            disabled={saving()}
                            ref={(el) => setTimeout(() => el?.focus(), 0)}
                            aria-label="New agent name"
                        />
                        <Show when={renameError()}>
                            <span class="agent-focused-rename-error">{renameError()}</span>
                        </Show>
                        <div class="agent-focused-rename-actions">
                            <button
                                class="agent-focused-rename-btn agent-focused-rename-btn--confirm"
                                onClick={() => void commitRename()}
                                disabled={saving()}
                                type="button"
                            >
                                {saving() ? "Saving…" : "Save"}
                            </button>
                            <button
                                class="agent-focused-rename-btn agent-focused-rename-btn--cancel"
                                onClick={() => props.onClose()}
                                disabled={saving()}
                                type="button"
                            >
                                Cancel
                            </button>
                        </div>
                    </div>
                </Show>

                <Show when={props.initialTab === "forge" || props.initialTab === "identity"}>
                    <AgentCardSettingsPanel
                        blockId={props.blockId}
                        nodeModel={props.nodeModel}
                        agent={props.agent}
                        initialTab={props.initialTab === "forge" ? "forge" : "identity"}
                        onClose={props.onClose}
                    />
                </Show>
            </div>
        </div>
    );
};

AgentFocusedPanel.displayName = "AgentFocusedPanel";
