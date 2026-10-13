// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A catalog MCP server's details, and the form that adds or edits one, as
// they open below its card in Connectors → MCP servers
// (SPEC_CONNECTORS_CARDS_2026_10_13.md §3.4, §3.5).

import { createUniqueId, onCleanup, Show, type JSX } from "solid-js";

import { Button, Field, Select, StatusDot, Switch, TextInput, type StatusTone } from "@/app/element/ui";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import { getMcpCapability, watchMcpCapability, type McpCapabilityStatus } from "@/app/store/mcp-capabilities";
import type { McpServerCatalogItem } from "@/types/rpc/McpServerCatalogItem";
import { focusOnOpen } from "@/util/focusutil";
import { McpUsesList } from "./mcp-in-use";
import { reachLabel, type McpCatalogModel, type McpDraft } from "./mcp-model";
import type { ServerUse } from "./mcp-observed";
import { findPreloadEntryByName } from "./mcp-preload-catalog";

const PROBE: Record<McpCapabilityStatus, { tone: StatusTone; label: string }> = {
    unknown: { tone: "idle", label: "Not checked" },
    checking: { tone: "busy", label: "Checking…" },
    connected: { tone: "ok", label: "Connected" },
    unreachable: { tone: "error", label: "Unreachable" },
    handshake_failed: { tone: "error", label: "Not responding" },
    invalid_config: { tone: "error", label: "Invalid config" },
};

/** The live probe, polled while this is mounted: only while a card is open. */
function McpProbeStatus(props: { serverId: string; serverName: string }): JSX.Element {
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
    const probe = () => PROBE[cap().status];

    return (
        <div class="mcp-probe">
            <span class="mcp-probe-line" data-status={cap().status}>
                <StatusDot tone={probe().tone} label={probe().label} />
                <span>
                    {probe().label}
                    <Show when={cap().status === "connected" && cap().toolCount !== undefined}>
                        {" "}· {cap().toolCount} tools
                    </Show>
                </span>
            </span>
            <Show when={remediation()}>
                <p class="mcp-probe-remediation">{remediation()}</p>
            </Show>
            <Show when={riskNote()}>
                <p class="mcp-probe-risk">⚠ {riskNote()}</p>
            </Show>
        </div>
    );
}

export function McpServerDetail(props: {
    model: McpCatalogModel;
    server: McpServerCatalogItem;
    /** Why some agent that should have it doesn't, if one doesn't. */
    notLoaded: string | null;
    uses: ServerUse[];
    onDelete: () => void;
}): JSX.Element {
    const model = props.model;
    const server = () => props.server;
    const setReach = (patch: Partial<{ all_agents: boolean; in_containers: boolean }>) =>
        void model.setReach(server().id, {
            all_agents: server().all_agents,
            in_containers: server().in_containers,
            ...patch,
        });
    const agentOptions = () => [
        { value: "", label: "Select an agent…" },
        ...model.agentsAtom().map((a) => ({ value: a.id, label: a.name })),
    ];

    return (
        <div class="mcp-detail">
            <p class="mcp-detail-note">
                {server().all_agents ? "Every agent gets it" : `Bound to ${reachLabel(server())}`}
            </p>
            <Show when={props.notLoaded}>{(text) => <p class="mcp-not-loaded">{text()}</p>}</Show>
            <Field label="All agents" description="Every agent gets it at launch, not only the ones it is bound to.">
                <Switch checked={server().all_agents} onChange={(on) => setReach({ all_agents: on })} />
            </Field>
            <Field
                label="Containers too"
                description="Container agents get it as well. Leave off for a server that runs a program on this computer."
            >
                <Switch checked={server().in_containers} onChange={(on) => setReach({ in_containers: on })} />
            </Field>
            {/* Keyed on the id, not the server object (a new one on every
                refresh), so the probe restarts only for another server. */}
            <Show when={server().id} keyed>
                {(id) => <McpProbeStatus serverId={id} serverName={server().name} />}
            </Show>
            <Show when={props.uses.length > 0}>
                <McpUsesList uses={props.uses} />
            </Show>
            <span class="mcp-detail-label">Transport</span>
            <pre class="mcp-detail-value">{server().transport}</pre>
            <span class="mcp-detail-label">Config</span>
            <pre class="mcp-detail-value">{server().config}</pre>
            <Field label="Bind to agent" layout="stacked">
                <div class="mcp-detail-row">
                    <Select
                        value={model.bindAgentIdAtom()}
                        onChange={(id) => model.setBindAgentId(id)}
                        options={agentOptions()}
                    />
                    <Button
                        disabled={!model.bindAgentIdAtom()}
                        onClick={() => void model.bindToAgent(server().id, model.bindAgentIdAtom())}
                    >
                        Bind
                    </Button>
                </div>
            </Field>
            <div class="mcp-detail-actions">
                <Button onClick={() => model.startEdit(server())}>Edit</Button>
                <Button tone="danger" onClick={() => props.onDelete()}>
                    Delete…
                </Button>
            </div>
        </div>
    );
}

/** The add or edit form, the same fields as before it moved below the card. */
export function McpServerForm(props: { model: McpCatalogModel; draft: McpDraft }): JSX.Element {
    const model = props.model;
    const draft = () => props.draft;
    const configId = `mcp-config-${createUniqueId()}`;
    return (
        <form
            class="mcp-detail mcp-server-form"
            aria-label={draft().id ? `Edit ${draft().name}` : "New MCP server"}
            onSubmit={(e) => {
                e.preventDefault();
                void model.saveDraft();
            }}
        >
            <Field label="Name" layout="stacked">
                <TextInput
                    value={draft().name}
                    onInput={(e) => model.setDraft({ ...draft(), name: e.currentTarget.value })}
                    onContextMenu={showTextInputContextMenu}
                    placeholder="e.g. filesystem"
                    required
                    ref={(el) => focusOnOpen(el, { select: true })}
                />
            </Field>
            <Field label="Transport" layout="stacked">
                <TextInput
                    value={draft().transport}
                    onInput={(e) => model.setDraft({ ...draft(), transport: e.currentTarget.value })}
                    onContextMenu={showTextInputContextMenu}
                    placeholder="stdio"
                />
            </Field>
            <Field label="Config (JSON)" layout="stacked" id={configId}>
                <textarea
                    id={configId}
                    class="ui-input mcp-config-input"
                    rows={6}
                    value={draft().config}
                    onInput={(e) => model.setDraft({ ...draft(), config: e.currentTarget.value })}
                    onContextMenu={showTextInputContextMenu}
                    spellcheck={false}
                />
            </Field>
            <div class="mcp-detail-actions">
                <Button onClick={() => model.cancelDraft()} disabled={model.savingAtom()}>
                    Cancel
                </Button>
                <Button type="submit" tone="accent" disabled={model.savingAtom() || !draft().name.trim()}>
                    {model.savingAtom() ? "Saving…" : "Save"}
                </Button>
            </div>
        </form>
    );
}
