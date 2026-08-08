// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * McpManager — the Armory's "MCP Servers" tab body. Context-free
 * list/create/edit/delete over the global MCP Server catalog
 * (mcp.catalog.* App API via McpCatalogModel). Every row here is global —
 * per-agent private servers live in the Agent-setup modal (AgentMcpModal),
 * not here.
 */

import { onCleanup, For, Show, type JSX } from "solid-js";
import { PrimitiveListDetail } from "@/app/element/primitive-list-detail";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import { McpCatalogModel } from "./mcp-model";
import { McpCatalogPicker } from "./McpCatalogPicker";
import { findPreloadEntryByName } from "./mcp-preload-catalog";
import { getMcpCapability, watchMcpCapability, type McpCapabilityStatus } from "@/app/store/mcp-capabilities";
import "../agent/components/AgentPrimitiveModal.scss";
import "./mcp-status-pill.scss";

const STATUS_LABEL: Record<McpCapabilityStatus, string> = {
    unknown: "Not checked",
    checking: "Checking…",
    connected: "Connected",
    unreachable: "Unreachable",
    handshake_failed: "Not responding",
    invalid_config: "Invalid config",
};

function McpStatusPill(props: { serverId: string; serverName: string }): JSX.Element {
    onCleanup(watchMcpCapability(props.serverId));
    const cap = () => getMcpCapability(props.serverId);
    // §6 acceptance bar: a non-"connected" status pairs with the catalog's
    // static remediation text (when this server was created from a known
    // catalog entry) instead of a bare status word.
    const remediation = () => {
        const s = cap().status;
        if (s === "unknown" || s === "checking" || s === "connected") return null;
        return findPreloadEntryByName(props.serverName)?.prereqNote ?? null;
    };
    // Unlike remediation() above, this is NOT gated on status — a risk that's
    // real once the server is connected and usable must stay visible exactly
    // then, not disappear the moment the server starts working (spec §4
    // policy #2; this is what Codex flagged on #2298).
    const riskNote = () => findPreloadEntryByName(props.serverName)?.riskNote ?? null;

    return (
        <div class="mcp-status-block">
            <span class="mcp-status-pill" classList={{ "is-connected": cap().status === "connected" }}>
                {STATUS_LABEL[cap().status]}
                <Show when={cap().status === "connected" && cap().toolCount !== undefined}>
                    {" "}· {cap().toolCount} tools
                </Show>
            </span>
            <Show when={remediation()}>
                <p class="mcp-status-remediation">{remediation()}</p>
            </Show>
            <Show when={riskNote()}>
                <p class="mcp-status-risk">⚠ {riskNote()}</p>
            </Show>
        </div>
    );
}

export const McpManager = (): JSX.Element => {
    const model = new McpCatalogModel();
    onCleanup(() => model.dispose());

    // Single-pane — see docs/specs/SPEC_ARMORY_RESPONSIVE_SINGLE_PANE_LAYOUT_2026_07_15.md.
    const inDetail = () => model.selectedIdAtom() !== null || model.draftAtom() !== null;
    const handleBack = () => {
        model.setError(null);
        model.setSelectedId(null);
        model.setDraft(null);
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
                            <span class="agent-primitive-modal-list-item-badge">
                                {server.bound_count} {server.bound_count === 1 ? "agent" : "agents"}
                            </span>
                        </button>
                    )}
                </For>
            </Show>

            <button class="agent-primitive-modal-new-btn" onClick={() => model.openCatalogPicker()}>
                + Browse catalog
            </button>
            <button class="agent-primitive-modal-new-btn" onClick={() => model.startNew()}>
                + New MCP server
            </button>
        </div>
    );

    const detailView = (
        <div class="agent-primitive-modal-detail">
            <Show when={model.errorAtom()}>
                <div class="agent-primitive-modal-error">{model.errorAtom()}</div>
            </Show>
            <Show
                when={model.draftAtom()}
                fallback={
                    <Show when={model.selectedAtom()}>
                        {(server) => (
                                    <div class="agent-primitive-modal-readonly">
                                        <h3 class="agent-primitive-modal-name">{server().name}</h3>
                                        <p class="agent-primitive-modal-global-note">
                                            Used by {server().bound_count} {server().bound_count === 1 ? "agent" : "agents"}
                                        </p>
                                        {/* Keyed specifically on id (a stable primitive, unlike the
                                            outer non-keyed Show's `server` object, which is a fresh
                                            reference on every refresh()) — remounts McpStatusPill, and
                                            so restarts its watchMcpCapability poll, exactly when the
                                            selected server actually changes, not on every list refresh. */}
                                        <Show when={server().id} keyed>
                                            {(id) => <McpStatusPill serverId={id} serverName={server().name} />}
                                        </Show>
                                        <span class="agent-primitive-modal-field-label">Transport</span>
                                        <pre class="agent-primitive-modal-field-value">{server().transport}</pre>
                                        <span class="agent-primitive-modal-field-label">Config</span>
                                        <pre class="agent-primitive-modal-field-value">{server().config}</pre>
                                        <span class="agent-primitive-modal-field-label">Bind to agent</span>
                                        <div class="agent-primitive-modal-bind-row">
                                            <select
                                                class="agent-primitive-modal-input"
                                                value={model.bindAgentIdAtom()}
                                                onChange={(e) => model.setBindAgentId(e.currentTarget.value)}
                                            >
                                                <option value="">Select an agent…</option>
                                                <For each={model.agentsAtom()}>
                                                    {(agent) => <option value={agent.id}>{agent.name}</option>}
                                                </For>
                                            </select>
                                            <button
                                                class="agent-primitive-modal-btn"
                                                disabled={!model.bindAgentIdAtom()}
                                                onClick={() => void model.bindToAgent(server().id, model.bindAgentIdAtom())}
                                            >
                                                Bind
                                            </button>
                                        </div>
                                        <div class="agent-primitive-modal-actions">
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
                                    onContextMenu={showTextInputContextMenu}
                                    placeholder="e.g. filesystem"
                                    required
                                />
                                <span class="agent-primitive-modal-field-label">Transport</span>
                                <input
                                    class="agent-primitive-modal-input"
                                    type="text"
                                    value={draft().transport}
                                    onInput={(e) => model.setDraft({ ...draft(), transport: e.currentTarget.value })}
                                    onContextMenu={showTextInputContextMenu}
                                    placeholder="stdio"
                                />
                                <span class="agent-primitive-modal-field-label">Config (JSON)</span>
                                <textarea
                                    class="agent-primitive-modal-textarea"
                                    rows={6}
                                    value={draft().config}
                                    onInput={(e) => model.setDraft({ ...draft(), config: e.currentTarget.value })}
                                    onContextMenu={showTextInputContextMenu}
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
    );

    return (
        <>
            <PrimitiveListDetail
                showDetail={inDetail()}
                backLabel="MCP Servers"
                onBack={handleBack}
                list={listView}
                detail={detailView}
            />
            <Show when={model.catalogPickerOpenAtom()}>
                <McpCatalogPicker model={model} />
            </Show>
        </>
    );
};

McpManager.displayName = "McpManager";
