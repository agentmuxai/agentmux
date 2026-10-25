// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The icon for each right-click menu action, in one place, so the same action is the same icon in
// every menu (SPEC_CONTEXT_MENU_ICONS_2026_10_25.md §3 and §4). Names are Font Awesome names as
// `makeIconClass` reads them ("copy", "regular@window-maximize"); every one resolves in the
// vendored stylesheets under public/fontawesome/css (menu-icons.test.ts checks).

import { makeIconClass } from "@/util/util";

export const MENU_ICON = {
    // Clipboard and text
    copy: "copy",
    copyLink: "link",
    paste: "paste",
    cut: "scissors",
    undo: "rotate-left",
    // Open and reveal
    open: "arrow-up-right-from-square",
    openFolder: "folder-open",
    openInEditor: "pen-to-square",
    openTerminal: "terminal",
    reveal: "folder-open",
    // Create
    add: "plus",
    newFile: "file-circle-plus",
    newFolder: "folder-plus",
    // Change
    rename: "pen",
    delete: "trash",
    deletePermanently: "trash-can",
    refresh: "arrows-rotate",
    close: "xmark",
    settings: "gear",
    // Visibility and pinning
    pin: "thumbtack",
    unpin: "thumbtack-slash",
    hide: "eye-slash",
    show: "eye",
    // Panes
    magnify: "regular@window-maximize",
    unmagnify: "regular@window-restore",
    splitDown: "table-rows",
    splitRight: "table-columns",
    floatingPane: "window-restore",
    inspect: "bug",
    viewSource: "code",
    paneColor: "palette",
    paneColorAll: "clone",
    // Browser
    back: "arrow-left",
    forward: "arrow-right",
    reload: "arrows-rotate",
    print: "print",
    home: "house",
    // Agents and connections
    history: "clock-rotate-left",
    fork: "code-branch",
    mention: "at",
    attach: "paperclip",
    bind: "link",
    sessions: "list",
    connect: "plug",
    disconnect: "plug-circle-xmark",
    endSession: "circle-stop",
    remote: "network-wired",
    plotType: "chart-line",
    seat: "chair",
    bundles: "layer-group",
    memory: "brain",
} as const;

export type MenuIconName = (typeof MENU_ICON)[keyof typeof MENU_ICON];

/**
 * The class list of a menu row's leading icon: a fixed-width Font Awesome glyph in the shared
 * `.menu-item-icon` slot (flyoutmenu.scss). Null for a blank or unrecognised name, so a typo draws
 * the empty slot rather than a broken glyph.
 */
export function menuIconClass(icon: string | null | undefined): string | null {
    if (!icon) return null;
    const cls = makeIconClass(icon, true);
    return cls ? `${cls} menu-item-icon` : null;
}
