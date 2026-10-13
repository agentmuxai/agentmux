// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { PopoverMenu, type PopoverMenuItem } from "@/app/element/popover-menu";
import { hostHas } from "@/app/host/host-caps";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { createTab } from "@/store/global";
import { openNewWindowFromHere } from "@/app/store/window-appearance";
import { fireAndForget } from "@/util/util";
import { createEffect, createSignal, untrack, type JSX } from "solid-js";
import { getEffectiveGroupedChildKeys, getHiddenKeys, getSavedPinNames, isWidgetHidden, pinIdOf } from "./action-widgets-config";

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
    // The saved pins (hidden ones included), so a toggle here never drops a pin that is hidden
    // right now; the checklist below only lists entries that aren't hidden.
    const effectivePinned = (): string[] => localPinned() ?? getSavedPinNames(settings(), widgets());
    const pinnedKeys = (): Set<string> => new Set(effectivePinned());
    // Drop the optimistic copy once the saved pins catch up with it, so a change made
    // elsewhere while this menu is open (the bar, Settings → Widgets) shows here too.
    createEffect(() => {
        const saved = getSavedPinNames(settings(), widgets());
        const local = untrack(localPinned);
        if (local && local.length === saved.length && local.every((k, i) => k === saved[i])) setLocalPinned(null);
    });

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

        if (hostHas("multiWindow")) {
            items.push({
                label: "New Window",
                click: () => {
                    fireAndForget(async () => openNewWindowFromHere());
                },
            });
        }

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
        const hidden = getHiddenKeys(settings());
        const widgetEntries = Object.entries(widgets())
            .filter(([key]) => !grouped.has(pinIdOf(key)))
            // Hidden in Settings → Widgets: not offered for pinning until it's shown again there.
            .filter(([key]) => !isWidgetHidden(key, widgets(), settings(), hidden))
            .sort(([, a]: any, [, b]: any) => (a["display:order"] ?? 0) - (b["display:order"] ?? 0));

        if (widgetEntries.length > 0) {
            items.push({ type: "separator" });
            items.push({
                type: "section",
                label: "Pin Widgets",
                // A heading for the list below, not a fold: always open.
                collapsible: false,
                items: widgetEntries.map(([key, cfg]: [string, any]) => {
                    const shortName = pinIdOf(key);
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
