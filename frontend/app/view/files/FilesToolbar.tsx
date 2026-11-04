// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Files pane's toolbar: history, the breadcrumb (or the path editor), the filter box and the view toggles. */

import { Button, IconButton } from "@/app/element/ui";
import { shortcutFor } from "@/app/keybindings";
import { tip } from "@/app/element/tip";
import { For, Show, type JSX } from "solid-js";
import type { FilesModel } from "./files-model";
import { crumbsOf } from "./files-path";

export function FilesToolbar(props: {
    model: FilesModel;
    grid: () => boolean;
    editingPath: () => boolean;
    filterOpen: () => boolean;
    startEditingPath: () => void;
    commitPath: (value: string) => void;
    /** Leave the path editor without navigating. */
    onStopEditing: () => void;
    openFilter: () => void;
    closeFilter: () => void;
    onToggleViewMode: () => void;
    focusList: () => void;
    pathInputRef: (el: HTMLInputElement) => void;
    crumbsRef: (el: HTMLElement) => void;
    filterRef: (el: HTMLInputElement) => void;
}): JSX.Element {
    const model = props.model;
    return (
        <div class="files-toolbar">
            <IconButton icon="arrow-left" label={`Back (${shortcutFor("files:back")})`} class="files-tool" disabled={!model.canBack()} onClick={() => model.goBack()} />
            <IconButton icon="arrow-right" label={`Forward (${shortcutFor("files:forward")})`} class="files-tool" disabled={!model.canForward()} onClick={() => model.goForward()} />
            <IconButton icon="arrow-up" label={`Up (${shortcutFor("files:up")})`} class="files-tool" disabled={crumbsOf(model.path()).length < 2} onClick={() => model.goUp()} />
            <Show
                when={!props.editingPath()}
                fallback={
                    <input
                        ref={props.pathInputRef}
                        class="files-path-input" data-raw-text-field="the breadcrumb row's inline path editor: it replaces the crumbs in place and ends on Enter, Escape or blur, not a form field"
                        value={model.path()}
                        spellcheck={false}
                        aria-label="Folder path"
                        onKeyDown={(e) => {
                            e.stopPropagation();
                            if (e.key === "Enter") props.commitPath(e.currentTarget.value);
                            else if (e.key === "Escape") {
                                props.onStopEditing();
                                props.focusList();
                            }
                        }}
                        onBlur={() => props.onStopEditing()}
                    />
                }
            >
                <nav ref={props.crumbsRef} class="files-breadcrumb" aria-label="Folder path" onDblClick={props.startEditingPath} {...tip(`Double-click or ${shortcutFor("files:editPath")} to type a path`)}>
                    <Show when={model.connection()}>
                        <span class="files-crumb-host" {...tip(`On ${model.connection()}, over SSH`)}>
                            <i class="fa fa-server" aria-hidden="true" />
                            {model.connection()}
                        </span>
                    </Show>
                    <For each={crumbsOf(model.path())}>
                        {(crumb, i) => (
                            <>
                                <Show when={i() > 0}>
                                    <i class="fa fa-chevron-right files-crumb-sep" aria-hidden="true" />
                                </Show>
                                <Button tone="quiet" density="compact" class="files-crumb" onClick={() => void model.navigate(crumb.path)}>
                                    {crumb.label}
                                </Button>
                            </>
                        )}
                    </For>
                </nav>
            </Show>
            <Show when={props.filterOpen() || model.filter() !== ""}>
                <input
                    ref={props.filterRef}
                    class="files-filter-input" data-raw-text-field="the folder filter in the pane's toolbar row: it owns Escape, Enter and ArrowDown to hand the focus to the list, not a form field"
                    placeholder="Filter"
                    aria-label="Filter this folder"
                    value={model.filter()}
                    spellcheck={false}
                    onInput={(e) => model.setFilter(e.currentTarget.value)}
                    onKeyDown={(e) => {
                        e.stopPropagation();
                        if (e.key === "Escape") props.closeFilter();
                        else if (e.key === "Enter" || e.key === "ArrowDown") {
                            e.preventDefault();
                            const first = model.order()[0];
                            if (first) model.setSelection({ names: new Set([first]), focus: first, anchor: first });
                            props.focusList();
                        }
                    }}
                />
            </Show>
            <IconButton icon="filter" label={`Filter (${shortcutFor("files:filter")})`} class="files-tool" onClick={props.openFilter} />
            <IconButton icon="folder-plus" label="New folder" class="files-tool" onClick={() => void model.createNew("dir")} />
            <IconButton icon={model.showHidden() ? "eye" : "eye-slash"} label={model.showHidden() ? "Hide hidden files" : "Show hidden files"} pressed={model.showHidden()} class="files-tool" onClick={() => model.toggleHidden()} />
            <IconButton icon="arrows-rotate" label="Refresh (F5)" class="files-tool" onClick={() => model.refresh()} />
            <IconButton icon="table-columns" label={model.showSidebar() ? "Hide Places" : "Show Places"} pressed={model.showSidebar()} class="files-tool" onClick={() => model.toggleSidebar()} />
            <IconButton icon={props.grid() ? "list" : "table-cells-large"} label={props.grid() ? "Show as a list" : "Show as thumbnails"} pressed={props.grid()} class="files-tool" onClick={props.onToggleViewMode} />
            <IconButton icon="eye" label={model.showPreview() ? "Hide preview (Space)" : "Show preview (Space)"} pressed={model.showPreview()} class="files-tool" onClick={() => model.togglePreview()} />
        </div>
    );
}
