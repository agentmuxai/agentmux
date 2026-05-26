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
// Spec: specs/SPEC_EDITOR_FILE_TREE_2026-05-26.md

import { createMemo, For, Show, type JSX } from "solid-js";
import { FileTreeModel, isHiddenName, joinPath, type Root } from "./file-tree-model";

interface FileTreeProps {
    model: FileTreeModel;
    activeFilePath: string;
    showHidden: boolean;
    onFileClick: (path: string) => void;
    onToggleHidden: () => void;
}

export function FileTree(props: FileTreeProps): JSX.Element {
    return (
        <div class="file-tree">
            <FileTreeToolbar
                model={props.model}
                showHidden={props.showHidden}
                onToggleHidden={props.onToggleHidden}
            />
            <div class="file-tree-body">
                <For each={props.model.rootsAtom()}>
                    {(root: Root) => (
                        <TreeNode
                            model={props.model}
                            path={root.path}
                            name={root.name}
                            depth={0}
                            isDir={true}
                            isSymlink={false}
                            activeFilePath={props.activeFilePath}
                            showHidden={props.showHidden}
                            onFileClick={props.onFileClick}
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
}

function TreeNode(props: TreeNodeProps): JSX.Element {
    const expanded = createMemo(() => props.model.isExpanded(props.path));
    const nodeData = createMemo(() => props.model.getNodeData(props.path));
    const isActive = createMemo(() => props.path === props.activeFilePath);

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

    return (
        <div>
            <div
                class="file-tree-row"
                classList={{
                    "file-tree-row--active": isActive(),
                    "file-tree-row--dir": props.isDir,
                }}
                style={{ "padding-left": `${4 + props.depth * 16}px` }}
                onClick={handleClick}
                title={props.path}
            >
                <Show
                    when={props.isDir}
                    fallback={<span class="file-tree-chevron-spacer" />}
                >
                    <i
                        class={`fa fa-chevron-${expanded() ? "down" : "right"} file-tree-chevron`}
                    />
                </Show>
                <i class={`fa fa-${iconForEntry(props.name, props.isDir, isActive())} file-tree-icon`} />
                <span class="file-tree-label">{props.name}</span>
                <Show when={props.isSymlink}>
                    <span class="file-tree-symlink" aria-label="symlink">↗</span>
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
                <For each={filteredEntries()}>
                    {(entry) => (
                        <TreeNode
                            model={props.model}
                            path={joinPath(props.path, entry.name)}
                            name={entry.name}
                            depth={props.depth + 1}
                            isDir={entry.is_dir}
                            isSymlink={entry.is_symlink}
                            activeFilePath={props.activeFilePath}
                            showHidden={props.showHidden}
                            onFileClick={props.onFileClick}
                        />
                    )}
                </For>
            </Show>
        </div>
    );
}

function iconForEntry(name: string, isDir: boolean, isActive: boolean): string {
    if (isDir) return "folder";
    // Active file gets a filled-dot indicator (`circle-dot`) per spec.
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
