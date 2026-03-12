// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * ActionWidgets - Right-aligned buttons for creating blocks
 */

import { Tooltip } from "@/app/element/tooltip";
import { ContextMenuModel } from "@/app/store/contextmenu";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { atoms, createBlock, getApi } from "@/store/global";
import { fireAndForget, isBlank, makeIconClass } from "@/util/util";
import { invoke } from "@tauri-apps/api/core";
import { For, Show, type JSX } from "solid-js";
import "./action-widgets.scss";

function sortByDisplayOrder(wmap: { [key: string]: WidgetConfigType }): WidgetConfigType[] {
    if (wmap == null) {
        return [];
    }
    const wlist = Object.values(wmap);
    wlist.sort((a, b) => {
        return (a["display:order"] ?? 0) - (b["display:order"] ?? 0);
    });
    return wlist;
}

/**
 * Determine whether a widget is hidden.
 * Priority: settings["widget:hidden@<key>"] > widget["display:hidden"] > false
 */
function isWidgetHidden(settings: Record<string, any>, widgetKey: string, widgetConfig: WidgetConfigType): boolean {
    const settingsKey = `widget:hidden@${widgetKey}`;
    if (settingsKey in settings) {
        return Boolean(settings[settingsKey]);
    }
    return widgetConfig?.["display:hidden"] ?? false;
}

async function handleWidgetSelect(widget: WidgetConfigType) {
    // Special handling for devtools widget
    if (widget.blockdef?.meta?.view === "devtools") {
        getApi().toggleDevtools();
        return;
    }
    // Special handling for settings widget -- open in external editor
    if (widget.blockdef?.meta?.view === "settings") {
        try {
            const path = await invoke<string>("ensure_settings_file");
            await invoke("open_in_editor", { path });
        } catch (e) {
            console.error("Failed to open settings:", e);
        }
        return;
    }
    const blockDef = widget.blockdef;
    createBlock(blockDef, widget.magnified);
}

const ActionWidget = ({
    widget,
    widgetKey,
    iconOnly,
    settings,
}: {
    widget: WidgetConfigType;
    widgetKey?: string;
    iconOnly: boolean;
    settings: Record<string, any>;
}): JSX.Element => {
    if (widgetKey && isWidgetHidden(settings, widgetKey, widget)) {
        return null;
    }

    return (
        <div data-tauri-drag-region="false">
            <Tooltip
                content={widget.description || widget.label}
                placement="bottom"
                divClassName="flex flex-row items-center gap-1 px-2 py-0.5 text-secondary hover:bg-hoverbg hover:text-white cursor-pointer rounded-sm h-full"
                divOnClick={() => handleWidgetSelect(widget)}
            >
                <div style={{ color: widget.color }} class="text-sm">
                    <i class={makeIconClass(widget.icon, true, { defaultIcon: "browser" })}></i>
                </div>
                <Show when={!iconOnly && !isBlank(widget.label)}>
                    <div class="text-xs whitespace-nowrap">{widget.label}</div>
                </Show>
            </Tooltip>
        </div>
    );
};

const ActionWidgets = (): JSX.Element => {
    const fullConfig = atoms.fullConfigAtom;
    const settings = (): Record<string, any> => fullConfig()?.settings ?? {};
    const iconOnly = (): boolean => settings()["widget:icononly"] ?? false;
    const widgets = (): WidgetConfigType[] => sortByDisplayOrder(fullConfig()?.widgets);

    // Build widget key lookup
    const widgetKeyMap = (): Map<WidgetConfigType, string> => {
        const map = new Map<WidgetConfigType, string>();
        if (fullConfig()?.widgets) {
            for (const [key, w] of Object.entries(fullConfig().widgets)) {
                map.set(w as WidgetConfigType, key);
            }
        }
        return map;
    };

    const handleWidgetsBarContextMenu = (e: MouseEvent) => {
        e.preventDefault();
        const menu: ContextMenuItem[] = [
            {
                label: "Icon Only",
                type: "checkbox",
                checked: iconOnly(),
                click: () => {
                    fireAndForget(async () => {
                        await RpcApi.SetConfigCommand(TabRpcClient, { "widget:icononly": !iconOnly() } as any);
                    });
                },
            },
        ];
        ContextMenuModel.showContextMenu(menu, e);
    };

    return (
        <div
            class="action-widgets"
            data-testid="action-widgets"
            onContextMenu={handleWidgetsBarContextMenu}
        >
            <For each={widgets()}>
                {(data, idx) => (
                    <ActionWidget
                        widget={data}
                        widgetKey={widgetKeyMap().get(data)}
                        iconOnly={iconOnly()}
                        settings={settings()}
                    />
                )}
            </For>
        </div>
    );
};

export { ActionWidgets };
