// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Everything that waits on the person, as one list (SPEC_ATTENTION_TOAST_WITH_LINK_TO_AGENT_2026_11_02 §3.1):
 * this window's registry (`waitingPanes`, with what each wants, instantly) joined with srv's agent runtime rows
 * that are `waiting_on_user`, which cover the agents of every other window. A pane is one item, whichever
 * feed names it; the registry's words win. An agent that waits on a Browser request (`waiting_in`) is listed
 * as that Browser pane, where the answer is given.
 */

import { paneRoleColor } from "@/app/block/pane-color-scheme";
import { describePaneTab } from "@/app/element/pane-tab-model";
import { isLightThemeActive } from "@/app/block/pane-identity";
import { sharedAgentRuntime } from "@/app/store/agent-runtime-live";
import { MOS, workspace } from "@/app/store/global";
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { BROWSER_WHAT, browserWaitsShown } from "./browser-waits";
import { waitingPanes, type WaitingPane } from "./waiting-for-you";

export interface AttentionItem {
    /** The pane that asks; the click reveals it. */
    blockId: string;
    /** The agent's name; the pane's own name when it is not an agent's; "An agent" when neither is known. */
    name: string;
    /** Set when the pane is not an agent's (Remotes, Memory, a browser): the toast then says "pane", not "agent". */
    pane?: true;
    /** The agent's id, when srv has a row for the pane: what "Open" needs for an agent that has no pane. */
    agentId?: string;
    /** The agent's pane colour, from the shared palette, once it has had one. */
    color?: string;
    /** What it wants, in a few words, led by a verb. */
    what: string;
    /** When it began to wait (unix ms) as this window saw it. */
    since: number;
    /** False only when the pane is known to be in another window; a pane this window cannot place counts as here. */
    local: boolean;
}

/** An agent colour as readable text on the toast and the list in the current theme (the palette identity colour). */
export const attentionTint = (hex?: string) => paneRoleColor(undefined, hex, isLightThemeActive(), "identity");

/** What an agent that srv says waits on the user, with no words of its own, wants. */
const REMOTE_WHAT = "is waiting for you";

/** The items, oldest first, from this window's registry `panes` and srv's `rows`. A pure join, for tests. */
export function mergeAttention(
    panes: WaitingPane[],
    rows: AgentRuntime[],
    seen: (blockId: string) => number,
    here: (blockId: string) => boolean | undefined = () => undefined,
    paneName: (blockId: string) => string | undefined = () => undefined
): AttentionItem[] {
    const byBlock = new Map<string, AgentRuntime>();
    for (const r of rows) if (r.block_id) byBlock.set(r.block_id, r);
    const items: AttentionItem[] = panes.map((p) => {
        const row = byBlock.get(p.blockId);
        const own = row ? undefined : paneName(p.blockId);
        return {
            blockId: p.blockId,
            name: row?.name || own || "An agent",
            agentId: row?.agent_id,
            color: row?.color,
            what: p.what,
            since: p.since,
            local: true,
            ...(own ? { pane: true as const } : {}),
        };
    });
    const have = new Set(panes.map((p) => p.blockId));
    for (const r of rows) {
        if (r.state !== "waiting_on_user") continue;
        // A Browser request is answered in the pane it waits in, so that pane is the item, once: where the pane
        // is, this window's registry has it already. No agent id: the pane is not the agent's to open.
        const blockId = r.waiting_in ?? r.block_id;
        if (!blockId || have.has(blockId)) continue;
        have.add(blockId);
        items.push({
            blockId,
            name: r.name || "An agent",
            agentId: r.waiting_in ? undefined : r.agent_id,
            color: r.color,
            what: r.waiting_in ? BROWSER_WHAT : REMOTE_WHAT,
            since: seen(blockId),
            local: here(blockId) !== false,
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

/** What a pane that is not an agent's is called: its tab's label ("Connectors · Remotes"). None for an agent's pane. */
function ownPaneName(blockId: string): string | undefined {
    const meta = MOS.getObjectValue<Block>(MOS.makeORef("block", blockId))?.meta;
    const view = meta?.view as string | undefined;
    if (!view || view === "agent") return undefined;
    return describePaneTab({ blockId, view, meta, ordinal: 0, liveViewModel: null }).label || undefined;
}

// When this window first saw a pane that srv alone reports waiting, so its row has an age; forgotten once it stops.
const firstSeen = new Map<string, number>();

/** The rows as the list and the counts read them. With Browser waits switched off (Settings → Sounds) an agent
 *  that waits only on a Browser request reads idle here, as its pane's own announcement is left out. */
export function attentionRows(rows: AgentRuntime[], browserWaits: boolean): AgentRuntime[] {
    if (browserWaits) return rows;
    return rows.map((r) =>
        r.state === "waiting_on_user" && r.waiting_in
            ? { ...r, state: "idle" as const, detail: undefined, waiting_in: undefined }
            : r
    );
}

/** srv's runtime rows for the list and the status bar's counts; reactive. */
export const sharedAttentionRows = (): AgentRuntime[] => attentionRows(sharedAgentRuntime().rows(), browserWaitsShown());

/** The panes that wait on the person, anywhere on this machine; reactive. */
export function attentionItems(): AttentionItem[] {
    const rows = sharedAttentionRows();
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
        paneIsHere,
        ownPaneName
    );
    const live = new Set(items.map((i) => i.blockId));
    for (const id of firstSeen.keys()) if (!live.has(id)) firstSeen.delete(id);
    return items;
}
