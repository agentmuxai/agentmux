// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Files pane's right-click menus: a row (or the selection) and the empty space. */

import type { ContextMenuItem } from "@/app/components/context-menu";
import { shortcutFor } from "@/app/keybindings";
import { revealInFileManagerLabel } from "@/app/util/file-manager-label";
import { MENU_ICON } from "@/app/util/menu-icons";
import type { FsEntry } from "@/types/rpc/FsEntry";
import { errorText, type FilesModel } from "./files-model";
import { openInEditor, openTerminalHere, openWithOs, revealInOs } from "./files-open";
import { clipboard } from "./files-ops";
import { CONFINED_MENU } from "./files-view-text";

/** On a host, only what works there: nothing that hands its paths to
 *  this computer (an app, the file manager, an agent) or needs a Trash. */
const forHost = (model: FilesModel, items: ContextMenuItem[]): ContextMenuItem[] => {
    if (model.root()) return items.filter((i) => i.type !== "action" || CONFINED_MENU.test(i.label));
    if (!model.connection()) return items;
    const local = /^(Open with default app|Reveal in |Move .*to Trash)/;
    return items.filter((i) => i.type !== "action" || !local.test(i.label));
};


/** What a row menu calls back into the view for. */
export type RowMenuActions = {
    openEntry: (entry: FsEntry) => void;
    openInNewTab: (dir: string) => void;
    copyPaths: (list: FsEntry[], quiet?: boolean) => void;
    askDeletePermanently: (list: FsEntry[]) => void;
};

export const buildRowMenu = (model: FilesModel, entry: FsEntry, actions: RowMenuActions): ContextMenuItem[] => {
    const { openEntry, openInNewTab, copyPaths, askDeletePermanently } = actions;
    const list = model.selectedEntries();
    const many = list.length > 1;
    const path = model.pathOf(entry.name);
    const fail = (err: unknown) => model.setStatus({ text: errorText(err), tone: "error" });
    const items: ContextMenuItem[] = [{ type: "action", label: "Open", icon: entry.is_dir ? MENU_ICON.openFolder : MENU_ICON.open, shortcut: "Enter", onSelect: () => openEntry(entry) }];
    if (entry.is_dir) {
        items.push({ type: "action", label: "Open in new tab", icon: MENU_ICON.open, shortcut: shortcutFor("files:openInNewTab"), onSelect: () => openInNewTab(path) });
    }
    if (!entry.is_dir) {
        items.push(
            { type: "action", label: "Open in Editor", icon: MENU_ICON.openInEditor, onSelect: () => void openInEditor(model, path).catch(fail) },
            { type: "action", label: "Open with default app", icon: MENU_ICON.open, onSelect: () => void openWithOs(path, model.blockId).catch(fail) }
        );
    } else {
        items.push({ type: "action", label: "Open terminal here", icon: MENU_ICON.openTerminal, onSelect: () => void openTerminalHere(path, model.blockId, model.connection() || undefined).catch(fail) });
    }
    items.push(
        { type: "action", label: revealInFileManagerLabel(), icon: MENU_ICON.reveal, onSelect: () => void revealInOs(path, model.blockId).catch(fail) },
        { type: "separator" },
        { type: "action", label: "Cut", icon: MENU_ICON.cut, shortcut: shortcutFor("files:cut"), onSelect: () => model.copyToClipboard("cut", many ? list : [entry]) },
        { type: "action", label: "Copy", icon: MENU_ICON.copy, shortcut: shortcutFor("files:copy"), onSelect: () => model.copyToClipboard("copy", many ? list : [entry]) },
        { type: "action", label: many ? `Copy ${list.length} paths` : "Copy path", icon: MENU_ICON.copy, onSelect: () => copyPaths(many ? list : [entry]) },
        { type: "separator" },
        { type: "action", label: "Rename", icon: MENU_ICON.rename, shortcut: shortcutFor("files:rename"), disabled: many, onSelect: () => model.setRenaming(entry.name) },
        { type: "action", label: many ? `Move ${list.length} items to Trash` : "Move to Trash", icon: MENU_ICON.delete, shortcut: shortcutFor("files:trash"), onSelect: () => void model.trash(many ? list : [entry]) },
        {
            type: "action",
            label: "Delete permanently…", icon: MENU_ICON.deletePermanently,
            shortcut: shortcutFor("files:deletePermanently"),
            danger: true,
            onSelect: () => askDeletePermanently(many ? list : [entry]),
        }
    );
    return forHost(model, items);
};


