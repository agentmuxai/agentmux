// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** `openfloatingpane` in the live window: `open-floating-pane.ts` wired to the host and srv. */
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { liveFloaterWindow, openFloatingPane } from "@/app/store/open-floating-pane";
import { placeFloatingPane } from "@/app/store/place-floating-pane";
import { getApi } from "@/util/app-api";
import type { OpenFloatingPaneEvent } from "@/types/rpc/OpenFloatingPaneEvent";
import type { PlaceFloatingPaneEvent } from "@/types/rpc/PlaceFloatingPaneEvent";

export function handleOpenFloatingPaneEvent(data: OpenFloatingPaneEvent): Promise<void> {
    return openFloatingPane(data, liveFloaterWindow(), {
        openFloatingPane: (args) => getApi().windows.openFloatingPane(args),
        answer: async (args) => {
            try {
                await RpcApi.PaneFloatPlacedCommand(TabRpcClient, args);
            } catch (e) {
                console.warn("[openfloatingpane] pane.floatplaced failed", String(e));
            }
        },
    });
}

/** `placefloatingpane` in the live floating window: `place-floating-pane.ts` wired to the host and srv. */
export function handlePlaceFloatingPaneEvent(data: PlaceFloatingPaneEvent): Promise<void> {
    const label = new URLSearchParams(window.location.search).get("windowLabel") ?? "main";
    return placeFloatingPane(data, liveFloaterWindow(), {
        getRect: () => getApi().windows.getRect(label),
        setRect: (rect) => getApi().windows.setRect(label, rect),
        answer: async (args) => {
            try {
                await RpcApi.PaneFloatPlacedCommand(TabRpcClient, args);
            } catch (e) {
                console.warn("[placefloatingpane] pane.floatplaced failed", String(e));
            }
        },
    });
}
