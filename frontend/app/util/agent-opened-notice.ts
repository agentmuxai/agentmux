// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * `pane:agentopened`: an agent opened a pane, and its open took neither the person's tab, focus nor window
 * (REPORT_ACTION_RESULTS_ALWAYS_VISIBLE_ACROSS_WINDOWS_2026_10_31.md §4, item 9). The window the person is
 * in says so, quietly, with a Show button; the other windows say nothing.
 */

import { setNotificationAction } from "@/app/notification/usenotification";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { atoms, pushNotification } from "@/app/store/global";
import { loadObject, revealBlock } from "@/app/util/reveal-block";
import type { PaneAgentOpenedEvent } from "@/types/rpc/PaneAgentOpenedEvent";

const NOTICE_MS = 8000;

export interface AgentOpenedDeps {
    /** This window is the one the person is in. */
    focused(): boolean;
    /** The tab this window shows. */
    shownTab(): string | undefined;
    /** The pane's own name, when the open did not carry a title. */
    nameOf(blockId: string): Promise<string | undefined>;
    show(blockId: string, tabId: string): void;
    push(notice: NotificationType): void;
    onAction(key: string, fn: () => void): void;
}

/** "Settings opened": the title the agent gave, else the pane's own name, else its view. */
async function nameFor(d: PaneAgentOpenedEvent, deps: AgentOpenedDeps): Promise<string> {
    const title = d.title?.trim() || (await deps.nameOf(d.block_id));
    return title || d.view.charAt(0).toUpperCase() + d.view.slice(1);
}

/** Handle one event. Returns whether this window put the notice up. */
export async function handleAgentOpened(d: PaneAgentOpenedEvent | undefined, deps: AgentOpenedDeps): Promise<boolean> {
    if (!d?.block_id || !deps.focused() || d.tab_id === deps.shownTab()) return false;
    const id = `agent-opened:${d.block_id}`;
    deps.onAction(`${id}:show`, () => deps.show(d.block_id, d.tab_id));
    deps.push({
        id,
        icon: "fa-window-restore",
        title: `${await nameFor(d, deps)} opened`,
        message: "An agent opened it; it is not in front of you.",
        timestamp: new Date().toISOString(),
        type: "info",
        expiration: Date.now() + NOTICE_MS,
        actions: [{ label: "Show", actionKey: `${id}:show` }],
    });
    return true;
}

const liveDeps: AgentOpenedDeps = {
    focused: () => document.hasFocus(),
    shownTab: () => atoms.activeTabId(),
    nameOf: async (blockId) => {
        const meta = (await loadObject<Block>("block", blockId))?.meta ?? {};
        const name = [meta["agentName"], meta["title"], meta["name"]].find((v) => typeof v === "string" && v.trim());
        return typeof name === "string" ? name.trim() : undefined;
    },
    show: (blockId, tabId) => void revealBlock(blockId, { tabId, source: "agent-opened" }),
    push: pushNotification,
    onAction: setNotificationAction,
};

let installed = false;

/** Subscribe this window to `pane:agentopened`. Once per window. */
export function installAgentOpenedNotice(): () => void {
    if (installed) return () => {};
    installed = true;
    const unsub = onMuxEvent(WpsEvent.PaneAgentOpened, (d) => {
        void handleAgentOpened(d, liveDeps).catch((e) => console.warn("[agent-opened] notice failed", String(e)));
    });
    return () => {
        unsub();
        installed = false;
    };
}
