// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { getApi } from "@/store/global";
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
            navigator.clipboard.writeText(version);
            getApi().sendLog(`Version ${version} copied to clipboard`);
        },
    });

    return menu;
}

/**
 * Determine whether a widget is hidden.
 * Priority: settings["widget:hidden@<key>"] > widget["display:hidden"] > false
 */
function isWidgetHidden(settings: Record<string, any>, widgetKey: string, widgetConfig: any): boolean {
    const settingsKey = `widget:hidden@${widgetKey}`;
    if (settingsKey in settings) {
        return Boolean(settings[settingsKey]);
    }
    return widgetConfig?.["display:hidden"] ?? false;
}

/**
 * Create the widgets menu section.
 * Reads/writes widget visibility via settings.json ("widget:hidden@<key>").
 * Also includes icononly toggle.
 * "Edit widgets.json" is intentionally omitted — the menu IS the UI for this.
 */
export function createWidgetsMenu(fullConfig: any): MenuBuilder {
    const menu = new MenuBuilder();

    if (!fullConfig?.widgets) {
        return menu;
    }

    const widgets = fullConfig.widgets || {};
    const settings = fullConfig.settings || {};
    const iconOnly = settings["widget:icononly"] ?? false;

    const widgetEntries = Object.entries(widgets)
        .filter(([key]) => key.startsWith("defwidget@"))
        .sort((a: any, b: any) => {
            const orderA = a[1]["display:order"] ?? 0;
            const orderB = b[1]["display:order"] ?? 0;
            return orderA - orderB;
        });

    if (widgetEntries.length > 0) {
        menu.section("Widgets");

        widgetEntries.forEach(([widgetKey, widgetConfig]: [string, any]) => {
            const hidden = isWidgetHidden(settings, widgetKey, widgetConfig);
            const label = widgetConfig.label || widgetKey.replace("defwidget@", "");

            menu.add({
                label,
                type: "checkbox" as const,
                checked: !hidden,
                click: async () => {
                    const RpcApi = (await import("@/app/store/wshclientapi")).RpcApi;
                    const TabRpcClient = (await import("@/app/store/wshrpcutil")).TabRpcClient;
                    await RpcApi.SetConfigCommand(TabRpcClient, {
                        [`widget:hidden@${widgetKey}`]: !hidden,
                    } as any);
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
            const RpcApi = (await import("@/app/store/wshclientapi")).RpcApi;
            const TabRpcClient = (await import("@/app/store/wshrpcutil")).TabRpcClient;
            await RpcApi.SetConfigCommand(TabRpcClient, { "widget:icononly": !iconOnly } as any);
        },
    });

    return menu;
}

/**
 * Create the complete tabbar menu (base + widgets)
 */
export function createTabBarMenu(fullConfig: any): MenuBuilder {
    return createTabBarBaseMenu()
        .separator()
        .merge(createWidgetsMenu(fullConfig));
}
