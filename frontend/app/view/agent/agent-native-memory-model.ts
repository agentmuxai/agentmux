// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentNativeMemoryModel — view model for the agent pane's brain (native
 * memory) modal. Drives the file list + read/edit lifecycle over the
 * native-memory RPCs:
 *   NativeMemoryListCommand      — list *.md files
 *   NativeMemoryReadFileCommand  — read one file
 *   NativeMemoryWriteFileCommand — write/create one file
 *
 * Native memory is the `~/.claude/projects/<sanitized>/memory/` folder
 * Claude Code uses for autonomous, cross-session fact storage. This model
 * lets the user view, edit, prune, and create those files.
 *
 * Spec: SPEC_AGENT_PANE_MEMORY_IDENTITY_MODALS_2026_06_19.md §5/§8.
 *
 * The edit lifecycle is the shared MemoryDraftModel since 2026-09-24: a save
 * carries `base_sha256`, and a live `agent:memory:changed` for this agent
 * refreshes the open file in place without ever replacing an unsaved draft
 * (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.4).
 */

import { createMemo, createSignal, type Accessor } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { muxEventSubscribe } from "@/app/store/mps";
import type { NativeMemoryFileMeta } from "@/app/store/rpc-api";
import { MemoryDraftModel } from "@/app/view/memory-editor/memory-draft-model";
import { sha256Hex } from "@/util/sha256";

/** Validate a filename the same way the backend does, so the user gets
 *  feedback before the RPC round-trips. Mirrors validate_filename() in
 *  native_memory_handlers.rs: alphanumeric + `-_`, ends in `.md`, no path
 *  separators, stem ≤ 200 chars. */
export function validateMemoryFilename(filename: string): string | null {
    if (!filename) return "Filename must not be empty.";
    if (!filename.endsWith(".md")) return "Filename must end with .md.";
    if (filename.includes("/") || filename.includes("\\") || filename.includes("..")) {
        return "Filename must not contain path separators.";
    }
    const stem = filename.slice(0, -3);
    if (!stem) return "Filename needs a name before .md.";
    if (stem.length > 200) return "Filename is too long (max 200 chars).";
    if (!/^[a-zA-Z0-9_-]+$/.test(stem)) {
        return "Filename may only use letters, numbers, '-' and '_'.";
    }
    return null;
}

/** Normalize raw user input into a valid `.md` filename: trim, drop a
 *  leading path if pasted, append `.md` if omitted. */
export function normalizeMemoryFilename(raw: string): string {
    let name = raw.trim();
    if (!name) return name;
    // Strip any pasted directory portion.
    const slash = Math.max(name.lastIndexOf("/"), name.lastIndexOf("\\"));
    if (slash >= 0) name = name.slice(slash + 1);
    if (!name.toLowerCase().endsWith(".md")) name = `${name}.md`;
    return name;
}

/** Starter content for a freshly-created MEMORY.md index. */
const MEMORY_MD_TEMPLATE = `# Memory Index

This file is loaded into every new Claude session for this agent.
Keep it to an index of topic files — detail lives in the topic files,
which Claude loads on demand.
`;

export class AgentNativeMemoryModel {
    readonly agentId: string;
    readonly agentName: string;

    private _files = createSignal<NativeMemoryFileMeta[]>([]);
    filesAtom: Accessor<NativeMemoryFileMeta[]> = this._files[0];
    private setFiles = this._files[1];

    private _selected = createSignal<string | null>(null);
    selectedFilenameAtom: Accessor<string | null> = this._selected[0];
    private setSelected = this._selected[1];

    private _content = createSignal<string | null>(null);
    contentAtom: Accessor<string | null> = this._content[0];
    // Not private: kept settable for callers that already hold fresh
    // content; the history panel's revert flow goes through
    // applyExternalContent (below) so an open draft is never replaced.
    setContent = this._content[1];

    /** The open file's edit lifecycle (draft, base hash, conflict banner). */
    readonly draft: MemoryDraftModel<string>;
    editingAtom: Accessor<boolean>;
    draftContentAtom: Accessor<string>;

    private _loading = createSignal<boolean>(false);
    loadingAtom: Accessor<boolean> = this._loading[0];
    private setLoading = this._loading[1];

