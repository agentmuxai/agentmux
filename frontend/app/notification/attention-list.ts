// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Everything that waits on the person, as one list (SPEC_ATTENTION_TOAST_WITH_LINK_TO_AGENT_2026_11_02 §3.1):
 * this window's registry (`waitingPanes`, with what each wants, instantly) joined with srv's agent runtime rows
 * that are `waiting_on_user`, which cover the agents of every other window. A pane is one item, whichever
 * feed names it; the registry's words win.
 */

import { sharedAgentRuntime } from "@/app/store/agent-runtime-live";
import { MOS, workspace } from "@/app/store/global";
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { waitingPanes, type WaitingPane } from "./waiting-for-you";

export interface AttentionItem {
    /** The pane that asks; the click reveals it. */
    blockId: string;
    /** The agent's name, or "An agent" when srv has no row for the pane. */
    name: string;
    /** The agent's pane colour, from the shared palette, once it has had one. */
    color?: string;
    /** What it wants, in a few words, led by a verb. */
    what: string;
    /** When it began to wait (unix ms) as this window saw it. */
    since: number;
    /** False only when the pane is known to be in another window; a pane this window cannot place counts as here. */
    local: boolean;
}

/** What an agent that srv says waits on the user, with no words of its own, wants. */
const REMOTE_WHAT = "is waiting for you";

/** The items, oldest first, from this window's registry `panes` and srv's `rows`. A pure join, for tests. */
export function mergeAttention(
    panes: WaitingPane[],
    rows: AgentRuntime[],
    seen: (blockId: string) => number,
    here: (blockId: string) => boolean | undefined = () => undefined
): AttentionItem[] {
    const byBlock = new Map<string, AgentRuntime>();
    for (const r of rows) if (r.block_id) byBlock.set(r.block_id, r);
    const items: AttentionItem[] = panes.map((p) => {
        const row = byBlock.get(p.blockId);
        return {
            blockId: p.blockId,
            name: row?.name || "An agent",
            color: row?.color,
            what: p.what,
            since: p.since,
            local: true,
        };
    });
    const have = new Set(panes.map((p) => p.blockId));
    for (const r of rows) {
        if (r.state !== "waiting_on_user" || !r.block_id || have.has(r.block_id)) continue;
        items.push({
            blockId: r.block_id,
            name: r.name || "An agent",
            color: r.color,
            what: REMOTE_WHAT,
            since: seen(r.block_id),
            local: here(r.block_id) !== false,
        });
    }
    return items.sort((a, b) => a.since - b.since);
}

/** Whether `blockId` is in a tab of this window's workspace: undefined while a tab is not loaded to say. */
function paneIsHere(blockId: string): boolean | undefined {
    const ws = workspace();
    if (!ws) return undefined;
    let unknown = false;
    for (const tabId of [...(ws.pinnedtabids ?? []), ...(ws.tabids ?? [])]) {
        const tab = MOS.getObjectValue<Tab>(MOS.makeORef("tab", tabId));
        if (!tab) unknown = true;
        else if (tab.blockids?.includes(blockId)) return true;
    }
    return unknown ? undefined : false;
}

// When this window first saw a pane that srv alone reports waiting, so its row has an age; forgotten once it stops.
const firstSeen = new Map<string, number>();

/** The panes that wait on the person, anywhere on this machine; reactive. */
export function attentionItems(): AttentionItem[] {
    const rows = sharedAgentRuntime().rows();
    const items = mergeAttention(
        waitingPanes(),
        rows,
        (blockId) => {
            let t = firstSeen.get(blockId);
            if (t == null) {
                t = Date.now();
                firstSeen.set(blockId, t);
            }
            return t;
        },
        paneIsHere
    );
    const live = new Set(items.map((i) => i.blockId));
    for (const id of firstSeen.keys()) if (!live.has(id)) firstSeen.delete(id);
    return items;
}
