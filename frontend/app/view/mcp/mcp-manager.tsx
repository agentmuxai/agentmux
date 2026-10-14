// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * McpManager — Connectors → MCP servers. The global MCP server catalog
 * (mcp.catalog.* MuxAPI via McpCatalogModel) and the servers running agents
 * report (McpObservedModel), as cards: an Add gallery of presets, "Your
 * servers" (the catalog) and "Also in use" (what agents run that isn't in
 * it), with a card's details and forms opening below its row
 * (docs/specs/SPEC_CONNECTORS_CARDS_2026_10_13.md §3.5). Per-agent private
 * servers live in the Agent-setup modal (AgentMcpModal), not here.
 */

import { createMemo, createSignal, onCleanup, Show, type JSX } from "solid-js";

import { ConfirmModal } from "@/app/element/confirm-modal";
import { Button, Card, CardGrid, FilterInput } from "@/app/element/ui";
import { ContextMenuModel } from "@/app/store/contextmenu";
import { matchesEveryWord } from "@/app/util/fuzzysearch";
import type { McpServerCatalogItem } from "@/types/rpc/McpServerCatalogItem";
import { McpObservedDetail, observedStatus } from "./mcp-in-use";
import { connectionSummary, McpCatalogModel, reachLabel } from "./mcp-model";
import { McpObservedModel, notLoadedBy, notLoadedText, rowSummary, sourceLabel, type ObservedServerRow } from "./mcp-observed";
import { findPreloadEntryByName, MCP_PRELOAD_CATALOG, type McpPreloadEntry } from "./mcp-preload-catalog";
import { McpServerDetail, McpServerForm } from "./mcp-server-detail";
import "./mcp-cards.scss";

/** The card of a server being added, first in "Your servers" while its form is open. */
const NEW_KEY = "\u0000new";
const CUSTOM_KEY = "\u0000custom";

const observedKey = (row: ObservedServerRow) => `${row.source}\u0000${row.name}`;

const Icon = (props: { name: string }) => <i class={`fa-solid fa-${props.name}`} />;

