// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Hide from widget bar", from the bar's own right-click menus: hides the entry (bar, More
 * and the pane "+" picker) and says where to bring it back, with an Undo that puts back the
 * exact pins and hidden list from before.
 * SPEC_WIDGET_BAR_VISIBILITY_SETTINGS_2026_10_12.md §4.2.
 */

import { clearNotificationAction, setNotificationAction } from "@/app/notification/usenotification";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { pushNotification } from "@/store/global";
import { fireAndForget } from "@/util/util";
import { setWidgetBarState } from "./action-widgets-config";

// Each hide gets its own Undo key, so a second hide inside the first one's 8 s can't make
// the first notification's Undo restore the second snapshot.
let undoSeq = 0;

export function hideFromWidgetBar(key: string, settings: Record<string, any>, wmap: Record<string, WidgetConfigType>): void {
    const before = {
        "widget:pinned": settings["widget:pinned"] ?? null,
        "widget:hidden": settings["widget:hidden"] ?? null,
    };
    fireAndForget(async () => {
        await setWidgetBarState(key, "hidden", settings, wmap);
        const undoKey = `widgetBarUndoHide:${++undoSeq}`;
        setNotificationAction(undoKey, () => {
            clearNotificationAction(undoKey);
            fireAndForget(() => RpcApi.SetConfigCommand(TabRpcClient, before as unknown as SettingsType));
        });
        pushNotification({
            icon: "fa-eye-slash",
            title: `${wmap[key]?.label ?? "Widget"} hidden`,
            message: "Show it again in Settings → Widgets.",
            timestamp: new Date().toISOString(),
            type: "info",
            expiration: Date.now() + 8000,
            actions: [{ label: "Undo", actionKey: undoKey, color: "grey" }],
        });
    });
}
