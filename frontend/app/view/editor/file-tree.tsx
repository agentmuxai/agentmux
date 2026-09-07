// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Editor file-tree explorer — recursive tree + toolbar.
// State lives in FileTreeModel (file-tree-model.ts); this file is rendering.
//
// Tooltips use the `data-tip` CSS pattern (pure :hover::after, instant — no
// JS, no delay). SCSS rules in editor-view.scss scope `[data-tip]:hover::after`
// to the file-tree-toolbar so they don't leak to the rest of the editor.
//
// Spec: docs/specs/SPEC_EDITOR_FILE_TREE_2026-05-26.md
// Context menu: docs/specs/SPEC_FILE_TREE_CONTEXT_MENU_2026_06_14.md

import { createEffect, createMemo, createSignal, For, Show, type JSX } from "solid-js";
import { FileTreeModel, isHiddenName, joinPath, type Root } from "./file-tree-model";

interface FileTreeProps {
    model: FileTreeModel;
    activeFilePath: string;
    showHidden: boolean;
    /** Called on file row single-click — VS Code-style "preview" open. */
    onFileClick: (path: string) => void;
    /** Called on file row double-click — pins the tab. */
    onFileDblClick?: (path: string) => void;
    onToggleHidden: () => void;
    /** Right-click on a node or empty tree background.
     *  path=null means the background was right-clicked. */
    onContextMenu?: (path: string | null, isDir: boolean, e: MouseEvent) => void;
    /** When non-null, the node at this path renders an inline rename input. */
    renamingPath?: string | null;
    onRenameConfirm?: (path: string, newName: string) => void;
    onRenameCancel?: () => void;
    /** When set, renders an inline new-entry input inside the specified parent. */
    newEntry?: { parentPath: string; kind: "file" | "dir" } | null;
    onNewEntryConfirm?: (parentPath: string, name: string, kind: "file" | "dir") => void;
    onNewEntryCancel?: () => void;
    /** Called when F2 is pressed in the tree — trigger rename for the given path. */
    onStartRename?: (path: string) => void;
}

export function FileTree(props: FileTreeProps): JSX.Element {
    const handleBackgroundContextMenu = (e: MouseEvent) => {
        if (props.onContextMenu) {
            e.preventDefault();
            e.stopPropagation();
            props.onContextMenu(null, false, e);
        }
    };

    const handleKeyDown = (e: KeyboardEvent) => {
        if (e.key === "F2" && props.onStartRename && props.activeFilePath) {
            e.preventDefault();
            props.onStartRename(props.activeFilePath);
        }
    };

    return (
        <div class="file-tree" tabIndex={0} onKeyDown={handleKeyDown}>
            <FileTreeToolbar
                model={props.model}
                showHidden={props.showHidden}
                onToggleHidden={props.onToggleHidden}
            />
            <div
                class="file-tree-body"
                onContextMenu={handleBackgroundContextMenu}
            >
                <For each={props.model.rootsAtom()}>
                    {(root: Root) => (
                        <TreeNode
                            {...props}
                            path={root.path}
                            name={root.name}
                            depth={0}
                            isDir={true}
                            isSymlink={false}
                        />
                    )}
                </For>
            </div>
        </div>
    );
}

interface FileTreeToolbarProps {
    model: FileTreeModel;
    showHidden: boolean;
    onToggleHidden: () => void;
}

function FileTreeToolbar(props: FileTreeToolbarProps): JSX.Element {
    return (
        <div class="file-tree-toolbar">
            <button
                type="button"
                class="file-tree-toolbar-btn"
                classList={{ "file-tree-toolbar-btn--active": props.showHidden }}
                data-tip={props.showHidden ? "Hide hidden files" : "Show hidden files"}
                aria-label={props.showHidden ? "Hide hidden files" : "Show hidden files"}
                onClick={props.onToggleHidden}
            >
                <i class={`fa ${props.showHidden ? "fa-eye" : "fa-eye-slash"}`} />
            </button>
            <button
                type="button"
                class="file-tree-toolbar-btn"
                data-tip="Collapse all folders"
                aria-label="Collapse all folders"
                onClick={() => props.model.collapseAll()}
            >
                <i class="fa fa-square-minus" />
            </button>
            <button
                type="button"
                class="file-tree-toolbar-btn"
                data-tip="Refresh tree"
                aria-label="Refresh tree"
                onClick={() => void props.model.refresh()}
            >
                <i class="fa fa-arrows-rotate" />
            </button>
        </div>
    );
}

