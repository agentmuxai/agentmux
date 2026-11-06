// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What the Memory pane's Global section holds (global-bundle-manager.tsx lays it out): its view model, the entries
// in the order they are delivered with what each costs, the discard question, and the drag that reorders them.

import { createSignal, onCleanup } from "solid-js";

import { createConfirm } from "@/app/element/create-confirm";
import { askDiscard } from "@/app/view/memory-editor/editor-keys";
import { beginDrag, endDrag, session, type DragEndReason } from "@/app/drag/drag-session";
import type { Bundle } from "@/app/store/rpc-api";
import { formatFileAge } from "@/app/view/native-memory/MemoryFileCard";
import { shareLabel } from "@/app/view/native-memory/memory-tokens";
import { agentKindLabel, GlobalBundleViewModel, type GlobalMemoryView } from "./global-bundle-model";

/** The entries in the order they are delivered (or largest first), what each costs, and the line under each tile. */
function createEntryList(model: GlobalBundleViewModel) {
    // System rows first, then ordinary — NOT model.sectionsAtom() directly,
    // which sorts by raw sort_order/name and isn't documented as
    // system-first (only the backend's ORDER BY and the composed-file
    // formatter guarantee that independently of this atom's own order —
    // SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md §3.2/§3.4). Building it
    // explicitly from the two split atoms keeps on-screen order visibly
    // matching actual injection order.
    const injectionOrder = () => [...model.systemSectionsAtom(), ...model.ordinarySectionsAtom()];
    // Largest first, when asked: by what the chosen kind of agent is
    // delivered (SPEC_MEMORY_PANE_TOKENS_2026_10_15.md §5.2).
    const deliveredTokens = (entry: Bundle) => model.tokensByIdAtom().get(entry.id)?.tokens_est;
    const memoryEntries = () =>
        model.largestFirstAtom()
            ? [...injectionOrder()].sort((a, b) => (deliveredTokens(b) ?? -1) - (deliveredTokens(a) ?? -1))
            : injectionOrder();
    /** "17% · 2d ago", or why the entry costs this kind of agent nothing. */
    const entryMeta = (entry: Bundle) => {
        const delivered = model.tokensByIdAtom().get(entry.id);
        const age = formatFileAge(entry.updated_at);
        if (!delivered && model.deliveredAtom()) {
            const why = (entry.instructions ?? "").trim() ? `Not sent to ${agentKindLabel(model.agentKindAtom())} agents` : "Empty";
            return [why, age].filter(Boolean).join(" · ");
        }
        const share = delivered ? shareLabel(delivered.tokens_est, model.deliveredTotalsAtom()?.total ?? 0) : "";
        return [share, age].filter(Boolean).join(" · ");
    };
    return { deliveredTokens, memoryEntries, entryMeta };
}

/** Drag to reorder the ordinary entries, and the tile it would land on. */
function createListDrag(model: GlobalBundleViewModel) {
    // ── Drag to reorder (ordinary entries only; system entries never
    // reorder — see the model's move()). HTML5 DnD on the tiles; the full
    // view's ↑/↓ are the keyboard-reachable fallback.
    // The dragged entry lives on the drag session.
    const dragId = (): string | null => {
        const s = session();
        return s?.kind === "list-item" ? (s.source?.itemId ?? null) : null;
    };
    const endListDrag = (reason: DragEndReason) => {
        if (session()?.kind === "list-item") endDrag(reason);
    };
    const [dropTargetId, setDropTargetId] = createSignal<string | null>(null);
    // No dragging while the tiles aren't in injection order.
    const dragHandlers = (entry: Bundle) =>
        entry.is_system || model.largestFirstAtom()
            ? {}
            : {
                  draggable: true,
                  onDragStart: (e: DragEvent) => {
                      beginDrag("list-item", { itemId: entry.id });
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
                      endListDrag("drop");
                      setDropTargetId(null);
                      if (from && from !== entry.id) void model.moveTo(from, entry.id);
                  },
                  onDragEnd: () => {
                      endListDrag("dragend");
                      setDropTargetId(null);
                  },
              };
    return { dragId, dropTargetId, dragHandlers };
}

export function createGlobalBundleManager() {
    const model = new GlobalBundleViewModel();
    onCleanup(() => model.dispose());
    const list = createEntryList(model);

    // Leaving a full view with unsaved edits asks first, as Esc does (editor-keys.ts).
    const confirm = createConfirm();
    const openView = (view: GlobalMemoryView) => model.open(view);
    const back = async () => {
        if (!model.draft.dirtyAtom() || (await askDiscard(confirm)("Discard your unsaved changes?"))) model.close();
    };
    const drag = createListDrag(model);
    return { model, confirm, openView, back, ...list, ...drag };
}

export type GlobalBundleManagerState = ReturnType<typeof createGlobalBundleManager>;
