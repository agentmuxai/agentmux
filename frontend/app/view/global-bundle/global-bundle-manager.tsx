// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// GlobalBundleManager — the Global section of the Armory "Memory" tab (moves to
// the Bundles tab in Phase 4 of SPEC_ARMORY_NAMING_CONSOLIDATION_2026_09_09.md).
// Presents the workspace-wide global bundles (is_global rows) as an ordered
// set of memories that compose into every agent's startup instructions file
// (CLAUDE.md, GEMINI.md, or similar, depending on provider) at launch.
//
// Context-free: owns its own GlobalBundleViewModel and drives off the
// bundle_* RPCs. Spec: docs/specs/archive/SPEC_TRUST_CENTER_GLOBAL_BRAIN_2026_06_19.md.
//
// Layout restructured three times: per docs/specs/SPEC_ARMORY_GLOBAL_MEMORY_
// DECLUTTER_2026_09_15.md (one consistently-shaped file list), per
// docs/specs/SPEC_GLOBAL_MEMORY_UNIFY_SYSTEM_AND_ORDINARY_2026_09_15.md (the
// system tier and ordinary entries render as one list — the backend's
// write-path isolation between them is real and unchanged; only the
// presentation unified), and — since 2026-09-24 — per
// docs/specs/SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.3: TILES FIRST,
// then expand to full, matching an agent's Personal Memory file tiles
// (MemoryTile, the same grid and tile CSS). The always-visible 240px preview
// per card and the inline card editors are gone; a tile opens
// GlobalMemoryFullView with the Personal Memory back/breadcrumb header. The
// full view has history (globalmemory:history/diff/revert), an editor pinned
// to the bottom, and dirty-draft protection. Creating a NEW system-tier entry
// still has no UI trigger; existing ones remain editable/removable via their
// own still-isolated RPCs.

import { createSignal, For, onCleanup, Show, type JSX } from "solid-js";
import type { Bundle } from "@/app/store/rpc-api";
import { formatFileAge, formatFileSize } from "@/app/view/native-memory/MemoryFileCard";
import { MemoryTile } from "@/app/view/native-memory/MemoryTile";
import "@/app/view/native-memory/native-memory-manager.scss";
import { GlobalMemoryFullView } from "./GlobalMemoryFullView";
import { GlobalBundleViewModel, utf8Bytes, type GlobalMemoryView } from "./global-bundle-model";
import "./global-bundle.scss";

/** "size · updated" for an entry tile. */
export function entryMetaLabel(entry: Pick<Bundle, "instructions" | "updated_at">): string {
    return [formatFileSize(utf8Bytes(entry.instructions ?? "")), formatFileAge(entry.updated_at)]
        .filter(Boolean)
        .join(" · ");
}

function viewTitle(model: GlobalBundleViewModel, view: GlobalMemoryView): string {
    switch (view.kind) {
        case "new":
            return "New memory";
        case "claude-config":
            return "CLAUDE.md";
        case "preview":
            return "Combined preview";
        case "entry":
            return model.openEntryAtom()?.name ?? "";
    }
}

