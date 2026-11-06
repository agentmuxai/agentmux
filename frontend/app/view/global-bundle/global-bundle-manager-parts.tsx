// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The parts of the Memory pane's Global section (global-bundle-manager.tsx lays them out): the tile grid, with
// one tile per entry, the CLAUDE.md reference, the delivered preview and the add tile, what was removed, and the
// full view of one tile with its header.

import { For, Show, type Accessor, type JSX } from "solid-js";

import { Button as UiButton } from "@/app/element/ui";
import { MemoryTile } from "@/app/view/native-memory/MemoryTile";
import { formatFileAge } from "@/app/view/native-memory/MemoryFileCard";
import { tokensTitle } from "@/app/view/native-memory/memory-tokens";
import { estimateTokenCount } from "@/util/format-count";
import { Button } from "@/element/button";
import { agentKindLabel, removedByLabel, utf8Bytes, type GlobalBundleViewModel, type GlobalMemoryView } from "./global-bundle-model";
import { GlobalMemoryFullView } from "./GlobalMemoryFullView";
import { GlobalMemoryImportBanner } from "./GlobalMemoryImportBanner";
import { GlobalMemoryTokenStrip } from "./GlobalMemoryTokenStrip";
import type { GlobalBundleManagerState } from "./global-bundle-manager-state";

function viewTitle(model: GlobalBundleViewModel, view: GlobalMemoryView): string {
    switch (view.kind) {
        case "new":
            return "New memory";
        case "claude-config":
            return "CLAUDE.md";
        case "preview":
            return `Delivered to a ${agentKindLabel(model.agentKindAtom())} agent`;
        case "entry":
            return model.openEntryAtom()?.name ?? "";
    }
}

/** One tile per entry (system-tier first, then ordinary in injection order). */
function EntryTiles(part: { s: GlobalBundleManagerState }): JSX.Element {
    const { model, memoryEntries, deliveredTokens, entryMeta, openView, dragId, dropTargetId, dragHandlers } = part.s;
    // See the top-of-file comment in global-bundle-manager.tsx for why they're one set, not two.
    return (
        <For each={memoryEntries()}>
            {(entry) => (
                <MemoryTile
                    icon={entry.is_system ? "fa-shield-halved" : "fa-file-lines"}
                    title={entry.name}
                    monoTitle={false}
                    badges={entry.is_system ? [{ label: "system", variant: "accent" }] : []}
                    tokens={deliveredTokens(entry)}
                    tokensTitle={
                        deliveredTokens(entry) != null
                            ? tokensTitle(
                                  deliveredTokens(entry) ?? 0,
                                  model.tokensByIdAtom().get(entry.id)?.size_bytes,
                              )
                            : undefined
                    }
                    meta={entryMeta(entry)}
                    onSelect={() => openView({ kind: "entry", id: entry.id })}
                    testId="global-memory-tile"
                    data={{ id: entry.id, kind: entry.is_system ? "system" : "entry" }}
                    classList={{
                        "memory-file-card--dragging": dragId() === entry.id,
                        "memory-file-card--drop-target": dropTargetId() === entry.id,
                    }}
                    {...dragHandlers(entry)}
                />
            )}
        </For>
    );
}

/** Read-only reference display of the CLAUDE.md in the isolated config dir a default spawned agent actually
 *  launches with (CLAUDE_CONFIG_DIR). NOT part of AgentMux's own Global Memory composition. See
 *  docs/specs/SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md §7. */
function ClaudeConfigTile(part: { s: GlobalBundleManagerState }): JSX.Element {
    const { model, openView } = part.s;
    return (
        <Show when={model.claudeGlobalConfigAtom()}>
            {(cfg) => (
                <MemoryTile
                    icon="fa-file-code"
                    title="CLAUDE.md"
                    badges={[{ label: "read-only", icon: "fa-lock", variant: "accent" }]}
                    tokens={cfg().exists ? estimateTokenCount(cfg().content ?? "") : undefined}
                    tokensTitle={
                        cfg().exists
                            ? tokensTitle(estimateTokenCount(cfg().content ?? ""), utf8Bytes(cfg().content ?? ""))
                            : undefined
                    }
                    meta={cfg().exists ? "provider config" : "No file yet · provider config"}
                    onSelect={() => openView({ kind: "claude-config" })}
                    testId="global-memory-tile"
                    data={{ kind: "claude-config" }}
                    ariaLabel={`CLAUDE.md (read-only) — ${cfg().path}`}
                />
            )}
        </Show>
    );
}

