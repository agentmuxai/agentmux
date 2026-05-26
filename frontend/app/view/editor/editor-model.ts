// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// NOTE: Editor is a pane-level view for editing files with syntax highlighting.
// It is NOT a standalone IDE — complex editing happens in the agent's terminal
// or via agent tool calls. This covers quick edits, file viewing, and diffing.
//
// File-tree explorer + header chevron toggle landed in Phase 1 of
// SPEC_EDITOR_FILE_TREE_2026-05-26.md.

import { BlockNodeModel } from "@/app/block/blocktypes";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getWaveObjectAtom, makeORef } from "@/app/store/wos";
import { createMemo, createSignal, type Accessor } from "solid-js";
import { FileTreeModel } from "./file-tree-model";

const META_TREE_EXPANDED = "editor:tree_expanded";
const META_SHOW_HIDDEN = "editor:show_hidden";

export class EditorViewModel implements ViewModel {
    viewType = "editor";
    blockId: string;
    nodeModel: BlockNodeModel;

    viewIcon: Accessor<string> = () => "file-code";
    viewName: Accessor<string>;
    viewText: Accessor<string | HeaderElem[]>;
    noPadding: Accessor<boolean> = () => true;

    get viewComponent(): ViewComponent {
        return null; // overridden by barrel via Object.defineProperty
    }

    // Editor state
    private _filePath = createSignal<string>("");
    filePathAtom: Accessor<string> = this._filePath[0];
    setFilePath = this._filePath[1];

    private _content = createSignal<string>("");
    contentAtom: Accessor<string> = this._content[0];
    setContent = this._content[1];

    private _language = createSignal<string>("");
    languageAtom: Accessor<string> = this._language[0];

    private _loading = createSignal<boolean>(false);
    loadingAtom: Accessor<boolean> = this._loading[0];

    private _dirty = createSignal<boolean>(false);
    dirtyAtom: Accessor<boolean> = this._dirty[0];
    setDirty = this._dirty[1];

    private _readOnly = createSignal<boolean>(false);
    readOnlyAtom: Accessor<boolean> = this._readOnly[0];

    private _error = createSignal<string | null>(null);
    errorAtom: Accessor<string | null> = this._error[0];

    // File-tree state (per-pane, persisted in block meta)
    private _treeExpanded = createSignal<boolean>(true);
    treeExpandedAtom: Accessor<boolean> = this._treeExpanded[0];

    private _showHidden = createSignal<boolean>(false);
    showHiddenAtom: Accessor<boolean> = this._showHidden[0];

    treeModel = new FileTreeModel();

    blockAtom: Accessor<Block | undefined>;

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;

        this.blockAtom = getWaveObjectAtom<Block>(makeORef("block", blockId));

        // Derive view name from file path
        this.viewName = createMemo(() => {
            const fp = this.filePathAtom();
            if (!fp) return "Editor";
            const name = fp.split("/").pop() || fp.split("\\").pop() || fp;
            return this.dirtyAtom() ? `${name} *` : name;
        });

        // Header items — the file-tree chevron toggle.
        this.viewText = createMemo<HeaderElem[]>(() => {
            const expanded = this.treeExpandedAtom();
            return [
                {
                    elemtype: "iconbutton",
                    icon: expanded ? "folder-tree" : "folder",
                    title: expanded ? "Hide file tree" : "Show file tree",
                    click: () => void this.toggleTreeExpanded(),
                },
            ];
        });

        // Restore persisted tree state from block meta.
        const meta = this.blockAtom()?.meta;
        if (meta?.[META_TREE_EXPANDED] === false) {
            this._treeExpanded[1](false);
        }
        if (meta?.[META_SHOW_HIDDEN] === true) {
            this._showHidden[1](true);
        }

        // Load file from block meta on init
        if (meta?.["file"]) {
            void this.openFile(meta["file"] as string);
        }
    }

    async openFile(filePath: string): Promise<void> {
        this._loading[1](true);
        this._error[1](null);
        this.setFilePath(filePath);
        this._language[1](detectLanguage(filePath));

        try {
            const result = await RpcApi.ReadEditorFileCommand(TabRpcClient, {
                path: filePath,
            });
            this.setContent(result?.content ?? "");
            this._readOnly[1](result?.read_only ?? false);
            this._dirty[1](false);

            // Store file path in block meta
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref: makeORef("block", this.blockId),
                meta: { file: filePath },
            });
        } catch (e: any) {
            this._error[1](e?.message ?? String(e));
        } finally {
            this._loading[1](false);
        }
    }

    async saveFile(): Promise<void> {
        if (this.readOnlyAtom()) return;
        const filePath = this.filePathAtom();
        if (!filePath) return;

        try {
            await RpcApi.WriteEditorFileCommand(TabRpcClient, {
                path: filePath,
                content: this.contentAtom(),
            });
            this._dirty[1](false);
            this._error[1](null);
        } catch (e: any) {
            this._error[1](`Save failed: ${e?.message ?? String(e)}`);
        }
    }

    onContentChange(content: string): void {
        this.setContent(content);
        this._dirty[1](true);
    }

    /** Toggle file-tree visibility. Persists to block meta. */
    async toggleTreeExpanded(): Promise<void> {
        const next = !this._treeExpanded[0]();
        this._treeExpanded[1](next);
        await this.persistMeta({ [META_TREE_EXPANDED]: next });
    }

    /** Toggle hidden-file visibility in the tree. Persists to block meta. */
    async toggleShowHidden(): Promise<void> {
        const next = !this._showHidden[0]();
        this._showHidden[1](next);
        await this.persistMeta({ [META_SHOW_HIDDEN]: next });
    }

    private async persistMeta(meta: Record<string, unknown>): Promise<void> {
        try {
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref: makeORef("block", this.blockId),
                meta,
            });
        } catch {
            // Persistence failure isn't fatal — the in-memory signal still
            // drives the current pane's behavior. On reopen, the previous
            // persisted value (or default) wins.
        }
    }

    giveFocus(): boolean {
        return false;
    }

    dispose(): void {}
}

// ── Language detection ──────────────────────────────────────────────────────

const EXTENSION_MAP: Record<string, string> = {
    ".ts": "typescript",
    ".tsx": "typescript",
    ".js": "javascript",
    ".jsx": "javascript",
    ".mjs": "javascript",
    ".cjs": "javascript",
    ".py": "python",
    ".rs": "rust",
    ".html": "html",
    ".htm": "html",
    ".css": "css",
    ".scss": "css",
    ".json": "json",
    ".md": "markdown",
    ".markdown": "markdown",
    ".yaml": "yaml",
    ".yml": "yaml",
    ".toml": "toml",
    ".sh": "shell",
    ".bash": "shell",
    ".zsh": "shell",
    ".ps1": "powershell",
    ".sql": "sql",
    ".go": "go",
    ".java": "java",
    ".c": "c",
    ".cpp": "cpp",
    ".h": "c",
    ".hpp": "cpp",
    ".xml": "html",
    ".svg": "html",
};

function detectLanguage(filePath: string): string {
    const lower = filePath.toLowerCase();
    for (const [ext, lang] of Object.entries(EXTENSION_MAP)) {
        if (lower.endsWith(ext)) return lang;
    }
    // Special filenames
    const name = lower.split("/").pop() || "";
    if (name === "dockerfile") return "shell";
    if (name === "makefile") return "shell";
    if (name === "cargo.toml" || name === "cargo.lock") return "toml";
    return "text";
}
