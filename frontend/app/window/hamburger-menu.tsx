// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The app/overflow (hamburger ☰) menu. Extracted from TabBar so it can be
// rendered in two places depending on platform:
//   * Windows / Linux — at the LEFT of the tab strip (default position),
//     clear of the right-side caption buttons.
//   * macOS — at the FAR RIGHT of the window header, after the action
//     widgets, so it doesn't crowd the native traffic-light controls on
//     the left. See window-header.tsx.
//
// Every menu action resolves to a module-level store/RPC primitive, so the
// component is fully self-contained (no TabBar state coupling).

import { atoms, createTab, getApi } from "@/store/global";
import { FlyoutMenu } from "@/app/element/flyoutmenu";
import { hostHas } from "@/app/host/host-caps";
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
import { THEME_OPTIONS } from "@/app/menu/base-menus";
import { shortcutFor } from "@/app/keybindings";
import { refreshWorkspaceList, workspaceList, workspaceName } from "@/app/store/user-workspace";
import { createEffect, createMemo, on, onCleanup, type JSX } from "solid-js";
import "./hamburger-menu.scss";

interface HamburgerMenuProps {
    /**
     * Where the button sits in the header. "left" (default) opens the
     * flyout aligned to the left edge; "right" (macOS far-right) opens it
     * aligned to the right edge so it can't overflow off the window frame.
     */
    position?: "left" | "right";
}

// Every mounted hamburger's close, so the "…" More dropdown can close it when
// it opens beside it (action-widgets.tsx): on macOS both sit at the right of
// the header, and a More opened by hover drew over an open hamburger.
const hamburgerClosers = new Set<() => void>();

/** Closes the hamburger menu if it is open. */
export function closeHamburgerMenu(): void {
    for (const close of hamburgerClosers) close();
}

/** Run one of ☰ → Workspaces' actions (workspaces.ts, loaded when used). */
function workspaceAction(run: (m: typeof import("./workspaces")) => Promise<void> | void): () => void {
    return () => fireAndForget(async () => run(await import("./workspaces")));
}

export function HamburgerMenu(props: HamburgerMenuProps): JSX.Element {
    let close: (() => void) | undefined;
    onCleanup(() => {
        if (close) hamburgerClosers.delete(close);
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
            {
                label: "DevTools",
                icon: "code",
                onClick: () => getApi().toggleDevtools(),
            },
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
            items={menuItems()}
            placement={atRight() ? "bottom-end" : "bottom-start"}
            mirrored={atRight()}
            closeRef={(fn) => {
                close = fn;
                hamburgerClosers.add(fn);
            }}
        >
            <button
                class="hamburger-btn"
                classList={{ "hamburger-btn--right": atRight() }}
                title="Menu"
                data-drag-region="false"
            >
                {/*
                 * Inline SVG with three filled rects instead of
                 * `fa fa-bars`. Icon-font glyphs rasterized one of
                 * the three lines on a fractional pixel boundary at
                 * different chrome zoom levels, making it noticeably
                 * thinner — and the affected line shifted as zoom
                 * changed. Filled rects scale uniformly with the
                 * parent's CSS zoom, no rasterization unevenness.
                 */}
                {/* height 23 (not 22) centers the bars vertically: 3px above
                    AND below (bars span y=3..20). At 22 they sat 1px low (3px
                    top / 2px bottom). 23 also gives a crisp 2px button inset
                    (was a sub-pixel 2.5px). */}
                <svg width="26" height="23" viewBox="0 0 26 23" fill="currentColor">
                    <rect x="2" y="3" width="22" height="3" rx="1" />
                    <rect x="2" y="10" width="22" height="3" rx="1" />
                    <rect x="2" y="17" width="22" height="3" rx="1" />
                </svg>
            </button>
        </FlyoutMenu>
    );
}
