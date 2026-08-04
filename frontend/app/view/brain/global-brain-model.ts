// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// GlobalBrainViewModel — drives the Armory "Memory" tab (labeled "Brain"
// prior to the PROPOSAL_COMPOSABLE_AGENT_MODEL_2026_06_30.md §4.2 naming
// decision): the workspace-wide global brain that every agent inherits at
// launch.
//
// A "section" is a Memory bundle with is_global=true. The global brain is
// the ordered list of those sections; their instructions concatenate into
// each agent's CLAUDE.md at launch (backend: format_global_brain_block).
// Section order is the sort_order column, mutated via reorderglobalbrain.
//
// This model is block-free (same shape as MemoryViewModel) and drives off
// the bundle_* RPCs. Mutations refresh the list afterwards; it does not
// subscribe to memories:changed (matching MemoryViewModel — the manager is
// the only writer in practice).
//
// Spec: specs/archive/SPEC_TRUST_CENTER_GLOBAL_BRAIN_2026_06_19.md.

import { createMemo, createSignal, type Accessor } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

/** Sentinel editingId for the unsaved "new section" draft. */
export const NEW_SECTION_ID = "__new__";

/** Mirror of the backend format_global_brain_block — keep in sync so the
 *  preview matches exactly what lands in CLAUDE.md. */
function formatGlobalBrainBlock(sections: Memory[]): string {
    return sections
        .filter((s) => (s.instructions ?? "").trim().length > 0)
        .map((s) => `# [Workspace] ${s.name}\n\n${s.instructions}`)
        .join("\n\n---\n\n");
}

export class GlobalBrainViewModel {
    private _all = createSignal<Memory[]>([]);
    /** Every bundle (global + per-agent), used to derive sections + candidates. */
    allAtom: Accessor<Memory[]> = this._all[0];
    private setAll = this._all[1];

    private _editingId = createSignal<string | null>(null);
    editingIdAtom: Accessor<string | null> = this._editingId[0];
    private setEditingId = this._editingId[1];

    private _draftName = createSignal<string>("");
    draftNameAtom: Accessor<string> = this._draftName[0];
    setDraftName = this._draftName[1];

    private _draftInstructions = createSignal<string>("");
    draftInstructionsAtom: Accessor<string> = this._draftInstructions[0];
    setDraftInstructions = this._draftInstructions[1];

    private _saving = createSignal<boolean>(false);
    savingAtom: Accessor<boolean> = this._saving[0];
    private setSaving = this._saving[1];

    private _error = createSignal<string | null>(null);
    errorAtom: Accessor<string | null> = this._error[0];
    setError = this._error[1];

    private _showPreview = createSignal<boolean>(false);
    showPreviewAtom: Accessor<boolean> = this._showPreview[0];
    setShowPreview = this._showPreview[1];

    /** Global sections, in injection order (sort_order, then name). */
    sectionsAtom: Accessor<Memory[]>;
    /** Non-global, non-blank bundles eligible to promote into the brain. */
    candidatesAtom: Accessor<Memory[]>;
    /** Combined CLAUDE.md preview block. */
    previewAtom: Accessor<string>;

    constructor() {
        this.sectionsAtom = createMemo(() =>
            this.allAtom()
                .filter((m) => m.is_global && !m.is_blank)
                .sort(
                    (a, b) =>
                        (a.sort_order ?? 0) - (b.sort_order ?? 0) ||
                        a.name.localeCompare(b.name),
                ),
        );
        this.candidatesAtom = createMemo(() =>
            this.allAtom()
                .filter((m) => !m.is_global && !m.is_blank)
                .sort((a, b) => a.name.localeCompare(b.name)),
        );
        this.previewAtom = createMemo(() => formatGlobalBrainBlock(this.sectionsAtom()));
        void this.refresh();
    }

    async refresh(): Promise<void> {
        try {
            const list = await RpcApi.ListMemoriesCommand(TabRpcClient, {});
            this.setAll(list);
            this.setError(null);
        } catch (e) {
            this.setError(`Failed to load global brain: ${(e as Error).message ?? e}`);
        }
    }

