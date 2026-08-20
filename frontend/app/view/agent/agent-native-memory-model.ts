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
 */

import { createMemo, createSignal, type Accessor } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

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
    // Not private: NativeMemoryHistoryPanel's revert flow (mounted as a
    // sibling view inside the same detail pane, see AgentNativeMemoryModal)
    // pushes the newly-restored content here directly rather than doing a
    // second read_file round trip of its own.
    setContent = this._content[1];

    private _editing = createSignal<boolean>(false);
    editingAtom: Accessor<boolean> = this._editing[0];
    private setEditing = this._editing[1];

    private _draft = createSignal<string>("");
    draftContentAtom: Accessor<string> = this._draft[0];
    setDraftContent = this._draft[1];

    private _loading = createSignal<boolean>(false);
    loadingAtom: Accessor<boolean> = this._loading[0];
    private setLoading = this._loading[1];

    private _saving = createSignal<boolean>(false);
    savingAtom: Accessor<boolean> = this._saving[0];
    private setSaving = this._saving[1];

    private _error = createSignal<string | null>(null);
    errorAtom: Accessor<string | null> = this._error[0];
    setError = this._error[1];

    /** The selected file's metadata row, or null. */
    selectedMetaAtom: Accessor<NativeMemoryFileMeta | null>;

    constructor(agentId: string, agentName: string) {
        this.agentId = agentId;
        this.agentName = agentName;
        this.selectedMetaAtom = createMemo(() => {
            const name = this.selectedFilenameAtom();
            if (!name) return null;
            return this.filesAtom().find((f) => f.filename === name) ?? null;
        });
        void this.loadFiles();
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
        this.setDraftContent(this.contentAtom() ?? "");
        this.setEditing(true);
        this.setError(null);
    }

    cancelEdit(): void {
        this.setEditing(false);
        this.setDraftContent("");
        this.setError(null);
    }

    /** Persist the current draft to the selected file. */
    async saveEdit(): Promise<void> {
        const filename = this.selectedFilenameAtom();
        if (!filename) return;
        this.setSaving(true);
        this.setError(null);
        try {
            const content = this.draftContentAtom();
            await RpcApi.NativeMemoryWriteFileCommand(TabRpcClient, {
                agent_id: this.agentId,
                filename,
                content,
                // reagent P1 on PR #2678: without this, the backend
                // defaults to "agent_inferred", permanently mislabeling a
                // human-authored Stash edit as "Agent" in the history UI.
                provenance: { source: "human" },
            });
            this.setContent(content);
            this.setEditing(false);
            this.setDraftContent("");
            // Refresh so size/modified_at update in the list.
            await this.loadFiles();
        } catch (e) {
            this.setError(`Save failed: ${(e as Error).message ?? e}`);
        } finally {
            this.setSaving(false);
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
        this.setSaving(true);
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
            this.setSaving(false);
        }
    }

    /** Create the MEMORY.md index with the starter template (empty-state
     *  shortcut). */
    async createMemoryIndex(): Promise<void> {
        await this.createFile("MEMORY.md", MEMORY_MD_TEMPLATE);
    }

    dispose(): void {
        // Solid signals are GC'd with the instance; nothing to unsubscribe.
    }
}
