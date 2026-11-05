// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The app/overflow (AgentMux logo ☰) menu. Extracted from TabBar so it can be
// rendered in two places depending on platform:
//   * Windows / Linux — at the LEFT of the tab strip (default position),
//     clear of the right-side caption buttons.
//   * macOS — at the FAR RIGHT of the window header, after the action
//     widgets, so it doesn't crowd the native traffic-light controls on
//     the left. See window-header.tsx.
//
// Every menu action resolves to a module-level store/RPC primitive, so the
// component is fully self-contained (no TabBar state coupling).

import { atoms, getApi } from "@/store/global";
import { createTab } from "@/store/tab-actions";
import { FlyoutMenu } from "@/app/element/flyoutmenu";
import { hostHas } from "@/util/host-caps";
import { fireAndForget } from "@/util/util";
import { openModal } from "@/app/store/modalmodel";
import { CommandPaletteModal } from "@/app/modals/command-palette";
import {
    currentWindowLabel,
    currentWindowOpacity,
    currentWindowTheme,
    openNewWindowFromHere,
    setCurrentWindowTheme,
} from "@/app/store/window-appearance";
import { liveWindowOpacity, setWindowOpacityFor } from "@/app/store/window-opacity-store";
import { windowId } from "@/app/store/window-identity";
import { devModeOn } from "@/app/store/dev-mode";
import { THEME_OPTIONS } from "@/app/menu/base-menus";
import { shortcutFor } from "@/app/keybindings";
import { refreshWorkspaceList, workspaceList, workspaceName } from "@/app/store/user-workspace";
import { createEffect, createMemo, on, onCleanup, type JSX } from "solid-js";
import "./logo-menu.scss";

interface LogoMenuProps {
    /**
     * Where the button sits in the header. "left" (default) opens the
     * flyout aligned to the left edge; "right" (macOS far-right) opens it
     * aligned to the right edge so it can't overflow off the window frame.
     */
    position?: "left" | "right";
}

// Every mounted AgentMux logo's close, so the "…" More dropdown can close it when
// it opens beside it (action-widgets.tsx): on macOS both sit at the right of
// the header, and a More opened by hover drew over an open AgentMux logo.
const logoMenuClosers = new Set<() => void>();

/** Closes the AgentMux logo menu if it is open. */
export function closeLogoMenu(): void {
    for (const close of logoMenuClosers) close();
}

/** Run one of ☰ → Workspaces' actions (workspaces.ts, loaded when used). */
function workspaceAction(run: (m: typeof import("./workspaces")) => Promise<void> | void): () => void {
    return () => fireAndForget(async () => run(await import("./workspaces")));
}

