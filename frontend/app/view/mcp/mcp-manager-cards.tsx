// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * McpManager's cards and the details below them: the Add gallery's chips, a catalog server's card and its
 * details or form, and an observed server's card.
 */

import { Show, type JSX } from "solid-js";

import { Card, openMenuAt } from "@/app/element/ui";
import { McpObservedDetail, observedStatus } from "./mcp-in-use";
import { connectionSummary, reachLabel } from "./mcp-model";
import { rowSummary, sourceLabel } from "./mcp-observed";
import { findPreloadEntryByName, type McpPreloadEntry } from "./mcp-preload-catalog";
import { McpServerDetail, McpServerForm } from "./mcp-server-detail";
import type { McpActions } from "./mcp-manager-actions";
import { NEW_KEY, type McpState } from "./mcp-manager-state";

const Icon = (props: { name: string }) => <i class={`fa-solid fa-${props.name}`} />;

export const presetChip = (s: McpState, act: McpActions, entry: McpPreloadEntry) => {
    const added = () => s.catalogNames().has(entry.name);
    return (
        <Card
            size="chip"
            class="mcp-preset-chip"
            data-preset={entry.id}
            icon={<Icon name={entry.icon} />}
            title={entry.name}
            subtitle={added() ? "Added" : "Add"}
            aria-label={added() ? `${entry.name}, added: open it` : `Add ${entry.name}`}
            onSelect={() => act.pickPreset(entry)}
        />
    );
};

export const customChip = (act: McpActions) => (
    <Card
        size="chip"
        class="mcp-preset-chip"
        data-preset="custom"
        icon={<Icon name="plus" />}
        title="Custom server"
        subtitle="Any command or URL"
        aria-label="Add a custom server"
        onSelect={act.startNew}
    />
);

export const serverCard = (s: McpState, act: McpActions) => (key: string) => (
    <Show
        when={key !== NEW_KEY}
        fallback={
            <Card
                class="mcp-server-card"
                aria-label="New MCP server"
                icon={<Icon name="plus" />}
                title={s.newDraft()?.name.trim() || "New server"}
                subtitle="Not saved yet"
                selected
                onSelect={() => s.model.cancelDraft()}
            />
        }
    >
        <Show when={s.serverById().get(key)}>
            {(server) => {
                const status = () => observedStatus(s.usesFor(server().name), s.notLoaded(server().name));
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
                        selected={s.catalogOpen() === key}
                        onSelect={() => act.toggleServer(server())}
                        onContextMenu={(e: MouseEvent) => {
                            e.preventDefault();
                            openMenuAt(act.serverMenu(server()), e);
                        }}
                    />
                );
            }}
        </Show>
    </Show>
);

export const serverDetail = (s: McpState, act: McpActions) => (key: string) => (
    <Show
        when={key !== NEW_KEY}
        fallback={<Show when={s.newDraft()}>{(draft) => <McpServerForm model={s.model} draft={draft()} />}</Show>}
    >
        <Show when={s.serverById().get(key)}>
            {(server) => (
                <Show
                    when={s.model.draftAtom()?.id === key ? s.model.draftAtom() : null}
                    fallback={
                        <McpServerDetail
                            model={s.model}
                            server={server()}
                            notLoaded={s.notLoaded(server().name)}
                            uses={s.usesFor(server().name)}
                            onDelete={() => act.requestDelete(server())}
                        />
                    }
                >
                    {(draft) => <McpServerForm model={s.model} draft={draft()} />}
                </Show>
            )}
        </Show>
    </Show>
);

export const observedCard = (s: McpState, act: McpActions) => (key: string) => (
    <Show when={s.rowByKey().get(key)}>
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
                    selected={s.observedOpen() === key}
                    onSelect={() => act.toggleObserved(key)}
                />
            );
        }}
    </Show>
);

export const observedDetail = (s: McpState) => (key: string): JSX.Element => (
    <Show when={s.rowByKey().get(key)}>{(row) => <McpObservedDetail row={row()} />}</Show>
);