    /** Open the inline editor for an existing section. */
    startEdit(section: Memory): void {
        this.setError(null);
        this.setEditingId(section.id);
        this.setDraftName(section.name);
        this.setDraftInstructions(section.instructions ?? "");
    }

    /** Open a blank "new section" editor at the end of the list — where
     *  saveEdit appends the saved section, so the draft sits where it lands. */
    startNew(): void {
        this.setError(null);
        this.setEditingId(NEW_SECTION_ID);
        this.setDraftName("");
        this.setDraftInstructions("");
    }

    cancelEdit(): void {
        this.setEditingId(null);
        this.setDraftName("");
        this.setDraftInstructions("");
        this.setError(null);
    }

    /** Persist the current draft. Creates a new global section (appended to
     *  the end of the order) or updates the section being edited. */
    async saveEdit(): Promise<void> {
        const name = this.draftNameAtom().trim();
        if (!name) {
            this.setError("Section name is required.");
            return;
        }
        const editingId = this.editingIdAtom();
        if (editingId === null) return;
        this.setSaving(true);
        this.setError(null);
        try {
            const instructions = this.draftInstructionsAtom();
            if (editingId === NEW_SECTION_ID) {
                const saved = await RpcApi.UpsertMemoryCommand(TabRpcClient, {
                    id: "",
                    name,
                    is_global: true,
                    instructions,
                });
                // Append the new section to the end of the order.
                const order = [...this.sectionsAtom().map((s) => s.id), saved.id];
                await RpcApi.ReorderGlobalBrainCommand(TabRpcClient, { ids: order });
            } else {
                const existing = this.allAtom().find((m) => m.id === editingId);
                if (!existing) {
                    this.setError("Section no longer exists.");
                    return;
                }
                await RpcApi.UpsertMemoryCommand(TabRpcClient, {
                    ...existing,
                    name,
                    instructions,
                    is_global: true,
                });
            }
            await this.refresh();
            this.cancelEdit();
        } catch (e) {
            this.setError(`Save failed: ${(e as Error).message ?? e}`);
        } finally {
            this.setSaving(false);
        }
    }

    /** Promote an existing non-global bundle into the brain, appended last. */
    async promote(id: string): Promise<void> {
        const bundle = this.allAtom().find((m) => m.id === id);
        if (!bundle) return;
        this.setError(null);
        try {
            await RpcApi.UpsertMemoryCommand(TabRpcClient, { ...bundle, is_global: true });
            const order = [...this.sectionsAtom().map((s) => s.id), id];
            await RpcApi.ReorderGlobalBrainCommand(TabRpcClient, { ids: order });
            await this.refresh();
        } catch (e) {
            this.setError(`Promote failed: ${(e as Error).message ?? e}`);
        }
    }

    /** Remove a section from the brain (clears is_global). The bundle itself
     *  is kept — it stays available in the Memories tab. */
    async remove(id: string): Promise<void> {
        const bundle = this.allAtom().find((m) => m.id === id);
        if (!bundle) return;
        this.setError(null);
        try {
            await RpcApi.UpsertMemoryCommand(TabRpcClient, { ...bundle, is_global: false });
            if (this.editingIdAtom() === id) this.cancelEdit();
            await this.refresh();
        } catch (e) {
            this.setError(`Remove failed: ${(e as Error).message ?? e}`);
        }
    }

    /** Move a section one slot earlier/later and persist the new order. */
    async move(id: string, dir: -1 | 1): Promise<void> {
        const ids = this.sectionsAtom().map((s) => s.id);
        const i = ids.indexOf(id);
        const j = i + dir;
        if (i === -1 || j < 0 || j >= ids.length) return;
        [ids[i], ids[j]] = [ids[j], ids[i]];
        this.setError(null);
        try {
            await RpcApi.ReorderGlobalBrainCommand(TabRpcClient, { ids });
            await this.refresh();
        } catch (e) {
            this.setError(`Reorder failed: ${(e as Error).message ?? e}`);
        }
    }

    dispose(): void {
        // Solid signals are GC'd with the instance; nothing to unsubscribe.
    }
}