/**
 * Everything except `path` / `name` / `depth` / `isDir` / `isSymlink` is
 * per-tree state that every node forwards unchanged to its children; the
 * two `<TreeNode {...props} …/>` sites spread it rather than re-listing it.
 */
interface TreeNodeProps {
    model: FileTreeModel;
    path: string;
    name: string;
    depth: number;
    isDir: boolean;
    isSymlink: boolean;
    activeFilePath: string;
    showHidden: boolean;
    onFileClick: (path: string) => void;
    onFileDblClick?: (path: string) => void;
    onContextMenu?: (path: string | null, isDir: boolean, e: MouseEvent) => void;
    renamingPath?: string | null;
    onRenameConfirm?: (path: string, newName: string) => void;
    onRenameCancel?: () => void;
    newEntry?: { parentPath: string; kind: "file" | "dir" } | null;
    onNewEntryConfirm?: (parentPath: string, name: string, kind: "file" | "dir") => void;
    onNewEntryCancel?: () => void;
}

function TreeNode(props: TreeNodeProps): JSX.Element {
    const expanded = createMemo(() => props.model.isExpanded(props.path));
    const nodeData = createMemo(() => props.model.getNodeData(props.path));
    const isActive = createMemo(() => props.path === props.activeFilePath);
    const isRenaming = createMemo(() => props.renamingPath === props.path);

    const filteredEntries = createMemo(() => {
        const data = nodeData();
        if (!data?.entries) return [];
        if (props.showHidden) return data.entries;
        return data.entries.filter((e) => !isHiddenName(e.name));
    });

    const handleClick = (e: MouseEvent) => {
        e.stopPropagation();
        if (props.isDir) {
            void props.model.toggleExpand(props.path);
        } else {
            props.onFileClick(props.path);
        }
    };

    const handleDblClick = (e: MouseEvent) => {
        if (props.isDir) return;
        e.stopPropagation();
        if (props.onFileDblClick) {
            props.onFileDblClick(props.path);
        }
    };

    const handleContextMenu = (e: MouseEvent) => {
        if (props.onContextMenu) {
            e.preventDefault();
            e.stopPropagation();
            props.onContextMenu(props.path, props.isDir, e);
        }
    };

    return (
        <div>
            <div
                class="file-tree-row"
                classList={{
                    "file-tree-row--active": isActive(),
                    "file-tree-row--dir": props.isDir,
                    "file-tree-row--renaming": isRenaming(),
                }}
                style={{ "padding-left": `${4 + props.depth * 16}px` }}
                onClick={isRenaming() ? undefined : handleClick}
                onDblClick={handleDblClick}
                onContextMenu={handleContextMenu}
                title={isRenaming() ? undefined : props.path}
            >
                <Show
                    when={props.isDir}
                    fallback={<span class="file-tree-chevron-spacer" />}
                >
                    <i
                        class={`fa fa-chevron-${expanded() ? "down" : "right"} file-tree-chevron`}
                    />
                </Show>
                <Show
                    when={!isRenaming()}
                    fallback={
                        <InlineInput
                            initialValue={props.name}
                            onConfirm={(v) => props.onRenameConfirm?.(props.path, v)}
                            onCancel={() => props.onRenameCancel?.()}
                        />
                    }
                >
                    <i class={`fa fa-${iconForEntry(props.name, props.isDir, isActive())} file-tree-icon`} />
                    <span class="file-tree-label">{props.name}</span>
                    <Show when={props.isSymlink}>
                        <span class="file-tree-symlink" aria-label="symlink">↗</span>
                    </Show>
                </Show>
            </div>
            <Show when={props.isDir && expanded()}>
                <Show when={nodeData()?.phase === "loading"}>
                    <div
                        class="file-tree-loading"
                        style={{ "padding-left": `${4 + (props.depth + 1) * 16}px` }}
                    >
                        Loading…
                    </div>
                </Show>
                <Show when={nodeData()?.phase === "error"}>
                    <div
                        class="file-tree-error"
                        style={{ "padding-left": `${4 + (props.depth + 1) * 16}px` }}
                    >
                        ⚠ {nodeData()?.error}
                    </div>
                </Show>
                {/* New-entry placeholder — rendered inside this dir when active */}
                <Show when={props.newEntry?.parentPath === props.path}>
                    <div
                        class="file-tree-row file-tree-row--new-entry"
                        style={{ "padding-left": `${4 + (props.depth + 1) * 16}px` }}
                    >
                        <span class="file-tree-chevron-spacer" />
                        <i class={`fa fa-${props.newEntry?.kind === "dir" ? "folder" : "file"} file-tree-icon`} />
                        <InlineInput
                            initialValue=""
                            placeholder={props.newEntry?.kind === "dir" ? "folder name" : "file name"}
                            onConfirm={(v) => {
                                if (props.newEntry) {
                                    props.onNewEntryConfirm?.(props.newEntry.parentPath, v, props.newEntry.kind);
                                }
                            }}
                            onCancel={() => props.onNewEntryCancel?.()}
                        />
                    </div>
                </Show>
                <For each={filteredEntries()}>
                    {(entry) => (
                        <TreeNode
                            {...props}
                            path={joinPath(props.path, entry.name)}
                            name={entry.name}
                            depth={props.depth + 1}
                            isDir={entry.is_dir}
                            isSymlink={entry.is_symlink}
                        />
                    )}
                </For>
            </Show>
        </div>
    );
}

