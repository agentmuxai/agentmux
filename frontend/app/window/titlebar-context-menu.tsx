// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { PopoverMenu, type PopoverMenuItem } from "@/app/element/popover-menu";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { createTab, getApi } from "@/store/global";
import { fireAndForget } from "@/util/util";
import { createSignal, type JSX } from "solid-js";
import { getEffectiveGroupedChildKeys, getPinnedKeys } from "./action-widgets-config";

interface TitleBarContextMenuProps {
    pos: { x: number; y: number };
    fullConfig: any;
    onClose: () => void;
}

const TitleBarContextMenu = (props: TitleBarContextMenuProps): JSX.Element => {
    const settings = (): Record<string, any> => props.fullConfig?.settings ?? {};
    const widgets  = (): Record<string, any> => props.fullConfig?.widgets  ?? {};
    const iconOnly = (): boolean => settings()["widget:icononly"] ?? false;

    // Optimistic local state for pinned keys. Initialized lazily from server
    // config on first read. Each toggle updates this immediately so rapid
    // checkbox clicks chain off the already-updated list rather than
    // re-reading stale settings() and overwriting each other.
    const [localPinned, setLocalPinned] = createSignal<string[] | null>(null);
    const effectivePinned = (): string[] => localPinned() ?? getPinnedKeys(settings(), widgets());
    const pinnedKeys = (): Set<string> => new Set(effectivePinned());

    const toggleWidget = (shortName: string) => {
        const current = effectivePinned();
        const next = pinnedKeys().has(shortName)
            ? current.filter((k) => k !== shortName)
            : [...current, shortName];
        setLocalPinned(next); // optimistic — drives UI immediately
        fireAndForget(async () => {
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
                fireAndForget(async () => getApi().openNewWindow());
            },
        });

        items.push({
            label: "New Tab",
            click: () => {
                createTab();
            },
        });

        items.push({
            type: "checkbox",
            label: "Show Icons Only",
            checked: iconOnly(),
            click: () => {
                toggleIconOnly();
            },
        });

        // A grouped child not currently individually pinned (e.g.
        // Messengers' Discord/Slack/etc.) is excluded here the same way
        // it's excluded from the bar's own pinned/More lists — only
        // reachable by expanding its parent's flyout. Once a user promotes
        // one out via that flyout's own right-click "Pin to bar", it shows
        // up here too, checked, like any other pinned widget.
        const grouped = getEffectiveGroupedChildKeys(widgets(), settings());
        const widgetEntries = Object.entries(widgets())
            .filter(([key]) => key.startsWith("defwidget@") && !grouped.has(key.replace("defwidget@", "")))
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

        return items;
    };

    return <PopoverMenu items={buildItems()} pos={props.pos} onClose={props.onClose} />;
};

export { TitleBarContextMenu };