/** What a chosen kind of agent is delivered, and how many entries that is. */
function PreviewTile(part: { s: GlobalBundleManagerState }): JSX.Element {
    const { model, memoryEntries, openView } = part.s;
    return (
        <MemoryTile
            icon="fa-layer-group"
            title={`Delivered to a ${agentKindLabel(model.agentKindAtom())} agent`}
            monoTitle={false}
            tokens={model.deliveredTotalsAtom()?.total}
            tokensTitle={
                model.deliveredTotalsAtom()
                    ? tokensTitle(model.deliveredTotalsAtom()?.total ?? 0, utf8Bytes(model.previewAtom()))
                    : undefined
            }
            meta={(() => {
                const n = model.deliveredTotalsAtom()?.count ?? memoryEntries().length;
                return `${n} ${n === 1 ? "entry" : "entries"}`;
            })()}
            onSelect={() => openView({ kind: "preview" })}
            classList={{ "memory-file-card--action": true }}
            testId="global-memory-tile"
            data={{ kind: "preview" }}
        />
    );
}

function AddTile(part: { s: GlobalBundleManagerState }): JSX.Element {
    const { openView } = part.s;
    return (
        <MemoryTile
            icon="fa-plus"
            title="+ Add memory"
            monoTitle={false}
            meta="Appended to the end of the order"
            onSelect={() => openView({ kind: "new" })}
            classList={{ "memory-file-card--action": true }}
            testId="global-memory-tile"
            data={{ kind: "add" }}
        />
    );
}

/** What was taken out of Global Memory: kept in history, so it can come back. */
function RemovedSection(part: { s: GlobalBundleManagerState }): JSX.Element {
    const { model } = part.s;
    return (
        <Show when={model.removedAtom().length > 0}>
            <section class="global-bundle-removed" data-testid="global-memory-removed">
                <h3 class="global-bundle-removed-title">Removed</h3>
                <p class="global-bundle-removed-hint">
                    Taken out of Global Memory and kept in history. Restoring puts an entry back at the end of the
                    order.
                </p>
                <For each={model.removedAtom()}>
                    {(entry) => (
                        <div class="global-bundle-removed-row" data-id={entry.id}>
                            <span class="global-bundle-removed-name">{entry.name}</span>
                            <span class="global-bundle-removed-meta">
                                {`Removed by ${removedByLabel(entry.removed_by)} · ${formatFileAge(entry.removed_at)}`}
                            </span>
                            <Button onClick={() => void model.restore(entry)}>Restore</Button>
                        </div>
                    )}
                </For>
            </section>
        </Show>
    );
}

export function MemoryGrid(part: { s: GlobalBundleManagerState }): JSX.Element {
    const { model, memoryEntries } = part.s;
    return (
        <div class="native-memory-manager-file-grid-view global-bundle-grid-view">
            <p class="global-bundle-intro">
                Every agent inherits this at launch — takes effect after a restart. Drag entries to change their
                order.
            </p>

            <GlobalMemoryTokenStrip model={model} />

            <GlobalMemoryImportBanner onImported={() => void model.refresh()} />

            <Show when={model.errorAtom()}>
                <div class="global-bundle-error">{model.errorAtom()}</div>
            </Show>

            <div class="native-memory-manager-file-grid global-bundle-grid">
                <EntryTiles s={part.s} />
                <ClaudeConfigTile s={part.s} />
                <PreviewTile s={part.s} />
                <AddTile s={part.s} />
            </div>

            <Show when={memoryEntries().length === 0}>
                <div class="global-bundle-empty">No memories yet.</div>
            </Show>

            <RemovedSection s={part.s} />
        </div>
    );
}

/** One tile opened: the Personal Memory back/breadcrumb header, one level deep here, and its full view. */
export function FullViewPane(part: { s: GlobalBundleManagerState; view: Accessor<GlobalMemoryView> }): JSX.Element {
    const { model, back } = part.s;
    const view = part.view;
    return (
        <div class="native-memory-manager-detail global-bundle-detail">
            {/* The Personal Memory back/breadcrumb header
                (native-memory-manager.tsx), one level deep here. */}
            <div class="native-memory-manager-header">
                <UiButton class="native-memory-manager-back" density="compact" onClick={back}>
                    ← All global memory
                </UiButton>
                <span class="native-memory-manager-crumbs">
                    <span class="native-memory-manager-agent-name">Global Memory</span>
                    <span class="native-memory-manager-crumb-sep" aria-hidden="true">
                        {"·"}
                    </span>
                    <span class="native-memory-manager-filename global-bundle-crumb-name">
                        {viewTitle(model, view())}
                    </span>
                </span>
            </div>
            <Show when={model.errorAtom()}>
                <div class="global-bundle-error">{model.errorAtom()}</div>
            </Show>
            <div class="native-memory-manager-body native-memory-manager-body--pinned">
                <GlobalMemoryFullView model={model} />
            </div>
        </div>
    );
}
