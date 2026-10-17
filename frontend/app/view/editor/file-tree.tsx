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
// Keyboard (the WAI-ARIA tree pattern: roles, one tab stop, arrows over the
// visible rows), icons and the open file's mark:
// docs/specs/SPEC_EDITOR_TREE_KEYBOARD_ICONS_AND_CLICK_ZOOM_2026_10_16.md §3–§6.
// The key logic is pure, in tree-keyboard.ts.
//
// Spec: docs/specs/SPEC_EDITOR_FILE_TREE_2026-05-26.md
// Context menu: docs/specs/SPEC_FILE_TREE_CONTEXT_MENU_2026_06_14.md

import { createEffect, createMemo, createSignal, For, on, onCleanup, Show, type Accessor, type JSX } from "solid-js";
import { fileIcon } from "./file-icons";
import { FileTreeModel, type Root } from "./file-tree-model";
import { isOpenFile, joinPath, shownEntries, treeMove, TreeTypeAhead, visibleRows } from "./tree-keyboard";

interface FileTreeProps {
    model: FileTreeModel;
    activeFilePath: string;
    showHidden: boolean;
    /** Called on file row single-click — VS Code-style "preview" open. */
    onFileClick: (path: string) => void;
    /** Called on file row double-click — pins the tab. */
    onFileDblClick?: (path: string) => void;
    onToggleHidden: () => void;
    /** Hides the file tree: the toolbar's first button. */
    onHideTree?: () => void;
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
    /** Enter on a file, or Escape: the keyboard goes to the editor body. */
    onFocusEditor?: () => void;
}

/** How long a file waits under the keyboard's selection before it previews,
 *  so holding ↓ doesn't load every file it passes (spec §5.3). */
const PREVIEW_DELAY_MS = 100;