export const McpManager = (): JSX.Element => {
    const model = new McpCatalogModel();
    onCleanup(() => model.dispose());
    const observed = new McpObservedModel();
    onCleanup(() => observed.dispose());

    const [filter, setFilter] = createSignal("");
    // One card is open in the section: a catalog server's (in the model) or
    // an observed server's (here).
    const [observedOpen, setObservedOpen] = createSignal<string | null>(null);
    const [deleting, setDeleting] = createSignal<McpServerCatalogItem | null>(null);
    const [deletingBound, setDeletingBound] = createSignal<string[] | null>(null);

    // Cards are keyed by id, so a refreshed list keeps them (and an open form).
    const serverById = createMemo(() => new Map(model.serversAtom().map((s) => [s.id, s])));
    const catalogNames = createMemo(() => new Set(model.serversAtom().map((s) => s.name)));
    const usesFor = (name: string) =>
        observed
            .rowsAtom()
            .filter((r) => r.name === name)
            .flatMap((r) => r.uses);
    /** Why a catalog server isn't on every agent it should be: they started first. */
    const notLoaded = (name: string) => notLoadedText(notLoadedBy(observed.agentsAtom(), name));
    const otherRows = createMemo(() => observed.rowsAtom().filter((r) => !catalogNames().has(r.name)));
    const rowByKey = createMemo(() => new Map(otherRows().map((r) => [observedKey(r), r])));

    const newDraft = () => {
        const d = model.draftAtom();
        return d && !d.id ? d : null;
    };
    const serverKeys = createMemo(() => {
        const keys = model
            .serversAtom()
            .filter((s) => matchesEveryWord(filter(), s.name, connectionSummary(s.transport, s.config)))
            .map((s) => s.id);
        return newDraft() ? [NEW_KEY, ...keys] : keys;
    });
    const otherKeys = createMemo(() =>
        otherRows()
            .filter((r) => matchesEveryWord(filter(), r.name, sourceLabel(r.source)))
            .map(observedKey),
    );
    const catalogOpen = () => (newDraft() ? NEW_KEY : model.selectedIdAtom());
    const count = () => model.serversAtom().length + otherRows().length;
    const hasAny = () => count() > 0 || newDraft() !== null;

    const closeCatalog = () => {
        model.setError(null);
        model.setDraft(null);
        model.setSelectedId(null);
    };
    const toggleServer = (server: McpServerCatalogItem) => {
        setObservedOpen(null);
        if (catalogOpen() === server.id) closeCatalog();
        else model.handleSelect(server);
    };
    const toggleObserved = (key: string) => {
        if (observedOpen() === key) {
            setObservedOpen(null);
            return;
        }
        closeCatalog();
        setObservedOpen(key);
    };
    const startNew = () => {
        setObservedOpen(null);
        model.startNew();
    };
    // A preset already in the catalog opens its card instead of a second copy.
    const pickPreset = (entry: McpPreloadEntry) => {
        setObservedOpen(null);
        const existing = model.serversAtom().find((s) => s.name === entry.name);
        if (existing) {
            setFilter("");
            model.handleSelect(existing);
        } else {
            model.startFromCatalog(entry);
        }
    };
    const requestDelete = (server: McpServerCatalogItem) => {
        setDeleting(server);
        setDeletingBound(null);
        void model.boundAgentNames(server).then((names) => {
            if (deleting()?.id === server.id) setDeletingBound(names);
        });
    };

    const serverMenu = (server: McpServerCatalogItem): ContextMenuItem[] => {
        const agents = model.agentsAtom();
        return [
            {
                label: "Edit",
                click: () => {
                    setObservedOpen(null);
                    model.startEdit(server);
                },
            },
            agents.length === 0
                ? { label: "Bind to agent", sublabel: "no agents", enabled: false }
                : {
                      label: "Bind to agent",
                      type: "submenu",
                      submenu: agents.map((a) => ({
                          label: a.name,
                          click: () => void model.bindToAgent(server.id, a.id),
                      })),
                  },
            // Last, after a separator, so it is never under the pointer.
            { type: "separator" },
            { label: "Delete…", click: () => requestDelete(server) },
        ];
    };

    const presetChip = (entry: McpPreloadEntry) => {
        const added = () => catalogNames().has(entry.name);
        return (
            <Card
                size="chip"
                class="mcp-preset-chip"
                data-preset={entry.id}
                icon={<Icon name={entry.icon} />}
                title={entry.name}
                subtitle={added() ? "Added" : "Add"}
                aria-label={added() ? `${entry.name}, added: open it` : `Add ${entry.name}`}
                onSelect={() => pickPreset(entry)}
            />
        );
    };

    const serverCard = (key: string) => (
        <Show
            when={key !== NEW_KEY}
            fallback={
                <Card
                    class="mcp-server-card"
                    aria-label="New MCP server"
                    icon={<Icon name="plus" />}
                    title={newDraft()?.name.trim() || "New server"}
                    subtitle="Not saved yet"
                    selected
                    onSelect={() => model.cancelDraft()}
                />
            }
        >
            <Show when={serverById().get(key)}>
                {(server) => {
                    const status = () => observedStatus(usesFor(server().name), notLoaded(server().name));
                    return (
                        <Card
                            class="mcp-server-card"
                            data-mcp-server={server().name}
                            aria-label={server().name}
                            icon={<Icon name={findPreloadEntryByName(server().name)?.icon ?? "plug"} />}
                            title={server().name}
                            subtitle={connectionSummary(server().transport, server().config)}
                            meta={[
                                <span>{reachLabel(server())}</span>,
                                ...(server().in_containers ? [<span>Containers too</span>] : []),
                            ]}
                            status={status()}
                            attention={status().tone === "error"}
                            selected={catalogOpen() === key}
                            onSelect={() => toggleServer(server())}
                            onContextMenu={(e: MouseEvent) => {
                                e.preventDefault();
                                ContextMenuModel.showContextMenu(serverMenu(server()), e);
                            }}
                        />
                    );
                }}
            </Show>
        </Show>
    );

    const serverDetail = (key: string) => (
        <Show
            when={key !== NEW_KEY}
            fallback={<Show when={newDraft()}>{(draft) => <McpServerForm model={model} draft={draft()} />}</Show>}
        >
            <Show when={serverById().get(key)}>
                {(server) => (
                    <Show
                        when={model.draftAtom()?.id === key ? model.draftAtom() : null}
                        fallback={
                            <McpServerDetail
                                model={model}
                                server={server()}
                                notLoaded={notLoaded(server().name)}
                                uses={usesFor(server().name)}
                                onDelete={() => requestDelete(server())}
                            />
                        }
                    >
                        {(draft) => <McpServerForm model={model} draft={draft()} />}
                    </Show>
                )}
            </Show>
        </Show>
    );

    const observedCard = (key: string) => (
        <Show when={rowByKey().get(key)}>
            {(row) => {
                const status = () => observedStatus(row().uses);
                return (
                    <Card
                        class="mcp-observed-card"
                        data-mcp-observed={row().name}
                        aria-label={row().name}
                        icon={<Icon name="plug" />}
                        title={row().name}
                        subtitle={sourceLabel(row().source)}
                        meta={[<span>{rowSummary(row())}</span>]}
                        status={status()}
                        attention={status().tone === "error"}
                        selected={observedOpen() === key}
                        onSelect={() => toggleObserved(key)}
                    />
                );
            }}
        </Show>
    );

    const addButton = (
        <Button tone="accent" icon="plus" class="mcp-cards-add" onClick={startNew}>
            Add server
        </Button>
    );

    return (
        <div class="ui-card-section ui-density-compact mcp-cards">
            <div class="ui-card-section-header">
                <span class="ui-card-section-title">MCP servers</span>
                <span class="ui-card-section-count">{count()}</span>
                <FilterInput
                    class="ui-card-section-filter"
                    value={filter()}
                    onInput={setFilter}
                    placeholder="Filter servers"
                />
                {addButton}
            </div>

            <Show when={model.errorAtom()}>
                <div class="mcp-cards-error" role="alert">
                    {model.errorAtom()}
                </div>
            </Show>

            <CardGrid
                size="chip"
                class="mcp-add-gallery"
                ariaLabel="Add an MCP server"
                items={[...MCP_PRELOAD_CATALOG.map((e) => e.id), CUSTOM_KEY]}
                keyOf={(k) => k}
                renderCard={(k) => (
                    <Show
                        when={MCP_PRELOAD_CATALOG.find((e) => e.id === k)}
                        fallback={
                            <Card
                                size="chip"
                                class="mcp-preset-chip"
                                data-preset="custom"
                                icon={<Icon name="plus" />}
                                title="Custom server"
                                subtitle="Any command or URL"
                                aria-label="Add a custom server"
                                onSelect={startNew}
                            />
                        }
                    >
                        {(entry) => presetChip(entry())}
                    </Show>
                )}
            />

            <Show
                when={hasAny()}
                fallback={
                    <div class="ui-card-empty mcp-cards-empty">
                        <span>No MCP servers yet. Add one to give your agents its tools.</span>
                        <Button tone="accent" icon="plus" onClick={startNew}>
                            Add server
                        </Button>
                    </div>
                }
            >
                <Show when={serverKeys().length > 0}>
                    <h4 class="ui-card-group-heading">Your servers</h4>
                    <CardGrid
                        class="mcp-servers-grid"
                        ariaLabel="Your servers"
                        items={serverKeys()}
                        keyOf={(k) => k}
                        renderCard={serverCard}
                        selectedKey={catalogOpen()}
                        renderDetail={serverDetail}
                        onClose={closeCatalog}
                    />
                </Show>
                <Show when={otherKeys().length > 0}>
                    <h4 class="ui-card-group-heading">Also in use</h4>
                    <CardGrid
                        class="mcp-observed-grid"
                        ariaLabel="Also in use"
                        items={otherKeys()}
                        keyOf={(k) => k}
                        renderCard={observedCard}
                        selectedKey={observedOpen()}
                        renderDetail={(k) => (
                            <Show when={rowByKey().get(k)}>{(row) => <McpObservedDetail row={row()} />}</Show>
                        )}
                        onClose={() => setObservedOpen(null)}
                    />
                </Show>
                <Show when={filter() && serverKeys().length === 0 && otherKeys().length === 0}>
                    <div class="ui-card-empty">No servers match “{filter()}”.</div>
                </Show>
            </Show>

            <Show when={deleting()}>
                {(server) => (
                    <ConfirmModal
                        open={true}
                        scope="tab"
                        title={`Delete ${server().name}?`}
                        confirmLabel="Delete server"
                        destructive
                        onConfirm={async () => {
                            await model.deleteServer(server().id);
                            setDeleting(null);
                        }}
                        onCancel={() => setDeleting(null)}
                    >
                        <DeleteDetails
                            server={server()}
                            boundNames={deletingBound()}
                            runningIn={usesFor(server().name).map((u) => u.agent)}
                        />
                    </ConfirmModal>
                )}
            </Show>
        </div>
    );
};

McpManager.displayName = "McpManager";

/** What deleting a catalog server takes away, and from whom. */
function DeleteDetails(props: {
    server: McpServerCatalogItem;
    /** Null while they're being looked up. */
    boundNames: string[] | null;
    runningIn: string[];
}): JSX.Element {
    const bound = () => {
        const names = props.boundNames;
        if (names && names.length > 0) return `Bound to ${names.join(", ")}.`;
        if (props.server.bound_count > 0) return `Bound to ${reachLabel({ ...props.server, all_agents: false })}.`;
        return null;
    };
    const running = () => [...new Set(props.runningIn)];
    return (
        <div class="mcp-delete-confirm">
            <p>AgentMux removes it from the catalog. Agents stop getting it the next time they start.</p>
            <Show when={props.server.all_agents}>
                <p>Every agent gets it now.</p>
            </Show>
            <Show when={bound()}>{(text) => <p>{text()}</p>}</Show>
            <Show when={running().length > 0}>
                <p>Running now in {running().join(", ")}, which keep it until they restart.</p>
            </Show>
        </div>
    );
}
