// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Memory pane — first-class management of Memory bundles.
//
// A Memory bundle is the agent's personality + capability stack:
// provider/CLI choice, model, system instructions, context files, MCP
// servers, skills. Bundles are reusable across many agent instances —
// pick one in the launch modal alongside an Identity bundle.
//
// This module is the ViewModel for `view: "memory"` panes. It owns the
// list of memories, the currently-selected one, and the in-flight edit
// draft. CRUD goes through the v7 RPC commands
// (listmemories / upsertmemory / deletememory).

import { BlockNodeModel } from "@/app/block/blocktypes";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getWaveObjectAtom, makeORef } from "@/app/store/wos";
import { createMemo, createSignal, type Accessor } from "solid-js";

/** What the form fields look like in flight. Maps 1:1 to the Memory
 *  shape but with everything optional + JSON-array fields exposed as
 *  parsed arrays for ergonomic editing. The shape is converted back to
 *  Memory on save. */
export interface MemoryDraft {
    id?: string;
    name: string;
    description: string;
    provider: string;          // "" | "claude" | "codex" | "gemini"
    model: string;
    instructions: string;
    /** Edited as `[{ path, content }]`; serialized to JSON on save. */
    context_files: Array<{ path: string; content: string }>;
    /** Edited as raw JSON string for now (advanced). */
    mcp_servers: string;
    /** Edited as comma-separated ids for now. */
    skills: string;
}

/** Empty draft for the "+ New Memory" flow.
 *
 *  All JSON-array fields default to `"[]"` (not `""`). The backend's
 *  `db_memories.skills` column is JSON-encoded; a literal `""` would
 *  trip downstream `JSON.parse(skills)` readers. Reagent P1 on
 *  PR #747 (2026-05-08). */
export function emptyDraft(): MemoryDraft {
    return {
        id: undefined,
        name: "",
        description: "",
        provider: "",
        model: "",
        instructions: "",
        context_files: [],
        mcp_servers: "[]",
        skills: "[]",
    };
}

/** Hydrate a draft from a stored Memory. JSON fields are parsed; on
 *  parse failure we fall back to safe empties so the UI stays usable
 *  even if the row is malformed. */
export function draftFromMemory(m: Memory): MemoryDraft {
    let context_files: Array<{ path: string; content: string }> = [];
    try {
        const parsed = JSON.parse(m.context_files ?? "[]");
        if (Array.isArray(parsed)) context_files = parsed;
    } catch {
        // Fall back to empty list; user can re-add files.
    }
    return {
        id: m.id,
        name: m.name,
        description: m.description ?? "",
        provider: m.provider ?? "",
        model: m.model ?? "",
        instructions: m.instructions ?? "",
        context_files,
        // Both JSON-array fields use the same empty-string-aware
        // fallback. A legacy row with mcp_servers = "" would
        // otherwise load empty into the textarea, looking
        // unconfigured. Reagent P2 (PR #749).
        mcp_servers:
            m.mcp_servers && m.mcp_servers.trim().length > 0 ? m.mcp_servers : "[]",
        skills: m.skills && m.skills.trim().length > 0 ? m.skills : "[]",
    };
}

/** Serialize a draft into the wire shape for `upsertmemory`.
 *
 *  The backend deserializes directly into the Rust `Memory` struct,
 *  which has no serde defaults for `created_at` / `updated_at`. Send
 *  0 for both — the upsert handler server-sets `created_at = now`
 *  when it sees 0 and always overwrites `updated_at` with now. Codex
 *  P1 (PR #749). */
export function draftToWire(d: MemoryDraft): Memory {
    return {
        id: d.id ?? "",
        name: d.name.trim(),
        description: d.description.trim(),
        provider: d.provider,
        model: d.model.trim(),
        instructions: d.instructions,
        context_files: JSON.stringify(d.context_files),
        mcp_servers: d.mcp_servers || "[]",
        // Same JSON-array invariant as mcp_servers — never write an
        // empty string to the skills column. Reagent P1 (PR #747).
        skills: d.skills || "[]",
        created_at: 0,
        updated_at: 0,
    };
}

export class MemoryViewModel implements ViewModel {
    viewType = "memory";
    blockId: string;
    nodeModel: BlockNodeModel;

    viewIcon: Accessor<string> = () => "brain";
    viewName: Accessor<string>;
    viewText: Accessor<string | HeaderElem[]> = () => "Memory";
    noPadding: Accessor<boolean> = () => false;

    get viewComponent(): ViewComponent {
        return null; // overridden by the barrel via Object.defineProperty
    }

    blockAtom: Accessor<Block | undefined>;

    private _memories = createSignal<Memory[]>([]);
    memoriesAtom: Accessor<Memory[]> = this._memories[0];
    setMemories = this._memories[1];

