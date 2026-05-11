// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getApi } from "@/store/global";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import { MenuBuilder } from "./menu-builder";

/**
 * Create the base tabbar context menu.
 * Includes: Version info
 */
export function createTabBarBaseMenu(): MenuBuilder {
    const menu = new MenuBuilder();
    const aboutDetails = getApi().getAboutModalDetails();
    const version = aboutDetails.version;

    menu.add({
        label: `AgentMux v${version}`,
        click: () => {
            clipboardWriteText(version);
            getApi().sendLog(`Version ${version} copied to clipboard`);
        },
    });

    return menu;
}

function getPinnedKeys(settings: Record<string, any>, widgets: Record<string, any>): string[] {
    const pinned: string[] | undefined = settings["widget:pinned"];
    if (pinned !== undefined) return pinned;
    return Object.entries(widgets)
        .filter(([, w]: any) => w["display:pinned"])
        .sort(([, a]: any, [, b]: any) => (a["display:order"] ?? 0) - (b["display:order"] ?? 0))
        .map(([key]) => key.replace("defwidget@", ""));
}

/**
 * Create the widgets menu section.
 * Shows: pinned status (checkbox), icon-only toggle.
 */
export function createWidgetsMenu(fullConfig: any): MenuBuilder {
    const menu = new MenuBuilder();
    if (!fullConfig?.widgets) return menu;

    const widgets = fullConfig.widgets || {};
    const settings = fullConfig.settings || {};
    const iconOnly = settings["widget:icononly"] ?? false;
    const pinnedKeys = new Set(getPinnedKeys(settings, widgets));

    const widgetEntries = Object.entries(widgets)
        .filter(([key]) => key.startsWith("defwidget@"))
        .sort((a: any, b: any) => (a[1]["display:order"] ?? 0) - (b[1]["display:order"] ?? 0));

    if (widgetEntries.length > 0) {
        menu.section("Pinned in bar");
        widgetEntries.forEach(([widgetKey, widgetConfig]: [string, any]) => {
            const shortName = widgetKey.replace("defwidget@", "");
            const label = widgetConfig.label || shortName;
            const isPinned = pinnedKeys.has(shortName);
            menu.add({
                label,
                type: "checkbox" as const,
                checked: isPinned,
                click: async () => {
                    const { RpcApi } = await import("@/app/store/rpc-api");
                    const { TabRpcClient } = await import("@/app/store/rpc-util");
                    const currentPinned = getPinnedKeys(settings, widgets);
                    const next = isPinned
                        ? currentPinned.filter((k) => k !== shortName)
                        : [...currentPinned, shortName];
                    await RpcApi.SetConfigCommand(TabRpcClient, { "widget:pinned": next } as any);
                },
            });
        });

        menu.separator();
    }

    menu.add({
        label: "Icon Only",
        type: "checkbox" as const,
        checked: iconOnly,
        click: async () => {
            const { RpcApi } = await import("@/app/store/rpc-api");
            const { TabRpcClient } = await import("@/app/store/rpc-util");
            await RpcApi.SetConfigCommand(TabRpcClient, { "widget:icononly": !iconOnly } as any);
        },
    });

    return menu;
}

/**
 * Theme list shown in the hamburger menu Theme submenu. Order matters
 * for muscle memory — don't reshuffle. Ids must match the schema enum
 * at schema/settings.json `window:theme`.
 */
const THEME_OPTIONS: ReadonlyArray<{ id: string; label: string }> = [
    { id: "default", label: "Default" },
    { id: "midnight", label: "Midnight" },
    { id: "high-contrast", label: "High Contrast" },
    { id: "monokai", label: "Monokai" },
    { id: "nord", label: "Nord" },
    { id: "dracula", label: "Dracula" },
    { id: "tokyo-night", label: "Tokyo Night" },
    { id: "catppuccin", label: "Catppuccin" },
    { id: "gruvbox", label: "Gruvbox" },
];

/**
 * Build the Theme submenu — radio list, one item per theme. Selection
 * writes `window:theme`; AppSettingsUpdater (frontend/app/app.tsx)
 * picks the change up via the config WebSocket and flips the
 * data-theme attribute on <html>.
 */
function createThemeMenu(settings: Record<string, any>): MenuBuilder {
    const menu = new MenuBuilder();
    const current = (settings["window:theme"] as string) || "default";
    for (const opt of THEME_OPTIONS) {
        menu.add({
            label: opt.label,
            type: "radio",
            checked: current === opt.id,
            click: async () => {
                const { RpcApi } = await import("@/app/store/rpc-api");
                const { TabRpcClient } = await import("@/app/store/rpc-util");
                await RpcApi.SetConfigCommand(TabRpcClient, {
                    "window:theme": opt.id,
                } as any);
            },
        });
    }
    return menu;
}

/**
 * Build the Opacity submenu (100% → 35%, 5% steps).
 * Selecting < 100% enables window:transparent; selecting 100% disables it.
 */
function createOpacityMenu(settings: Record<string, any>): MenuBuilder {
    const menu = new MenuBuilder();
    const rawOpacity = settings["window:opacity"] ?? 0.8;
    const isTransparent = settings["window:transparent"] ?? false;
    const effective = isTransparent ? rawOpacity : 1.0;
    const currentStep = Math.round(effective * 20) / 20; // snap to 0.05 grid

    for (let pct = 100; pct >= 35; pct -= 5) {
        const value = pct / 100;
        menu.add({
            label: `${pct}%`,
            type: "radio",
            checked: Math.abs(value - currentStep) < 0.001,
            click: async () => {
                const { RpcApi } = await import("@/app/store/rpc-api");
                const { TabRpcClient } = await import("@/app/store/rpc-util");
                if (value < 1.0) {
                    await RpcApi.SetConfigCommand(TabRpcClient, {
                        "window:opacity": value,
                        "window:transparent": true,
                    } as any);
                } else {
                    await RpcApi.SetConfigCommand(TabRpcClient, {
                        "window:opacity": 1.0,
                        "window:transparent": false,
                    } as any);
                }
            },
        });
    }
    return menu;
}

/**
 * Create the complete tabbar menu (base + widgets)
 */
export function createTabBarMenu(fullConfig: any): MenuBuilder {
    const settings = fullConfig?.settings ?? {};
    return createTabBarBaseMenu()
        .separator()
        .submenu("Opacity", createOpacityMenu(settings))
        .submenu("Theme", createThemeMenu(settings))
        .merge(createWidgetsMenu(fullConfig));
}
