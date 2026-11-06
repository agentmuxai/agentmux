// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** McpManager's page: the header with its filter, the Add gallery, the two groups of cards and the empty states. */

import { Show, type JSX } from "solid-js";

import { Button, CardGrid, EmptyState, FilterInput, Notice, SectionHeader } from "@/app/element/ui";
import { customChip, observedCard, observedDetail, presetChip, serverCard, serverDetail } from "./mcp-manager-cards";
import type { McpActions } from "./mcp-manager-actions";
import { CUSTOM_KEY, type McpState } from "./mcp-manager-state";
import { MCP_PRELOAD_CATALOG } from "./mcp-preload-catalog";

interface McpViewProps {
    s: McpState;
    act: McpActions;
}

function McpHeader(props: McpViewProps): JSX.Element {
    const { s, act } = props;
    const addButton = (
        <Button tone="accent" icon="plus" class="mcp-cards-add" onClick={act.startNew}>
            Add server
        </Button>
    );
    return (
        <SectionHeader title="MCP servers" count={s.count()}>
            <FilterInput
                class="ui-card-section-filter"
                value={s.filter()}
                onInput={s.setFilter}
                placeholder="Filter servers"
            />
            {addButton}
        </SectionHeader>
    );
}

function McpAddGallery(props: McpViewProps): JSX.Element {
    return (
        <CardGrid
            size="chip"
            class="mcp-add-gallery"
            ariaLabel="Add an MCP server"
            items={[...MCP_PRELOAD_CATALOG.map((e) => e.id), CUSTOM_KEY]}
            keyOf={(k) => k}
            renderCard={(k) => (
                <Show when={MCP_PRELOAD_CATALOG.find((e) => e.id === k)} fallback={customChip(props.act)}>
                    {(entry) => presetChip(props.s, props.act, entry())}
                </Show>
            )}
        />
    );
}

function McpGroups(props: McpViewProps): JSX.Element {
    const { s, act } = props;
    return (
        <>
            <Show when={s.serverKeys().length > 0}>
                <SectionHeader variant="group" level={4} title="Your servers" />
                <CardGrid
                    class="mcp-servers-grid"
                    ariaLabel="Your servers"
                    items={s.serverKeys()}
                    keyOf={(k) => k}
                    renderCard={serverCard(s, act)}
                    selectedKey={s.catalogOpen()}
                    renderDetail={serverDetail(s, act)}
                    onClose={act.closeCatalog}
                />
            </Show>
            <Show when={s.otherKeys().length > 0}>
                <SectionHeader variant="group" level={4} title="Also in use" />
                <CardGrid
                    class="mcp-observed-grid"
                    ariaLabel="Also in use"
                    items={s.otherKeys()}
                    keyOf={(k) => k}
                    renderCard={observedCard(s, act)}
                    selectedKey={s.observedOpen()}
                    renderDetail={observedDetail(s)}
                    onClose={() => s.setObservedOpen(null)}
                />
            </Show>
            <Show when={s.filter() && s.serverKeys().length === 0 && s.otherKeys().length === 0}>
                <EmptyState>No servers match “{s.filter()}”.</EmptyState>
            </Show>
        </>
    );
}

export function McpView(props: McpViewProps): JSX.Element {
    const { s, act } = props;
    return (
        <>
            <McpHeader s={s} act={act} />

            <Show when={s.model.errorAtom()}>
                <Notice tone="danger">{s.model.errorAtom()}</Notice>
            </Show>

            <McpAddGallery s={s} act={act} />

            <Show
                when={s.hasAny()}
                fallback={
                    <EmptyState
                        actions={
                            <Button tone="accent" icon="plus" onClick={act.startNew}>
                                Add server
                            </Button>
                        }
                    >
                        No MCP servers yet. Add one to give your agents its tools.
                    </EmptyState>
                }
            >
                <McpGroups s={s} act={act} />
            </Show>
        </>
    );
}