    private _selectedId = createSignal<string | null>(null);
    selectedIdAtom: Accessor<string | null> = this._selectedId[0];
    setSelectedId = this._selectedId[1];

    private _draft = createSignal<MemoryDraft | null>(null);
    draftAtom: Accessor<MemoryDraft | null> = this._draft[0];
    setDraft = this._draft[1];

    private _saving = createSignal<boolean>(false);
    savingAtom: Accessor<boolean> = this._saving[0];
    setSaving = this._saving[1];

    private _error = createSignal<string | null>(null);
    errorAtom: Accessor<string | null> = this._error[0];
    setError = this._error[1];

    /** Memo: the currently-selected Memory row, or null. */
    selectedAtom: Accessor<Memory | null>;

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
        this.blockAtom = getWaveObjectAtom(makeORef("block", blockId));
        this.viewName = createMemo(() => {
            const block = this.blockAtom();
            return (block?.meta?.["frame:title"] as string) ?? "Memory";
        });
        this.selectedAtom = createMemo(() => {
            const id = this.selectedIdAtom();
            if (!id) return null;
            return this.memoriesAtom().find((m) => m.id === id) ?? null;
        });

        // Kick off initial load. Errors land in errorAtom for UI surfacing.
        void this.refresh();
    }

    /** Re-fetch the full list. Called on mount and after each mutation. */
    async refresh(): Promise<void> {
        try {
            const list = await RpcApi.ListMemoriesCommand(TabRpcClient, {});
            this.setMemories(list);
            this.setError(null);
        } catch (e) {
            this.setError(`Failed to load memories: ${(e as Error).message ?? e}`);
        }
    }

    /** Open the form for a new memory. Clears any stale error so the
     *  user starts on a clean banner. */
    startNew(): void {
        this.setError(null);
        this.setDraft(emptyDraft());
        this.setSelectedId(null);
    }

    /** Open the form for editing an existing memory. Refuses on the blank
     *  singleton because the backend rejects mutations to it anyway —
     *  better to show a disabled UI than to surface a backend error after
     *  the user types. Successful entry clears any stale error from a
     *  previous failed action (e.g. clicking the blank singleton, then
     *  clicking a real memory should not leave the "system-managed"
     *  banner showing alongside the new edit form). Reagent P2 (#747). */
    startEdit(memory: Memory): void {
        if (memory.is_blank) {
            this.setError("The blank Memory is system-managed and cannot be edited.");
            return;
        }
        this.setError(null);
        this.setDraft(draftFromMemory(memory));
        this.setSelectedId(memory.id);
    }

    /** Discard the current draft AND clear any stale error. The save
     *  path leaves an error banner up if it failed; cancelling the
     *  form should also clear it so the read-only view comes back
     *  clean. Reagent P2 (#747). */
    cancelDraft(): void {
        this.setDraft(null);
        this.setError(null);
    }

    /** Persist the current draft (creates if id is empty, else updates). */
    async saveDraft(): Promise<void> {
        const draft = this.draftAtom();
        if (!draft) return;
        if (!draft.name.trim()) {
            this.setError("Memory name is required.");
            return;
        }
        this.setSaving(true);
        this.setError(null);
        try {
            const saved = await RpcApi.UpsertMemoryCommand(TabRpcClient, draftToWire(draft));
            this.setDraft(null);
            // Refresh BEFORE setSelectedId so the list contains the
            // saved row when selectedAtom resolves it. Otherwise on
            // create, selectedAtom returns null for one tick and the
            // empty-state ("Select a Memory bundle…") flashes briefly.
            // Reagent P2 (PR #749).
            await this.refresh();
            this.setSelectedId(saved.id);
        } catch (e) {
            this.setError(`Save failed: ${(e as Error).message ?? e}`);
        } finally {
            this.setSaving(false);
        }
    }

    async deleteMemory(id: string): Promise<void> {
        const target = this.memoriesAtom().find((m) => m.id === id);
        if (target?.is_blank) {
            this.setError("The blank Memory is system-managed and cannot be deleted.");
            return;
        }
        this.setError(null);
        try {
            await RpcApi.DeleteMemoryCommand(TabRpcClient, { id });
            if (this.selectedIdAtom() === id) this.setSelectedId(null);
            this.setDraft(null);
            await this.refresh();
        } catch (e) {
            this.setError(`Delete failed: ${(e as Error).message ?? e}`);
        }
    }

    /** ViewModel teardown — Solid signals are GC'd with the instance. */
    dispose(): void {
        // No-op for now. If we add a wave-event subscription later (to
        // refresh on `memories:changed` from other clients), unsubscribe
        // here.
    }
}
