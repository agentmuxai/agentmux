// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentMcpModal — the "MCP Servers" tab body inside AgentSetupModal.
 * List/create/edit/delete for this agent's own MCP servers, plus a
 * bound-state-aware Bind/Unbind toggle for global ones (driven by
 * `bound_to_agent` — see AgentMcpModel's doc comment).
 */

import { onCleanup, For, Show, type JSX } from "solid-js";
import { AgentMcpModel } from "../agent-mcp-model";
import "./AgentPrimitiveModal.scss";

interface AgentMcpModalProps {
    agentId: string;
}

export const AgentMcpModal = (props: AgentMcpModalProps): JSX.Element => {
    const model = new AgentMcpModel(props.agentId);
    onCleanup(() => model.dispose());

    return (
        <div class="agent-primitive-modal">
            <Show when={model.errorAtom()}>
                <div class="agent-primitive-modal-error">{model.errorAtom()}</div>
            </Show>

            <div class="agent-primitive-modal-body">
                <div class="agent-primitive-modal-list">
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
                                    <Show when={server.is_global}>
                                        <span class="agent-primitive-modal-list-item-badge">global</span>
                                        <span
                                            class="agent-primitive-modal-list-item-badge"
                                            classList={{ "is-bound": server.bound_to_agent }}
                                        >
                                            {server.bound_to_agent ? "bound" : "not bound"}
                                        </span>
                                    </Show>
                                </button>
                            )}
                        </For>
                    </Show>

                    <button class="agent-primitive-modal-new-btn" onClick={() => model.startNew()}>
                        + New MCP server
                    </button>
                </div>

                <div class="agent-primitive-modal-detail">
                    <Show
                        when={model.draftAtom()}
                        fallback={
                            <Show
                                when={model.selectedAtom()}
                                fallback={
                                    <div class="agent-primitive-modal-empty">
                                        Select a server from the list, or create a new one.
                                    </div>
                                }
                            >
                                {(server) => (
                                    <div class="agent-primitive-modal-readonly">
                                        <h3 class="agent-primitive-modal-name">{server().name}</h3>
                                        <Show when={server().is_global}>
                                            <p class="agent-primitive-modal-global-note">
                                                Global — managed in the Armory. You can bind/unbind it here.
                                            </p>
                                        </Show>
                                        <span class="agent-primitive-modal-field-label">Transport</span>
                                        <pre class="agent-primitive-modal-field-value">{server().transport}</pre>
                                        <span class="agent-primitive-modal-field-label">Config</span>
                                        <pre class="agent-primitive-modal-field-value">{server().config}</pre>
                                        <div class="agent-primitive-modal-actions">
                                            <Show
                                                when={!server().is_global}
                                                fallback={
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
                                                }
                                            >
                                                <button
                                                    class="agent-primitive-modal-btn agent-primitive-modal-btn-danger"
                                                    onClick={() => void model.deleteServer(server().id)}
                                                >
                                                    Delete
                                                </button>
                                                <button
                                                    class="agent-primitive-modal-btn"
                                                    onClick={() => model.startEdit(server())}
                                                >
                                                    Edit
                                                </button>
                                            </Show>
                                        </div>
                                    </div>
                                )}
                            </Show>
                        }
                    >
                        {(draft) => (
                            <form
                                class="agent-primitive-modal-form"
                                onSubmit={(e) => { e.preventDefault(); void model.saveDraft(); }}
                            >
                                <span class="agent-primitive-modal-field-label">Name *</span>
                                <input
                                    class="agent-primitive-modal-input"
                                    type="text"
                                    value={draft().name}
                                    onInput={(e) => model.setDraft({ ...draft(), name: e.currentTarget.value })}
                                    placeholder="e.g. filesystem"
                                    required
                                />
                                <span class="agent-primitive-modal-field-label">Transport</span>
                                <input
                                    class="agent-primitive-modal-input"
                                    type="text"
                                    value={draft().transport}
                                    onInput={(e) => model.setDraft({ ...draft(), transport: e.currentTarget.value })}
                                    placeholder="stdio"
                                />
                                <span class="agent-primitive-modal-field-label">Config (JSON)</span>
                                <textarea
                                    class="agent-primitive-modal-textarea"
                                    rows={6}
                                    value={draft().config}
                                    onInput={(e) => model.setDraft({ ...draft(), config: e.currentTarget.value })}
                                    spellcheck={false}
                                />
                                <div class="agent-primitive-modal-actions">
                                    <button
                                        type="button"
                                        class="agent-primitive-modal-btn"
                                        onClick={() => model.cancelDraft()}
                                        disabled={model.savingAtom()}
                                    >
                                        Cancel
                                    </button>
                                    <button
                                        type="submit"
                                        class="agent-primitive-modal-btn agent-primitive-modal-btn-primary"
                                        disabled={model.savingAtom() || !draft().name.trim()}
                                    >
                                        {model.savingAtom() ? "Saving…" : "Save"}
                                    </button>
                                </div>
                            </form>
                        )}
                    </Show>
                </div>
            </div>
        </div>
    );
};

AgentMcpModal.displayName = "AgentMcpModal";
