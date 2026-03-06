// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { Tooltip } from "@/app/element/tooltip";
import { ContextMenuModel } from "@/app/store/contextmenu";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { atoms, createBlock, getApi } from "@/store/global";
import { fireAndForget, isBlank, makeIconClass } from "@/util/util";
import clsx from "clsx";
import { useAtomValue } from "jotai";
import { memo } from "react";

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

async function handleWidgetSelect(widget: WidgetConfigType) {
    // Special handling for devtools widget
    if (widget.blockdef?.meta?.view === "devtools") {
        getApi().toggleDevtools();
        return;
    }
    // Special handling for settings widget -- open in external editor
    if (widget.blockdef?.meta?.view === "settings") {
        const path = `${getApi().getConfigDir()}/settings.json`;
        getApi().openNativePath(path);
        return;
    }
    const blockDef = widget.blockdef;
    createBlock(blockDef, widget.magnified);
}

const HorizontalWidget = memo(({ widget }: { widget: WidgetConfigType }) => {
    if (widget["display:hidden"]) {
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
                <div style={{ color: widget.color }} className="text-sm">
                    <i className={makeIconClass(widget.icon, true, { defaultIcon: "browser" })}></i>
                </div>
                {!isBlank(widget.label) && (
                    <div className="text-xs whitespace-nowrap">{widget.label}</div>
                )}
            </Tooltip>
        </div>
    );
});

const WidgetBar = memo(() => {
    const fullConfig = useAtomValue(atoms.fullConfigAtom);

    const helpWidget: WidgetConfigType = {
        icon: "circle-question",
        label: "help",
        blockdef: {
            meta: {
                view: "help",
            },
        },
    };
    const settingsWidget: WidgetConfigType = {
        icon: "cog",
        label: "settings",
        description: "Open Settings (external editor)",
        blockdef: {
            meta: {
                view: "settings",
            },
        },
    };
    const devToolsWidget: WidgetConfigType = {
        icon: "code",
        label: "devtools",
        description: "Toggle Developer Tools",
        blockdef: {
            meta: {
                view: "devtools",
            },
        },
    };
    const showHelp = fullConfig?.settings?.["widget:showhelp"] ?? true;
    const widgets = sortByDisplayOrder(fullConfig?.widgets);

    const handleWidgetsBarContextMenu = (e: React.MouseEvent) => {
        e.preventDefault();
        const menu: ContextMenuItem[] = [
            {
                label: "Edit widgets.json",
                click: () => {
                    fireAndForget(async () => {
                        const path = `${getApi().getConfigDir()}/widgets.json`;
                        const blockDef: BlockDef = {
                            meta: { view: "preview", file: path },
                        };
                        await createBlock(blockDef, false, true);
                    });
                },
            },
            {
                label: "Show Help Widgets",
                submenu: [
                    {
                        label: "On",
                        type: "checkbox",
                        checked: showHelp,
                        click: () => {
                            fireAndForget(async () => {
                                await RpcApi.SetConfigCommand(TabRpcClient, { "widget:showhelp": true });
                            });
                        },
                    },
                    {
                        label: "Off",
                        type: "checkbox",
                        checked: !showHelp,
                        click: () => {
                            fireAndForget(async () => {
                                await RpcApi.SetConfigCommand(TabRpcClient, { "widget:showhelp": false });
                            });
                        },
                    },
                ],
            },
        ];
        ContextMenuModel.showContextMenu(menu, e);
    };

    return (
        <div
            className="flex flex-row items-center gap-1 h-full px-1 select-none"
            onContextMenu={handleWidgetsBarContextMenu}
        >
            {widgets?.map((data, idx) => <HorizontalWidget key={`widget-${idx}`} widget={data} />)}
            {showHelp && <HorizontalWidget key="help" widget={helpWidget} />}
            <HorizontalWidget key="settings" widget={settingsWidget} />
            <HorizontalWidget key="devtools" widget={devToolsWidget} />
        </div>
    );
});

export { WidgetBar };