interface InlineInputProps {
    initialValue: string;
    placeholder?: string;
    onConfirm: (value: string) => void;
    onCancel: () => void;
}

function InlineInput(props: InlineInputProps): JSX.Element {
    const [value, setValue] = createSignal(props.initialValue);
    let inputRef: HTMLInputElement | undefined;

    createEffect(() => {
        // Auto-focus and select-all on mount.
        if (inputRef) {
            inputRef.focus();
            inputRef.select();
        }
    });

    // Guard against Escape → onCancel unmounts the input → blur fires confirm.
    let committed = false;
    const confirm = () => {
        if (committed) return;
        committed = true;
        const v = value().trim();
        if (v) props.onConfirm(v);
        else props.onCancel();
    };
    const cancel = () => {
        if (committed) return;
        committed = true;
        props.onCancel();
    };

    return (
        <input
            ref={(el) => { inputRef = el; }}
            class="file-tree-inline-input"
            type="text"
            value={value()}
            placeholder={props.placeholder}
            onInput={(e) => setValue(e.currentTarget.value)}
            onKeyDown={(e) => {
                e.stopPropagation();
                if (e.key === "Enter") confirm();
                if (e.key === "Escape") cancel();
            }}
            onBlur={confirm}
            onClick={(e) => e.stopPropagation()}
        />
    );
}

function iconForEntry(name: string, isDir: boolean, isActive: boolean): string {
    if (isDir) return "folder";
    if (isActive) return "circle-dot";
    const lower = name.toLowerCase();
    if (/\.(ts|tsx|js|jsx|mjs|cjs)$/.test(lower)) return "file-code";
    if (/\.(py|rs|go|java|c|cpp|h|hpp|sh|bash|zsh|ps1)$/.test(lower)) return "file-code";
    if (/\.(html|htm|css|scss|sass|less|svg|xml)$/.test(lower)) return "file-code";
    if (/\.(json|toml|yaml|yml|ini|env|conf)$/.test(lower)) return "file-code";
    if (/\.(md|markdown|txt|rst|adoc)$/.test(lower)) return "file-lines";
    if (/\.(png|jpe?g|gif|webp|bmp|ico)$/.test(lower)) return "file-image";
    if (/\.pdf$/.test(lower)) return "file-pdf";
    if (/\.(zip|tar|gz|7z|rar|bz2|xz)$/.test(lower)) return "file-zipper";
    return "file";
}
