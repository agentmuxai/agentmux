// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** `openfloatingpane` in the live window: `open-floating-pane.ts` wired to the host and srv. */
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { liveFloaterWindow, openFloatingPane } from "@/app/store/open-floating-pane";
import { getApi } from "@/util/app-api";
import type { OpenFloatingPaneEvent } from "@/types/rpc/OpenFloatingPaneEvent";

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