    /** A create in flight (the draft model owns the save-edit flag). */
    private _creating = createSignal<boolean>(false);
    private setCreating = this._creating[1];
    savingAtom: Accessor<boolean>;

    private _error = createSignal<string | null>(null);
    errorAtom: Accessor<string | null> = this._error[0];
    setError = this._error[1];

    /** The selected file's metadata row, or null. */
    selectedMetaAtom: Accessor<NativeMemoryFileMeta | null>;

    private unsubChanged: () => void;
    private changeDebounce: ReturnType<typeof setTimeout> | undefined;

    constructor(agentId: string, agentName: string) {
        this.agentId = agentId;
        this.agentName = agentName;
        this.selectedMetaAtom = createMemo(() => {
            const name = this.selectedFilenameAtom();
            if (!name) return null;
            return this.filesAtom().find((f) => f.filename === name) ?? null;
        });
        this.draft = new MemoryDraftModel<string>({
            hash: sha256Hex,
            equals: (a, b) => a === b,
            save: async (content, base) => {
                const filename = this.selectedFilenameAtom();
                if (!filename) throw new Error("No file selected.");
                await RpcApi.NativeMemoryWriteFileCommand(TabRpcClient, {
                    agent_id: this.agentId,
                    filename,
                    content,
                    // reagent P1 on PR #2678: without this, the backend
                    // defaults to "agent_inferred", permanently mislabeling a
                    // human-authored Stash edit as "Agent" in the history UI.
                    provenance: { source: "human" },
                    ...(base !== null ? { base_sha256: base } : {}),
                });
            },
            fetchCurrent: () => this.readSelected(),
        });
        this.editingAtom = this.draft.editingAtom;
        this.draftContentAtom = () => this.draft.draftAtom() ?? "";
        this.savingAtom = () => this._creating[0]() || this.draft.savingAtom();
        void this.loadFiles();
        // Live updates, debounced like the Armory's own subscription
        // (native-memory-manager.tsx): a burst of writes is one refresh.
        this.unsubChanged = muxEventSubscribe({
            eventType: `agent:memory:changed:${agentId}`,
            handler: () => {
                if (this.changeDebounce !== undefined) clearTimeout(this.changeDebounce);
                this.changeDebounce = setTimeout(() => {
                    this.changeDebounce = undefined;
                    void this.refreshFromChange();
                }, 250);
            },
        });
    }

    setDraftContent(value: string): void {
        this.draft.setDraft(value);
    }

    /** Current saved content of the selected file; `null` if it's gone. */
    private async readSelected(): Promise<string | null> {
        const filename = this.selectedFilenameAtom();
        if (!filename) return null;
        try {
            const res = await RpcApi.NativeMemoryReadFileCommand(TabRpcClient, { agent_id: this.agentId, filename });
            return res.content;
        } catch (e) {
            if (/not found/i.test((e as Error)?.message ?? "")) return null;
            throw e;
        }
    }

    /** A change event for this agent: refresh the list, and the open file in
     *  place — through the draft model, so a dirty draft is never replaced
     *  (it gets the "changed since you started editing" banner instead, and
     *  only if the file's hash actually moved off the draft's base). */
    private async refreshFromChange(): Promise<void> {
        await this.loadFiles();
        const filename = this.selectedFilenameAtom();
        if (!filename) return;
        let current: string | null;
        try {
            current = await this.readSelected();
        } catch {
            return;
        }
        if (this.selectedFilenameAtom() !== filename) return;
        this.applyExternalContent(current);
    }

    /** Show new saved content for the open file (a live change, a revert)
     *  without touching an open draft. */
    applyExternalContent(content: string | null): void {
        // Always track the saved content (the editor shows the draft, not
        // this), so cancelling a dirty draft reveals what's saved now, and
        // the next edit is based on it — not on a stale copy.
        if (content !== null) this.setContent(content);
        void this.draft.observeExternal(content);
    }

