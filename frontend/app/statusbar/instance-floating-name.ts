// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { FloatingPaneEntry } from "@/store/global";
import { getObjectValue, makeORef } from "@/store/mos";
import { resolveFloatingPaneName } from "@/util/window-title";

const PANE_VIEW_LABELS: Record<string, string> = {
    agent: "Agent",
    term: "Terminal",
    browser: "Browser",
    editor: "Editor",
    sysinfo: "System Info",
    drone: "Drone",
    swarm: "Swarm",
    help: "Help",
    warden: "Jobs",
    history: "History",
    audit: "Audit",
    vault: "Vault",
};

// Readable label for a floating pane. Resolution order:
//   1. block view type (title-cased, e.g. "Agent", "Terminal")
//   2. workspace name
//   3. positional fallback "Pane N"
export function resolveFloatingName(entry: FloatingPaneEntry, idx: number): string {
    let blockViewLabel: string | undefined;
    let workspaceName: string | undefined;
    if (entry.windowId) {
        const win = getObjectValue<MuxWindow>(makeORef("window", entry.windowId));
        if (win?.workspaceid) {
            const ws = getObjectValue<Workspace>(makeORef("workspace", win.workspaceid));
            workspaceName = ws?.name;
            const tab = ws?.activetabid ? getObjectValue<Tab>(makeORef("tab", ws.activetabid)) : null;
            const blockId = tab?.blockids?.[0];
            if (blockId) {
                const block = getObjectValue<Block>(makeORef("block", blockId));
                const view = block?.meta?.view as string | undefined;
                if (view) blockViewLabel = PANE_VIEW_LABELS[view] ?? view;
            }
        }
    }
    return resolveFloatingPaneName({ blockViewLabel, workspaceName, indexInOpenPanes: idx });
}
