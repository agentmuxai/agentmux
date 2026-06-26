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

import { createTab, getApi, openOrFocusPaneByView } from "@/store/global";
import { FlyoutMenu } from "@/app/element/flyoutmenu";
import { fireAndForget } from "@/util/util";
import { openModal } from "@/app/store/modalmodel";
import { CommandPaletteModal } from "@/app/modals/command-palette";
import { isMacOS } from "@/util/platformutil";
import { createMemo, type JSX } from "solid-js";
import "./hamburger-menu.scss";

interface HamburgerMenuProps {
    /**
     * Where the button sits in the header. "left" (default) opens the
     * flyout aligned to the left edge; "right" (macOS far-right) opens it
     * aligned to the right edge so it can't overflow off the window frame.
     */
    position?: "left" | "right";
}

export function HamburgerMenu(props: HamburgerMenuProps): JSX.Element {
    const menuItems = createMemo((): MenuItem[] => {
        const mac = isMacOS();
        const kbd = (m: string, w: string) => (mac ? m : w);

        return [
            {
                label: "New Tab",
                icon: "plus",
                shortcut: kbd("⌘T", "Ctrl+T"),
                onClick: () => createTab(),
            },
            { label: "", divider: true },
            {
                label: "New Window",
                icon: "window-restore",
                shortcut: kbd("⌘⇧N", "Ctrl+Shift+N"),
                onClick: () => getApi().openNewWindow().catch(console.error),
            },
            { label: "", divider: true },
            {
                label: "Settings",
                icon: "cog",
                onClick: () => fireAndForget(() => openOrFocusPaneByView("settings")),
            },
            {
                label: "Command Palette",
                icon: "magnifying-glass",
                shortcut: kbd("⌘P", "Ctrl+P"),
                onClick: () => openModal(CommandPaletteModal),
            },
            {
                label: "Trust Center",
                icon: "id-card",
                onClick: () => fireAndForget(() => openOrFocusPaneByView("trust")),
            },
            {
                label: "Toolchain",
                icon: "wrench",
                onClick: () => fireAndForget(() => openOrFocusPaneByView("toolchain")),
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
                onClick: () => getApi().closeWindow().catch(console.error),
            },
        ];
    });

    const atRight = (): boolean => props.position === "right";

    return (
        <FlyoutMenu
            items={menuItems()}
            placement={atRight() ? "bottom-end" : "bottom-start"}
            mirrored={atRight()}
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
