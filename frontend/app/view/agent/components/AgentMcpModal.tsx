// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentMcpModal — the "MCP Servers" tab body inside AgentStashModal. A
 * reactive, read-only list of every MCP server visible to this agent
 * (global + this agent's own, if any exist), plus a bound-state-aware
 * Bind/Unbind toggle for global ones (driven by `bound_to_agent` — see
 * AgentMcpModel's doc comment). Servers are authored in the Armory, not
 * here — see AgentMcpModel's doc comment for why this is no longer a
 * create/edit/delete surface.
 */

import { onCleanup, For, Show, type JSX } from "solid-js";
import { PrimitiveListDetail } from "@/app/element/primitive-list-detail";
import { openOrFocusPaneByView } from "@/app/store/global";
import { AgentMcpModel } from "../agent-mcp-model";
import "./AgentPrimitiveModal.scss";

interface AgentMcpModalProps {
    agentId: string;
}

export const AgentMcpModal = (props: AgentMcpModalProps): JSX.Element => {
    const model = new AgentMcpModel(props.agentId);
    onCleanup(() => model.dispose());

    // Single-pane — see docs/specs/SPEC_ARMORY_RESPONSIVE_SINGLE_PANE_LAYOUT_2026_07_15.md §5.
    const inDetail = () => model.selectedIdAtom() !== null;
    const handleBack = () => {
        model.setError(null);
        model.setSelectedId(null);
    };

    const listView = (
        <div class="agent-primitive-modal-list">
            <Show when={model.errorAtom()}>
                <div class="agent-primitive-modal-error">{model.errorAtom()}</div>
            </Show>
            <Show
                when={model.serversAtom().length > 0}
                fallback={<div class="agent-primitive-modal-list-empty">No MCP servers yet</div>}
            >
                <For each={model.serversAtom()}>
                    {(server) => (
                        <button
                            class="agent-primitive-modal-list-item"
                            classList={{ "is-selected": model.selectedIdAtom() === server.id }}
                            onClick={() => model.handleSelect(server)}
                        >
                            <span class="agent-primitive-modal-list-item-name">{server.name}</span>
                            <span
                                class="agent-primitive-modal-list-item-badge"
                                classList={{ "is-bound": server.bound_to_agent }}
                            >
                                {server.bound_to_agent ? "bound" : "not bound"}
                            </span>
                        </button>
                    )}
                </For>
            </Show>

            <button
                class="agent-primitive-modal-new-btn"
                onClick={() => void openOrFocusPaneByView("armory")}
            >
                Browse the Armory catalog →
            </button>
        </div>
    );

    const detailView = (
        <div class="agent-primitive-modal-detail">
            <Show when={model.errorAtom()}>
                <div class="agent-primitive-modal-error">{model.errorAtom()}</div>
            </Show>
            <Show when={model.selectedAtom()}>
                {(server) => (
                    <div class="agent-primitive-modal-readonly">
                        <h3 class="agent-primitive-modal-name">{server().name}</h3>
                        <p class="agent-primitive-modal-global-note">
                            Global — managed in the Armory. You can bind/unbind it here, or{" "}
                            <button
                                type="button"
                                class="agent-primitive-modal-link-btn"
                                onClick={() => void openOrFocusPaneByView("armory")}
                            >
                                edit it there
                            </button>
                            .
                        </p>
                        <span class="agent-primitive-modal-field-label">Transport</span>
                        <pre class="agent-primitive-modal-field-value">{server().transport}</pre>
                        <span class="agent-primitive-modal-field-label">Config</span>
                        <pre class="agent-primitive-modal-field-value">{server().config}</pre>
                        <p class="agent-primitive-modal-caveat">
                            Note: every global server is currently applied to every agent
                            regardless of bind state — unbinding here updates what shows as
                            "in use" but does not yet remove it from this agent's live config.
                        </p>
                        <div class="agent-primitive-modal-actions">
                            <Show
                                when={server().bound_to_agent}
                                fallback={
                                    <button
                                        class="agent-primitive-modal-btn agent-primitive-modal-btn-primary"
                                        onClick={() => void model.bind(server().id)}
                                    >
                                        Bind
                                    </button>
                                }
                            >
                                <button
                                    class="agent-primitive-modal-btn"
                                    onClick={() => void model.unbind(server().id)}
                                >
                                    Unbind
                                </button>
                            </Show>
                        </div>
                    </div>
                )}
            </Show>
        </div>
    );

    return (
        <PrimitiveListDetail
            showDetail={inDetail()}
            backLabel="MCP Servers"
            onBack={handleBack}
            list={listView}
            detail={detailView}
        />
    );
};

AgentMcpModal.displayName = "AgentMcpModal";
