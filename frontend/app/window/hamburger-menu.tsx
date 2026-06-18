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

import { createTab, getApi, settingsAtom } from "@/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { THEME_OPTIONS } from "@/app/menu/base-menus";
import { FlyoutMenu } from "@/app/element/flyoutmenu";
import { invokeCommand } from "@/app/platform/ipc";
import { fireAndForget } from "@/util/util";
import { openModal } from "@/app/store/modalmodel";
import { CommandPaletteModal } from "@/app/modals/command-palette";
import { openBundleManager } from "@/app/modals/bundle-manager-modal";
import { openToolchainModal } from "@/app/modals/toolchain-modal";
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
        const settings = settingsAtom() ?? ({} as any);

        const currentTheme = (settings["window:theme"] as string) || "default";
        const themeSubItems: MenuItem[] = THEME_OPTIONS.map((opt) => ({
            label: opt.label,
            checked: currentTheme === opt.id,
            onClick: () => {
                fireAndForget(() =>
                    RpcApi.SetConfigCommand(TabRpcClient, { "window:theme": opt.id } as any),
                );
            },
        }));

        const rawOpacity = (settings["window:opacity"] as number) ?? 0.8;
        const isTransparent = (settings["window:transparent"] as boolean) ?? false;
        const effectiveOpacity = isTransparent ? rawOpacity : 1.0;
        const opacityStep = Math.round(effectiveOpacity * 20) / 20;
        const opacitySubItems: MenuItem[] = [];
        for (let pct = 100; pct >= 35; pct -= 5) {
            const value = pct / 100;
            opacitySubItems.push({
                label: `${pct}%`,
                checked: Math.abs(value - opacityStep) < 0.001,
                onClick: () => {
                    fireAndForget(() =>
                        value < 1.0
                            ? RpcApi.SetConfigCommand(TabRpcClient, {
                                  "window:opacity": value,
                                  "window:transparent": true,
                              } as any)
                            : RpcApi.SetConfigCommand(TabRpcClient, {
                                  "window:opacity": 1.0,
                                  "window:transparent": false,
                              } as any),
                    );
                },
            });
        }

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
                label: "Theme",
                icon: "palette",
                subItems: themeSubItems,
            },
            {
                label: "Opacity",
                icon: "circle-half-stroke",
                subItems: opacitySubItems,
            },
            { label: "", divider: true },
            {
                label: "Settings",
                icon: "cog",
                onClick: () =>
                    fireAndForget(async () => {
                        const path = await invokeCommand<string>("ensure_settings_file");
                        await invokeCommand("open_in_editor", { path });
                    }),
            },
            {
                label: "Command Palette",
                icon: "magnifying-glass",
                shortcut: kbd("⌘P", "Ctrl+P"),
                onClick: () => openModal(CommandPaletteModal),
            },
            {
                // Trust Center — the app-wide hub for Accounts, Identity,
                // and Memory. Opens the manager modal here when the app-wide
                // singleton is free; when it is held in another window,
                // focuses that window instead (the persistent "open
                // elsewhere" banner stays the durable affordance).
                // See SPEC_BUNDLE_MANAGEMENT §3, SPEC_TRUST_CENTER_2026_06_15.
                label: "Trust Center",
                icon: "shield-halved",
                onClick: () => openBundleManager(),
            },
            {
                // Toolchain — visibility + control over the dev tools AgentMux
                // runs CLIs in (node/npm/git/docker + provider CLIs), incl. the
                // effective PATH. See SPEC_TOOLCHAIN_MANAGER_2026-06-15.
                label: "Toolchain Manager",
                icon: "wrench",
                onClick: () => openToolchainModal(),
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
                onClick: () => fireAndForget(() => invokeCommand("close_window", {})),
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