export const buildFolderMenu = (model: FilesModel, actions: Pick<RowMenuActions, "openInNewTab">): ContextMenuItem[] => {
    const { openInNewTab } = actions;
    const fail = (err: unknown) => model.setStatus({ text: errorText(err), tone: "error" });
    return forHost(model, [
        { type: "action", label: "New folder", icon: MENU_ICON.newFolder, shortcut: shortcutFor("files:newFolder"), onSelect: () => void model.createNew("dir") },
        { type: "action", label: "New file", icon: MENU_ICON.newFile, onSelect: () => void model.createNew("file") },
        { type: "action", label: "New tab here", icon: MENU_ICON.add, shortcut: shortcutFor("files:newTabHere"), onSelect: () => openInNewTab(model.path()) },
        {
            type: "action",
            label: clipboard()?.kind === "cut" ? `Paste (move ${clipboard()!.paths.length})` : clipboard() ? `Paste (copy ${clipboard()!.paths.length})` : "Paste",
            icon: MENU_ICON.paste, shortcut: shortcutFor("files:paste"),
            disabled: !clipboard(),
            onSelect: () => void model.paste(),
        },
        { type: "separator" },
        { type: "action", label: "Open terminal here", icon: MENU_ICON.openTerminal, onSelect: () => void openTerminalHere(model.path(), model.blockId, model.connection() || undefined).catch(fail) },
        { type: "action", label: revealInFileManagerLabel(), icon: MENU_ICON.reveal, onSelect: () => void revealInOs(model.path(), model.blockId).catch(fail) },
        { type: "action", label: "Copy folder path", icon: MENU_ICON.copy, onSelect: () => void navigator.clipboard?.writeText(model.path()) },
        { type: "separator" },
        { type: "action", label: model.showHidden() ? "Hide hidden files" : "Show hidden files", icon: model.showHidden() ? MENU_ICON.hide : MENU_ICON.show, onSelect: () => model.toggleHidden() },
        { type: "action", label: "Refresh", icon: MENU_ICON.refresh, shortcut: shortcutFor("files:refresh"), onSelect: () => model.refresh() },
        { type: "action", label: "Undo", icon: MENU_ICON.undo, shortcut: shortcutFor("files:undo"), onSelect: () => void model.undo() },
    ]);
};

/**
 * The width tiers of the toolbar: `useWidthFit` sets one as the row's `data-fit` when the row is too
 * narrow for its buttons. files.scss hides a group of buttons at each, and the More menu offers them.
 */
export const TOOLBAR_TIERS = ["less", "least", "bare"] as const;

/** What the More menu needs of the view: how the list is drawn, and the toggle for it. */
export type ToolbarMenuView = { grid: boolean; toggleViewMode: () => void };

/** The toolbar's More menu: the buttons its width tier has hidden, a group to each tier. */
export const buildToolbarMenu = (model: FilesModel, tier: string | undefined, view: ToolbarMenuView): ContextMenuItem[] => {
    const hidden = TOOLBAR_TIERS.indexOf(tier as (typeof TOOLBAR_TIERS)[number]) + 1;
    const items: ContextMenuItem[] = [];
    if (hidden >= 3) {
        items.push(
            { type: "action", label: "Back", icon: MENU_ICON.back, shortcut: shortcutFor("files:back"), disabled: !model.canBack(), onSelect: () => model.goBack() },
            { type: "action", label: "Forward", icon: MENU_ICON.forward, shortcut: shortcutFor("files:forward"), disabled: !model.canForward(), onSelect: () => model.goForward() },
            { type: "separator" }
        );
    }
    if (hidden >= 2) {
        items.push(
            { type: "action", label: "New folder", icon: MENU_ICON.newFolder, shortcut: shortcutFor("files:newFolder"), onSelect: () => void model.createNew("dir") },
            { type: "action", label: "Refresh", icon: MENU_ICON.refresh, shortcut: shortcutFor("files:refresh"), onSelect: () => model.refresh() },
            { type: "separator" }
        );
    }
    if (hidden >= 1) {
        items.push(
            { type: "action", label: model.showHidden() ? "Hide hidden files" : "Show hidden files", icon: model.showHidden() ? MENU_ICON.hide : MENU_ICON.show, onSelect: () => model.toggleHidden() },
            { type: "action", label: view.grid ? "Show as a list" : "Show as thumbnails", icon: view.grid ? "list" : "table-cells-large", onSelect: view.toggleViewMode }
        );
    }
    return items;
};