    /** Re-fetch the file list. No longer auto-selects a file on load (see
     *  SPEC_AGENT_PANE_ARMORY_HEADER_ICON_2026_07_20.md §7.2) — the modal
     *  moved to a single-pane list/detail layout (PrimitiveListDetail),
     *  where opening straight to the list is the norm for every sibling
     *  tab (MCP Servers, Skills, Startup); the old auto-select existed only
     *  to avoid an empty right pane in the previous two-column layout, and
     *  keeping it would make this tab the one inconsistent case that jumps
     *  straight into an item's detail. */
    async loadFiles(): Promise<void> {
        this.setLoading(true);
        try {
            const res = await RpcApi.NativeMemoryListCommand(TabRpcClient, {
                agent_id: this.agentId,
            });
            this.setFiles(res.files);
            this.setError(null);
        } catch (e) {
            this.setError(`Failed to list memory files: ${(e as Error).message ?? e}`);
        } finally {
            this.setLoading(false);
        }
    }

    /** Clear the current selection — returns to the list view. Refuses
     *  while an edit is in flight, mirroring `selectFile`'s guard below, so
     *  the always-visible Back button can't silently discard an unsaved
     *  draft (and can't leave `editingAtom` stuck true with `selected`
     *  cleared, which would permanently fail every future `selectFile`
     *  guard check until the modal is closed and reopened). */
    clearSelection(): void {
        if (this.editingAtom()) {
            this.setError("Finish editing (Save or Cancel) before going back.");
            return;
        }
        this.setSelected(null);
        this.setContent(null);
    }

    /** Select a file and fetch its content. Cancels any in-flight edit so
     *  switching files never silently discards an unsaved draft without
     *  the user noticing — they must Save or Cancel first. */
    async selectFile(filename: string): Promise<void> {
        if (this.editingAtom() && this.selectedFilenameAtom() !== filename) {
            this.setError("Finish editing (Save or Cancel) before switching files.");
            return;
        }
        this.setSelected(filename);
        this.setContent(null);
        this.setError(null);
        try {
            const res = await RpcApi.NativeMemoryReadFileCommand(TabRpcClient, {
                agent_id: this.agentId,
                filename,
            });
            // Guard against an out-of-order response: only apply if this is
            // still the selected file.
            if (this.selectedFilenameAtom() === filename) {
                this.setContent(res.content);
            }
        } catch (e) {
            this.setError(`Failed to read ${filename}: ${(e as Error).message ?? e}`);
        }
    }

    startEdit(): void {
        this.draft.startEdit(this.contentAtom() ?? "");
        this.setError(null);
    }

    cancelEdit(): void {
        this.draft.cancel();
        this.setError(null);
    }

    /** Persist the current draft to the selected file, against its base
     *  (a moved base is refused and surfaces as the draft's conflict). */
    async saveEdit(): Promise<void> {
        if (!this.selectedFilenameAtom()) return;
        const content = this.draftContentAtom();
        this.setError(null);
        if (await this.draft.save()) {
            this.setContent(content);
            // Refresh so size/modified_at update in the list.
            await this.loadFiles();
        }
    }

    /** Create a new file with optional starter content, then select it. */
    async createFile(rawFilename: string, content = ""): Promise<void> {
        const filename = normalizeMemoryFilename(rawFilename);
        const invalid = validateMemoryFilename(filename);
        if (invalid) {
            this.setError(invalid);
            return;
        }
        if (this.filesAtom().some((f) => f.filename === filename)) {
            this.setError(`${filename} already exists.`);
            return;
        }
        this.setCreating(true);
        this.setError(null);
        try {
            await RpcApi.NativeMemoryWriteFileCommand(TabRpcClient, {
                agent_id: this.agentId,
                filename,
                content,
                provenance: { source: "human" },
            });
            await this.loadFiles();
            await this.selectFile(filename);
        } catch (e) {
            this.setError(`Create failed: ${(e as Error).message ?? e}`);
        } finally {
            this.setCreating(false);
        }
    }

    /** Create the MEMORY.md index with the starter template (empty-state
     *  shortcut). */
    async createMemoryIndex(): Promise<void> {
        await this.createFile("MEMORY.md", MEMORY_MD_TEMPLATE);
    }

    dispose(): void {
        this.unsubChanged();
        if (this.changeDebounce !== undefined) clearTimeout(this.changeDebounce);
    }
}
