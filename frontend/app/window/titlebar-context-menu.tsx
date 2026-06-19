// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { PopoverMenu, type PopoverMenuItem } from "@/app/element/popover-menu";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { atoms, createTab, getApi } from "@/store/global";
import { fireAndForget } from "@/util/util";
import { type JSX } from "solid-js";

function getPinnedKeys(settings: Record<string, any>, widgets: Record<string, any>): string[] {
    const pinned: string[] | undefined = settings["widget:pinned"];
    if (pinned !== undefined) return pinned;
    return Object.entries(widgets)
        .filter(([, w]: any) => w["display:pinned"])
        .sort(([, a]: any, [, b]: any) => (a["display:order"] ?? 0) - (b["display:order"] ?? 0))
        .map(([key]) => key.replace("defwidget@", ""));
}

interface TitleBarContextMenuProps {
    pos: { x: number; y: number };
    fullConfig: any;
    onClose: () => void;
}

const TitleBarContextMenu = (props: TitleBarContextMenuProps): JSX.Element => {
    const settings = (): Record<string, any> => props.fullConfig?.settings ?? {};
    const widgets  = (): Record<string, any> => props.fullConfig?.widgets  ?? {};
    const iconOnly = (): boolean => settings()["widget:icononly"] ?? false;

    const pinnedKeys = (): Set<string> => new Set(getPinnedKeys(settings(), widgets()));

    const toggleWidget = (shortName: string) => {
        fireAndForget(async () => {
            const current = getPinnedKeys(settings(), widgets());
            const next = pinnedKeys().has(shortName)
                ? current.filter((k) => k !== shortName)
                : [...current, shortName];
            await RpcApi.SetConfigCommand(TabRpcClient, { "widget:pinned": next } as any);
        });
    };

    const toggleIconOnly = () => {
        fireAndForget(async () => {
            await RpcApi.SetConfigCommand(TabRpcClient, { "widget:icononly": !iconOnly() } as any);
        });
    };

    const buildItems = (): PopoverMenuItem[] => {
        const items: PopoverMenuItem[] = [];

        items.push({
            label: "New Window",
            click: () => {
                props.onClose();
                fireAndForget(async () => getApi().openNewWindow());
            },
        });

        items.push({
            label: "New Tab",
            click: () => {
                props.onClose();
                createTab();
            },
        });

        const widgetEntries = Object.entries(widgets())
            .filter(([key]) => key.startsWith("defwidget@"))
            .sort(([, a]: any, [, b]: any) => (a["display:order"] ?? 0) - (b["display:order"] ?? 0));

        if (widgetEntries.length > 0) {
            items.push({ type: "separator" });
            items.push({
                type: "section",
                label: "Pin Widgets",
                defaultOpen: true,
                items: widgetEntries.map(([key, cfg]: [string, any]) => {
                    const shortName = key.replace("defwidget@", "");
                    return {
                        type: "checkbox" as const,
                        label: cfg.label ?? shortName,
                        checked: pinnedKeys().has(shortName),
                        keepOpen: true,
                        click: () => toggleWidget(shortName),
                    };
                }),
            });
        }

        items.push({ type: "separator" });
        items.push({
            type: "checkbox",
            label: "Icon Only",
            checked: iconOnly(),
            click: () => {
                props.onClose();
                toggleIconOnly();
            },
        });

        return items;
    };

    return <PopoverMenu items={buildItems()} pos={props.pos} onClose={props.onClose} />;
};

export { TitleBarContextMenu };
