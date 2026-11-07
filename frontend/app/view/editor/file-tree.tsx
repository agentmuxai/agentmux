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

import { For, Show, type JSX } from "solid-js";
import { IconButton } from "@/app/element/ui";
import { FileTreeModel, type Root } from "./file-tree-model";
import { createFileTreeState, type FileTreeProps } from "./file-tree-state";
import { TreeNode } from "./file-tree-node";

export function FileTree(props: FileTreeProps): JSX.Element {
    const s = createFileTreeState(props);

    return (
        <div class="file-tree">
            <FileTreeToolbar
                model={props.model}
                showHidden={props.showHidden}
                onToggleHidden={props.onToggleHidden}
                onHideTree={props.onHideTree}
            />
            <div
                ref={s.setTreeRef}
                class="file-tree-body"
                role="tree"
                aria-label="Files"
                onKeyDown={s.handleKeyDown}
                // Focus and selection go together: a click or Tab that lands
                // on a row selects it.
                onFocusIn={(e) => {
                    const item = (e.target as HTMLElement).closest<HTMLElement>("[role=treeitem]");
                    if (item?.dataset.path) props.model.setSelectedPath(item.dataset.path);
                }}
                onContextMenu={s.handleBackgroundContextMenu}
            >
                <For each={props.model.rootsAtom()}>
                    {(root: Root, i) => (
                        <TreeNode
                            {...props}
                            onFileClick={s.preview}
                            onFileDblClick={s.keep}
                            tabStop={s.tabStop}
                            registerRow={s.registerRow}
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
                <IconButton
                    icon="angles-left"
                    label="Hide file tree"
                    density="compact"
                    onClick={() => props.onHideTree?.()}
                />
            </Show>
            <IconButton
                icon={props.showHidden ? "eye" : "eye-slash"}
                label={props.showHidden ? "Hide hidden files" : "Show hidden files"}
                density="compact"
                pressed={props.showHidden}
                onClick={props.onToggleHidden}
            />
            <IconButton
                icon="square-minus"
                label="Collapse all folders"
                density="compact"
                onClick={() => props.model.collapseAll()}
            />
            <IconButton
                icon="arrows-rotate"
                label="Refresh tree"
                density="compact"
                onClick={() => void props.model.refresh()}
            />
        </div>
    );
}
