// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A History pane's instance, loaded with the first one (history.tsx).

import type { PaneTabHostContext, PaneTabInstance } from "@/app/block/pane-tab-registry";
import { readZoom } from "@/app/store/zoom-factor";
import { HistoryManager } from "./history-manager";
import "./history.scss";

export function create(ctx: PaneTabHostContext): PaneTabInstance {
    return {
        component: () => (
            <div class="history-container">
                <div class="history-pane" style={{ zoom: readZoom(ctx.meta()) }}>
                    <HistoryManager paneBlockId={ctx.blockId} visible={ctx.visibility() === "active"} />
                </div>
            </div>
        ),
    };
}
