// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** One node of the editor file tree: its row, and below an open folder its loading line, new-entry input and children. */

import { createMemo, For, onCleanup, Show, type Accessor, type JSX } from "solid-js";
import { fileIcon } from "./file-icons";
import type { FileTreeModel } from "./file-tree-model";
import { InlineInput } from "./file-tree-inline-input";
import { isOpenFile, joinPath, shownEntries } from "./tree-keyboard";
import { tip } from "@/app/element/tip";

/**
 * Everything except `path` / `name` / `depth` / `posInSet` / `setSize` /
 * `isDir` / `isSymlink` is per-tree state that every node forwards unchanged
 * to its children; the two `<TreeNode {...props} …/>` sites spread it rather
 * than re-listing it.
 */
export interface TreeNodeProps {
    model: FileTreeModel;
    path: string;
    name: string;
    depth: number;
    /** Its place among its siblings, from 1, and their number (aria-posinset/-setsize). */
    posInSet: number;
    setSize: number;
    isDir: boolean;
    isSymlink: boolean;
    /** The row in the Tab order. */
    tabStop: Accessor<string | null>;
    registerRow: (path: string, el: HTMLElement, mounted: boolean) => void;
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

/** What a node derives from the model and the tree's props, and what its clicks do. */
function createNodeState(props: TreeNodeProps) {
    const expanded = createMemo(() => props.model.isExpanded(props.path));
    const nodeData = createMemo(() => props.model.getNodeData(props.path));
    const isOpen = createMemo(() => !props.isDir && isOpenFile(props.path, props.activeFilePath));
    const isSelected = createMemo(() => props.model.selectedPath() === props.path);
    const isRenaming = createMemo(() => props.renamingPath === props.path);
    const icon = createMemo(() => fileIcon(props.name, props.isDir, expanded()));

    const filteredEntries = createMemo(() => shownEntries(nodeData()?.entries, props.showHidden));

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

    return { expanded, nodeData, isOpen, isSelected, isRenaming, icon, filteredEntries, handleClick, handleDblClick, handleContextMenu };
}

type NodeState = ReturnType<typeof createNodeState>;

export function TreeNode(props: TreeNodeProps): JSX.Element {
    const n = createNodeState(props);

    return (
        <div
            ref={(el) => {
                props.registerRow(props.path, el, true);
                onCleanup(() => props.registerRow(props.path, el, false));
            }}
            class="file-tree-item"
            role="treeitem"
            data-path={props.path}
            tabIndex={props.tabStop() === props.path ? 0 : -1}
            // Named by its row alone: the item also holds an open folder's children.
            aria-label={props.name}
            aria-level={props.depth + 1}
            aria-posinset={props.posInSet}
            aria-setsize={props.setSize}
            aria-expanded={props.isDir ? n.expanded() : undefined}
            aria-selected={n.isSelected() ? true : undefined}
        >
            <NodeRow p={props} n={n} />
            <Show when={props.isDir && n.expanded()}>
                <NodeChildren p={props} n={n} />
            </Show>
        </div>
    );
}

/** The node's own line: chevron, icon and name (or the rename input), and the symlink mark. */
function NodeRow(props: { p: TreeNodeProps; n: NodeState }): JSX.Element {
    const p = props.p;
    const n = props.n;
    return (
        <div
            class="file-tree-row"
            classList={{
                "file-tree-row--open": n.isOpen(),
                "file-tree-row--selected": n.isSelected(),
                "file-tree-row--dir": p.isDir,
                "file-tree-row--renaming": n.isRenaming(),
            }}
            style={{ "padding-left": `${4 + p.depth * 16}px` }}
            onClick={n.isRenaming() ? undefined : n.handleClick}
            onDblClick={n.handleDblClick}
            onContextMenu={n.handleContextMenu}
            {...tip(n.isRenaming() ? undefined : p.path)}
        >
            <Show
                when={p.isDir}
                fallback={<span class="file-tree-chevron-spacer" />}
            >
                <i
                    class={`fa fa-chevron-${n.expanded() ? "down" : "right"} file-tree-chevron`}
                />
            </Show>
            <Show
                when={!n.isRenaming()}
                fallback={
                    <InlineInput
                        initialValue={p.name}
                        onConfirm={(v) => p.onRenameConfirm?.(p.path, v)}
                        onCancel={() => p.onRenameCancel?.()}
                    />
                }
            >
                <i class={`${n.icon().icon} file-tree-icon file-tree-icon--${n.icon().tone}`} aria-hidden="true" />
                <span class="file-tree-label">{p.name}</span>
                <Show when={p.isSymlink}>
                    <span class="file-tree-symlink" aria-label="symlink">↗</span>
                </Show>
            </Show>
        </div>
    );
}

/** An open folder's contents: its loading or error line, the new-entry input if it is the parent, and its entries. */
function NodeChildren(props: { p: TreeNodeProps; n: NodeState }): JSX.Element {
    const p = props.p;
    const n = props.n;
    return (
        <div role="group">
            <Show when={n.nodeData()?.phase === "loading"}>
                <div
                    class="file-tree-loading"
                    style={{ "padding-left": `${4 + (p.depth + 1) * 16}px` }}
                >
                    Loading…
                </div>
            </Show>
            <Show when={n.nodeData()?.phase === "error"}>
                <div
                    class="file-tree-error"
                    style={{ "padding-left": `${4 + (p.depth + 1) * 16}px` }}
                >
                    ⚠ {n.nodeData()?.error}
                </div>
            </Show>
            {/* New-entry placeholder — rendered inside this dir when active */}
            <Show when={p.newEntry?.parentPath === p.path}>
                <NewEntryRow p={p} />
            </Show>
            <For each={n.filteredEntries()}>
                {(entry, i) => (
                    <TreeNode
                        {...p}
                        path={joinPath(p.path, entry.name)}
                        name={entry.name}
                        depth={p.depth + 1}
                        posInSet={i() + 1}
                        setSize={n.filteredEntries().length}
                        isDir={entry.is_dir}
                        isSymlink={entry.is_symlink}
                    />
                )}
            </For>
        </div>
    );
}

/** The inline input for a new file or folder, shown inside the folder it will be made in. */
function NewEntryRow(props: { p: TreeNodeProps }): JSX.Element {
    const p = props.p;
    return (
        <div
            class="file-tree-row file-tree-row--new-entry"
            style={{ "padding-left": `${4 + (p.depth + 1) * 16}px` }}
        >
            <span class="file-tree-chevron-spacer" />
            <i
                class={(() => {
                    const { icon, tone } = fileIcon("", p.newEntry?.kind === "dir");
                    return `${icon} file-tree-icon file-tree-icon--${tone}`;
                })()}
                aria-hidden="true"
            />
            <InlineInput
                initialValue=""
                placeholder={p.newEntry?.kind === "dir" ? "folder name" : "file name"}
                onConfirm={(v) => {
                    if (p.newEntry) {
                        p.onNewEntryConfirm?.(p.newEntry.parentPath, v, p.newEntry.kind);
                    }
                }}
                onCancel={() => p.onNewEntryCancel?.()}
            />
        </div>
    );
}