export const GlobalBundleManager = (): JSX.Element => {
    const model = new GlobalBundleViewModel();
    onCleanup(() => model.dispose());

    // System rows first, then ordinary — NOT model.sectionsAtom() directly,
    // which sorts by raw sort_order/name and isn't documented as
    // system-first (only the backend's ORDER BY and the composed-file
    // formatter guarantee that independently of this atom's own order —
    // SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md §3.2/§3.4). Building it
    // explicitly from the two split atoms keeps on-screen order visibly
    // matching actual injection order.
    const memoryEntries = () => [...model.systemSectionsAtom(), ...model.ordinarySectionsAtom()];

    // Leaving a full view with unsaved edits asks first — the same
    // protection Esc has (editor-keys.ts).
    const guardDirty = () => !model.draft.dirtyAtom() || window.confirm("Discard your unsaved changes?");
    const openView = (view: GlobalMemoryView) => model.open(view);
    const back = () => {
        if (guardDirty()) model.close();
    };

    // ── Drag to reorder (ordinary entries only; system entries never
    // reorder — see the model's move()). HTML5 DnD on the tiles; the full
    // view's ↑/↓ are the keyboard-reachable fallback.
    const [dragId, setDragId] = createSignal<string | null>(null);
    const [dropTargetId, setDropTargetId] = createSignal<string | null>(null);
    const dragHandlers = (entry: Bundle) =>
        entry.is_system
            ? {}
            : {
                  draggable: true,
                  onDragStart: (e: DragEvent) => {
                      setDragId(entry.id);
                      e.dataTransfer?.setData("text/plain", entry.id);
                      if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
                  },
                  onDragOver: (e: DragEvent) => {
                      const from = dragId();
                      if (!from || from === entry.id) return;
                      e.preventDefault();
                      if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
                      setDropTargetId(entry.id);
                  },
                  onDragLeave: () => {
                      if (dropTargetId() === entry.id) setDropTargetId(null);
                  },
                  onDrop: (e: DragEvent) => {
                      e.preventDefault();
                      const from = dragId() ?? e.dataTransfer?.getData("text/plain") ?? "";
                      setDragId(null);
                      setDropTargetId(null);
                      if (from && from !== entry.id) void model.moveTo(from, entry.id);
                  },
                  onDragEnd: () => {
                      setDragId(null);
                      setDropTargetId(null);
                  },
              };

    const grid = (
        <div class="native-memory-manager-file-grid-view global-bundle-grid-view">
            <p class="global-bundle-intro">
                Every agent inherits this at launch — takes effect after a restart. Drag entries to change their
                order.
            </p>

            <Show when={model.errorAtom()}>
                <div class="global-bundle-error">{model.errorAtom()}</div>
            </Show>

            <div class="native-memory-manager-file-grid global-bundle-grid">
                {/* One tile per entry (system-tier first, then ordinary in
                    injection order) — see the top-of-file comment for why
                    they're one set, not two. */}
                <For each={memoryEntries()}>
                    {(entry) => (
                        <MemoryTile
                            icon={entry.is_system ? "fa-shield-halved" : "fa-file-lines"}
                            title={entry.name}
                            monoTitle={false}
                            badges={entry.is_system ? [{ label: "system", variant: "accent" }] : []}
                            meta={entryMetaLabel(entry)}
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

                {/* Read-only reference display of the CLAUDE.md in the
                    isolated config dir a default spawned agent actually
                    launches with (CLAUDE_CONFIG_DIR). NOT part of AgentMux's
                    own Global Memory composition. See
                    docs/specs/SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md §7. */}
                <Show when={model.claudeGlobalConfigAtom()}>
                    {(cfg) => (
                        <MemoryTile
                            icon="fa-file-code"
                            title="CLAUDE.md"
                            badges={[{ label: "read-only", icon: "fa-lock", variant: "accent" }]}
                            meta={
                                cfg().exists
                                    ? `${formatFileSize(utf8Bytes(cfg().content ?? ""))} · provider config`
                                    : "No file yet · provider config"
                            }
                            onSelect={() => openView({ kind: "claude-config" })}
                            testId="global-memory-tile"
                            data={{ kind: "claude-config" }}
                            ariaLabel={`CLAUDE.md (read-only) — ${cfg().path}`}
                        />
                    )}
                </Show>

                <MemoryTile
                    icon="fa-layer-group"
                    title="Combined preview"
                    monoTitle={false}
                    meta={`${memoryEntries().length} ${memoryEntries().length === 1 ? "entry" : "entries"} · ${formatFileSize(utf8Bytes(model.previewAtom()))}`}
                    onSelect={() => openView({ kind: "preview" })}
                    classList={{ "memory-file-card--action": true }}
                    testId="global-memory-tile"
                    data={{ kind: "preview" }}
                />

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
            </div>

            <Show when={memoryEntries().length === 0}>
                <div class="global-bundle-empty">No memories yet.</div>
            </Show>
        </div>
    );

    return (
        <div class="global-bundle">
            <Show when={model.viewAtom()} fallback={grid}>
                {(view) => (
                    <div class="native-memory-manager-detail global-bundle-detail">
                        {/* The Personal Memory back/breadcrumb header
                            (native-memory-manager.tsx), one level deep here. */}
                        <div class="native-memory-manager-header">
                            <button type="button" class="native-memory-manager-back" onClick={back}>
                                ← All global memory
                            </button>
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
                )}
            </Show>
        </div>
    );
};

GlobalBundleManager.displayName = "GlobalBundleManager";
