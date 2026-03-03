// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { getApi } from "@/store/global";
import { MenuBuilder } from "./menu-builder";

/**
 * Create the base tabbar context menu
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
 * Create the widgets menu section
 * Includes: Widget toggles and config editor
 */
export function createWidgetsMenu(fullConfig: any): MenuBuilder {
    const menu = new MenuBuilder();

    if (!fullConfig?.widgets) {
        return menu;
    }

    const widgets = fullConfig.widgets || {};
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
            const isHidden = widgetConfig["display:hidden"] ?? false;
            const label = widgetConfig.label || widgetKey.replace("defwidget@", "");

            menu.add({
                label: label,
                type: "checkbox" as const,
                checked: !isHidden,
                click: async () => {
                    const RpcApi = (await import("@/app/store/wshclientapi")).RpcApi;
                    const TabRpcClient = (await import("@/app/store/wshrpcutil")).TabRpcClient;

                    const newHiddenState = !isHidden;
                    const updatedConfig = {
                        ...widgetConfig,
                        "display:hidden": newHiddenState,
                    };

                    await RpcApi.SetConfigCommand(TabRpcClient, {
                        [widgetKey]: updatedConfig,
                    });

                    getApi().sendLog(`Widget ${label} ${newHiddenState ? "hidden" : "shown"}`);
                },
            });
        });

        menu.separator().add({
            label: "Edit widgets.json",
            click: async () => {
                const createBlock = (await import("@/store/global")).createBlock;
                const path = `${getApi().getConfigDir()}/widgets.json`;
                const blockDef: BlockDef = {
                    meta: { view: "preview", file: path },
                };
                await createBlock(blockDef, false, true);
            },
        });
    }

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