export function FileTree(props: FileTreeProps): JSX.Element {
    let treeRef: HTMLDivElement | undefined;
    const rowEls = new Map<string, HTMLElement>();
    const registerRow = (path: string, el: HTMLElement, mounted: boolean) => {
        if (mounted) rowEls.set(path, el);
        else if (rowEls.get(path) === el) rowEls.delete(path);
    };

    const rows = createMemo(() =>
        visibleRows(
            {
                roots: props.model.rootsAtom(),
                isExpanded: (p) => props.model.isExpanded(p),
                entries: (p) => props.model.getNodeData(p)?.entries,
            },
            props.showHidden
        )
    );
    // The one row in the Tab order (roving tabindex): the selected row while
    // it's visible, else the first.
    const tabStop = createMemo(() => {
        const selected = props.model.selectedPath();
        const list = rows();
        return list.find((r) => r.path === selected)?.path ?? list[0]?.path ?? null;
    });
    const treeHasFocus = () => !!treeRef?.contains(document.activeElement);

    // A file opened from the tree: the reveal below skips it, since the user
    // is already on its row. Only when it changes the open file, or nothing
    // would clear it.
    let openedHere: string | null = null;
    let previewTimer: ReturnType<typeof setTimeout> | undefined;
    onCleanup(() => clearTimeout(previewTimer));
    const openFrom = (open: (path: string) => void, path: string) => {
        clearTimeout(previewTimer);
        if (!isOpenFile(path, props.activeFilePath)) openedHere = path;
        open(path);
    };
    const preview = (path: string) => openFrom(props.onFileClick, path);
    const keep = (path: string) => openFrom(props.onFileDblClick ?? props.onFileClick, path);

    const scrollToRow = (path: string) => {
        // The row itself, not its item: an open folder's item holds its children.
        rowEls.get(path)?.querySelector(".file-tree-row")?.scrollIntoView?.({ block: "nearest" });
    };
    /** Select a row from the keyboard, and preview it if it's a file. */
    const select = (path: string) => {
        props.model.setSelectedPath(path);
        rowEls.get(path)?.focus({ preventScroll: true });
        scrollToRow(path);
        clearTimeout(previewTimer);
        if (rows().find((r) => r.path === path)?.isDir === false) {
            previewTimer = setTimeout(() => preview(path), PREVIEW_DELAY_MS);
        }
    };

    // Reveal the open file (VS Code's explorer.autoReveal): open the folders
    // down to it, select it unless the user is in the tree, and scroll it into
    // view if it's off screen. Again once the roots arrive.
    createEffect(
        on([() => props.activeFilePath, () => props.model.rootsAtom()], ([file]) => {
            if (!file) return;
            if (openedHere != null && isOpenFile(openedHere, file)) {
                openedHere = null;
                return;
            }
            void props.model.revealPath(file).then((row) => {
                if (row == null || !isOpenFile(row, props.activeFilePath)) return;
                if (!treeHasFocus()) props.model.setSelectedPath(row);
                scrollToRow(row);
            });
        })
    );

    const typeAhead = new TreeTypeAhead();
    const pageSize = (): number => {
        const rowHeight = treeRef?.querySelector<HTMLElement>(".file-tree-row")?.offsetHeight ?? 0;
        if (!treeRef || rowHeight <= 0) return 10;
        return Math.max(1, Math.floor(treeRef.clientHeight / rowHeight) - 1);
    };

    // The WAI-ARIA tree pattern's keys (spec §5.2). Modified keys are left to
    // the app's shortcuts.
    const handleKeyDown = (e: KeyboardEvent) => {
        if (e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey) return;
        const list = rows();
        const current = tabStop();
        const row = list.find((r) => r.path === current);
        const handled = () => {
            e.preventDefault();
            e.stopPropagation();
        };
        const move = treeMove(list, current, e.key, pageSize());
        if (move) {
            handled();
            if (move.kind === "select") select(move.path);
            else if (move.kind === "expand") void props.model.expandFolder(move.path);
            else if (move.kind === "collapse") props.model.collapseFolder(move.path);
            return;
        }
        switch (e.key) {
            case "Enter":
                if (!row) return;
                handled();
                if (row.isDir) {
                    void props.model.toggleExpand(row.path);
                } else {
                    keep(row.path);
                    props.onFocusEditor?.();
                }
                return;
            case " ":
                // A space inside a name being typed belongs to it.
                if (typeAhead.active()) break;
                if (!row) return;
                handled();
                if (row.isDir) void props.model.toggleExpand(row.path);
                else keep(row.path);
                return;
            case "F2":
                // Not a root: those are HOME and the drives.
                if (!props.onStartRename || !row || row.depth === 0) return;
                handled();
                props.onStartRename(row.path);
                return;
            case "Escape":
                if (!props.onFocusEditor) return;
                handled();
                props.onFocusEditor();
                return;
        }
        if (e.key.length === 1) {
            handled();
            const hit = typeAhead.next(e.key, list, current);
            if (hit) select(hit);
        }
    };

    const handleBackgroundContextMenu = (e: MouseEvent) => {
        if (props.onContextMenu) {
            e.preventDefault();
            e.stopPropagation();
            props.onContextMenu(null, false, e);
        }
    };

    return (
        <div class="file-tree">
            <FileTreeToolbar
                model={props.model}
                showHidden={props.showHidden}
                onToggleHidden={props.onToggleHidden}
                onHideTree={props.onHideTree}
            />
            <div
                ref={treeRef}
                class="file-tree-body"
                role="tree"
                aria-label="Files"
                onKeyDown={handleKeyDown}
                // Focus and selection go together: a click or Tab that lands
                // on a row selects it.
                onFocusIn={(e) => {
                    const item = (e.target as HTMLElement).closest<HTMLElement>("[role=treeitem]");
                    if (item?.dataset.path) props.model.setSelectedPath(item.dataset.path);
                }}
                onContextMenu={handleBackgroundContextMenu}
            >
                <For each={props.model.rootsAtom()}>
                    {(root: Root, i) => (
                        <TreeNode
                            {...props}
                            onFileClick={preview}
                            onFileDblClick={keep}
                            tabStop={tabStop}
                            registerRow={registerRow}
                            path={root.path}
                            name={root.name}
                            depth={0}
                            posInSet={i() + 1}
                            setSize={props.model.rootsAtom().length}
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
    onHideTree?: () => void;
}

function FileTreeToolbar(props: FileTreeToolbarProps): JSX.Element {
    return (
        <div class="file-tree-toolbar">
            {/* The tree's own hide button; the editor's document bar has the
                one that shows it again. */}
            <Show when={props.onHideTree}>
                <button
                    type="button"
                    class="file-tree-toolbar-btn"
                    data-tip="Hide file tree"
                    aria-label="Hide file tree"
                    onClick={() => props.onHideTree?.()}
                >
                    <i class="fa fa-angles-left" />
                </button>
            </Show>
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
 * Everything except `path` / `name` / `depth` / `posInSet` / `setSize` /
 * `isDir` / `isSymlink` is per-tree state that every node forwards unchanged
 * to its children; the two `<TreeNode {...props} …/>` sites spread it rather
 * than re-listing it.
 */
interface TreeNodeProps {
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

function TreeNode(props: TreeNodeProps): JSX.Element {
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
            aria-expanded={props.isDir ? expanded() : undefined}
            aria-selected={isSelected() ? true : undefined}
        >
            <div
                class="file-tree-row"
                classList={{
                    "file-tree-row--open": isOpen(),
                    "file-tree-row--selected": isSelected(),
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
                    <i class={`${icon().icon} file-tree-icon file-tree-icon--${icon().tone}`} aria-hidden="true" />
                    <span class="file-tree-label">{props.name}</span>
                    <Show when={props.isSymlink}>
                        <span class="file-tree-symlink" aria-label="symlink">↗</span>
                    </Show>
                </Show>
            </div>
            <Show when={props.isDir && expanded()}>
                <div role="group">
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
                            <i
                                class={(() => {
                                    const { icon, tone } = fileIcon("", props.newEntry?.kind === "dir");
                                    return `${icon} file-tree-icon file-tree-icon--${tone}`;
                                })()}
                                aria-hidden="true"
                            />
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
                        {(entry, i) => (
                            <TreeNode
                                {...props}
                                path={joinPath(props.path, entry.name)}
                                name={entry.name}
                                depth={props.depth + 1}
                                posInSet={i() + 1}
                                setSize={filteredEntries().length}
                                isDir={entry.is_dir}
                                isSymlink={entry.is_symlink}
                            />
                        )}
                    </For>
                </div>
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