export function LogoMenu(props: LogoMenuProps): JSX.Element {
    let close: (() => void) | undefined;
    onCleanup(() => {
        if (close) logoMenuClosers.delete(close);
    });
    // Open Workspace lists the workspaces: asked for again whenever the open
    // one, or its name, changes.
    createEffect(
        on(
            () => [atoms.client()?.meta?.["workspace:current"], workspaceName(atoms.client())],
            () => void refreshWorkspaceList()
        )
    );
    // Every workspace, the most recently opened first, the open one checked.
    // Before srv has answered, the open one alone.
    const openWorkspaceItems = (): MenuItem[] => {
        const list = workspaceList();
        const shown = list.length > 0 ? list : [{ id: "", name: workspaceName(atoms.client()), current: true }];
        return shown.map((w) => ({
            label: w.name,
            checked: w.current,
            onClick: workspaceAction((m) => m.switchWorkspace(w)),
        }));
    };

    const menuItems = createMemo((): MenuItem[] => {
        // Theme and Opacity stay open while the user tries values (keepOpen),
        // so a choice must not rebuild the items: that would remount the open
        // submenu under the cursor. The memo reads no reactive state but the
        // list of workspaces; each item's `checked` getter does, and
        // FlyoutMenu reads it in JSX.
        //
        // Both act on this window only (SPEC_PER_WINDOW_THEME_AND_OPACITY_
        // 2026_10_12). Settings → Appearance holds the default for windows
        // that have no value of their own.
        const themeSubItems: MenuItem[] = THEME_OPTIONS.map((opt) => ({
            label: opt.label,
            get checked() {
                return currentWindowTheme() === opt.id;
            },
            keepOpen: true,
            onClick: () => fireAndForget(() => setCurrentWindowTheme(opt.id)),
        }));

        // The status bar panel's slider sets the same value, so the check
        // follows a drag in progress too. A value between steps checks the
        // nearest one.
        const opacityStep = (): number => {
            const label = currentWindowLabel();
            const live = label ? liveWindowOpacity(label) : undefined;
            return Math.round((live ?? currentWindowOpacity()) * 20) / 20;
        };
        const opacitySubItems: MenuItem[] = [];
        for (let pct = 100; pct >= 35; pct -= 5) {
            const value = pct / 100;
            opacitySubItems.push({
                label: `${pct}%`,
                get checked() {
                    return Math.abs(value - opacityStep()) < 0.001;
                },
                keepOpen: true,
                onClick: () => {
                    const label = currentWindowLabel();
                    if (!label) return;
                    setWindowOpacityFor({ label, windowId: windowId() || null }, value, "commit");
                },
            });
        }

        return [
            {
                label: "New Tab",
                icon: "plus",
                // Shortcut labels render the binding keymodel.ts registers,
                // never a hand-written string: "Cmd" is Alt on Windows/Linux.
                shortcut: shortcutFor("tab:new"),
                onClick: () => createTab(),
            },
            { label: "", divider: true },
            ...(hostHas("multiWindow")
                ? [
                      {
                          label: "New Window",
                          icon: "window-restore",
                          shortcut: shortcutFor("window:new"),
                          onClick: () => openNewWindowFromHere().catch(console.error),
                      },
                      { label: "", divider: true },
                  ]
                : []),
            {
                label: "Theme",
                icon: "palette",
                subItems: themeSubItems,
            },
            ...(hostHas("windowTransparency")
                ? [
                      {
                          label: "Opacity",
                          icon: "circle-half-stroke",
                          subItems: opacitySubItems,
                      },
                  ]
                : []),
            {
                // SPEC_WORKSPACES_2026_10_23.md §3.2. Import… and Export…
                // replace Save layout… and Open layout….
                label: "Workspaces",
                icon: "grip",
                subItems: [
                    {
                        label: "New Workspace…",
                        icon: "plus",
                        onClick: workspaceAction((m) => m.newWorkspace()),
                    },
                    {
                        label: "Open Workspace",
                        icon: "folder-open",
                        subItems: openWorkspaceItems(),
                    },
                    {
                        label: "Import…",
                        icon: "file-import",
                        onClick: workspaceAction((m) => m.importWorkspace()),
                    },
                    {
                        label: "Export…",
                        icon: "file-export",
                        onClick: workspaceAction((m) => m.exportWorkspace()),
                    },
                    { label: "", divider: true },
                    {
                        label: "Rename workspace…",
                        icon: "pen",
                        onClick: workspaceAction((m) => m.renameWorkspacePrompt()),
                    },
                    {
                        label: "Delete workspace…",
                        icon: "trash-can",
                        onClick: workspaceAction((m) => m.deleteWorkspacePrompt()),
                    },
                ],
            },
            { label: "", divider: true },
            // Settings, Connectors, Memory and Toolchain open from the widget
            // bar, the palette and (Settings) its shortcut, not from here
            // (SPEC_WORKSPACES_2026_10_23.md §3).
            {
                label: "Command Palette",
                icon: "magnifying-glass",
                shortcut: shortcutFor("view:command-palette"),
                onClick: () => openModal(CommandPaletteModal),
            },
            // Only in dev mode. Reading it here makes this memo rebuild once
            // when the setting changes, which never happens with the menu open.
            ...(devModeOn()
                ? [
                      {
                          label: "DevTools",
                          icon: "code",
                          onClick: () => getApi().toggleDevtools(),
                      },
                  ]
                : []),
            {
                label: "Online Docs",
                icon: "book",
                onClick: () => getApi().openExternal("https://docs.agentmux.ai"),
            },
            { label: "", divider: true },
            {
                label: "Exit",
                icon: "right-from-bracket",
                // Warns first when an agent in this window is working.
                onClick: () =>
                    fireAndForget(async () => {
                        const { requestCloseWindow } = await import("./window-close");
                        await requestCloseWindow();
                    }),
            },
        ];
    });

    const atRight = (): boolean => props.position === "right";

    return (
        <FlyoutMenu
            anchorClass={atRight() ? "logo-menu-anchor logo-menu-anchor--right" : "logo-menu-anchor"}
            items={menuItems()}
            placement={atRight() ? "bottom-end" : "bottom-start"}
            mirrored={atRight()}
            closeRef={(fn) => {
                close = fn;
                logoMenuClosers.add(fn);
            }}
        >
            <button
                class="logo-menu-btn"
                title="Menu"
                data-drag-region="false"
            >
                {/*
                 * The AgentMux mark (frontend/app/asset/logo-mark.svg, the app icon's four blocks) as inline
                 * rects, not an image or an icon font: they scale evenly with chrome zoom. The mark is the
                 * icon's shape snapped to a 5 x 5 grid of 4 px cells (20 px), so every edge lands on a whole
                 * pixel and the bar, the gaps and the small blocks are all one cell. Colour comes from
                 * `--brand-mark-color` (logo-menu.scss).
                 */}
                <svg width="20" height="20" viewBox="0 0 5 5" fill="currentColor" shape-rendering="crispEdges" aria-hidden="true">
                    <rect x="0" y="0" width="5" height="1" />
                    <rect x="0" y="2" width="2" height="3" />
                    <rect x="3" y="2" width="2" height="1" />
                    <rect x="3" y="4" width="2" height="1" />
                </svg>
            </button>
        </FlyoutMenu>
    );
}
