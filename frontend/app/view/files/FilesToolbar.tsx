// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Files pane's toolbar: history, the breadcrumb (or the path editor), the filter box and the view toggles.
 * A row too narrow for its buttons moves them into a More menu, a group at a time (TOOLBAR_TIERS).
 */

import type { ContextMenuItem } from "@/app/components/context-menu";
import { Button, IconButton } from "@/app/element/ui";
import { useWidthFit } from "@/app/element/width-fit";
import { shortcutFor } from "@/app/keybindings";
import { tip } from "@/app/element/tip";
import { For, Show, type JSX } from "solid-js";
import { buildToolbarMenu, TOOLBAR_TIERS } from "./files-menus";
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
    toggleFilter: () => void;
    closeFilter: () => void;
    onToggleViewMode: () => void;
    focusList: () => void;
    pathInputRef: (el: HTMLInputElement) => void;
    crumbsRef: (el: HTMLElement) => void;
    filterRef: (el: HTMLInputElement) => void;
    filterButtonRef: (el: HTMLElement) => void;
    /** Show a menu at a point of the window: the More menu, under its button. */
    openMenu: (menu: { x: number; y: number; items: ContextMenuItem[] }) => void;
}): JSX.Element {
    const model = props.model;
    let bar: HTMLDivElement | undefined;
    // The filter box and the path editor change what the row needs without resizing it.
    useWidthFit(() => bar, TOOLBAR_TIERS, undefined, () => [props.filterOpen(), props.editingPath()]);
    const openMore = (e: MouseEvent): void => {
        const at = (e.currentTarget as HTMLElement).getBoundingClientRect();
        const items = buildToolbarMenu(model, bar?.dataset.fit, { grid: props.grid(), toggleViewMode: props.onToggleViewMode });
        props.openMenu({ x: at.left, y: at.bottom, items });
    };
    return (
        <div ref={bar} class="files-toolbar">
            <IconButton icon="arrow-left" label={`Back (${shortcutFor("files:back")})`} class="files-tool files-tool--history" disabled={!model.canBack()} onClick={() => model.goBack()} />
            <IconButton icon="arrow-right" label={`Forward (${shortcutFor("files:forward")})`} class="files-tool files-tool--history" disabled={!model.canForward()} onClick={() => model.goForward()} />
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
            <Show when={props.filterOpen()}>
                <FilterBox model={model} closeFilter={props.closeFilter} focusList={props.focusList} filterRef={props.filterRef} />
            </Show>
            <FilterButton model={model} open={props.filterOpen} toggle={props.toggleFilter} buttonRef={props.filterButtonRef} />
            <IconButton icon="folder-plus" label="New folder" class="files-tool files-tool--action" onClick={() => void model.createNew("dir")} />
            <IconButton icon={model.showHidden() ? "eye" : "eye-slash"} label={model.showHidden() ? "Hide hidden files" : "Show hidden files"} pressed={model.showHidden()} class="files-tool files-tool--view" onClick={() => model.toggleHidden()} />
            <IconButton icon="arrows-rotate" label="Refresh (F5)" class="files-tool files-tool--action" onClick={() => model.refresh()} />
            <IconButton icon="table-columns" label={model.showSidebar() ? "Hide Places" : "Show Places"} pressed={model.showSidebar()} class="files-tool files-tool--places" onClick={() => model.toggleSidebar()} />
            <IconButton icon={props.grid() ? "list" : "table-cells-large"} label={props.grid() ? "Show as a list" : "Show as thumbnails"} pressed={props.grid()} class="files-tool files-tool--view" onClick={props.onToggleViewMode} />
            <IconButton icon="eye" label={model.showPreview() ? "Hide preview (Space)" : "Show preview (Space)"} pressed={model.showPreview()} class="files-tool files-tool--preview" onClick={() => model.togglePreview()} />
            <IconButton icon="ellipsis" label="More" class="files-tool files-tool--more" aria-haspopup="menu" onClick={openMore} />
        </div>
    );
}

/** The filter box. Escape clears the text and hides it; Enter or ArrowDown hands the focus to the first match. */
function FilterBox(props: {
    model: FilesModel;
    closeFilter: () => void;
    focusList: () => void;
    filterRef: (el: HTMLInputElement) => void;
}): JSX.Element {
    const { model } = props;
    return (
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
    );
}

/**
 * The filter button: a toggle for the box. Pressed and in the accent tone while the box is shown or a
 * filter is applied; a filter with text stays applied while the box is hidden, and the button says so
 * (a dot, and its label).
 */
function FilterButton(props: {
    model: FilesModel;
    open: () => boolean;
    toggle: () => void;
    buttonRef: (el: HTMLElement) => void;
}): JSX.Element {
    const applied = () => props.model.filter() !== "" && !props.open();
    const label = () =>
        props.open()
            ? `Hide the filter (${shortcutFor("files:filter")})`
            : applied()
              ? `Filter: "${props.model.filter()}" (click to edit)`
              : `Filter (${shortcutFor("files:filter")})`;
    return (
        <IconButton
            ref={props.buttonRef}
            icon="filter"
            label={label()}
            class={`files-tool files-filter-button${applied() ? " files-filter-button--applied" : ""}`}
            pressed={props.open() || applied()}
            tone={props.open() || applied() ? "accent" : undefined}
            onClick={props.toggle}
        />
    );
}
