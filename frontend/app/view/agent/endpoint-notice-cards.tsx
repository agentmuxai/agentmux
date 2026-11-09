// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Puts the notice for a changed model server (`store/endpoint-notices.ts`) in every open pane of the agent
 * it names, as a card with an Undo, and takes it away when the window is over or the change was undone.
 */
import { agentModels } from "./agent-models";
import { onEndpointNotice, startEndpointNotices } from "@/app/store/endpoint-notices";
import { paneRequests, type PaneRequestHandle } from "@/app/store/pane-requests";
import type { EndpointNoticeEvent } from "@/types/rpc/EndpointNoticeEvent";
import { EndpointNoticeCard } from "./endpoint-notice-card";

/** The agent pane blocks that run the agent definition `agentId`. */
export function blocksOf(agentId: string): string[] {
    return agentModels
        .all()
        .filter((m) => m.blockAtom()?.meta?.["agentId"] === agentId)
        .map((m) => m.blockId);
}

const cards = new Map<string, PaneRequestHandle[]>();

function closeAll(id: string): void {
    for (const card of cards.get(id) ?? []) card.close();
    cards.delete(id);
}

export function showNotice(e: EndpointNoticeEvent, now: number = Date.now()): void {
    if (e.state !== "open") {
        closeAll(e.id);
        return;
    }
    if (cards.has(e.id) || now > e.expires_at) return;
    const shown = blocksOf(e.agent_id).map((blockId) =>
        paneRequests.push(blockId, {
            source: "endpoint-notice",
            Component: EndpointNoticeCard,
            props: { notice: e },
            summary: `${e.agent_name}: model server changed`,
            deadlineMs: e.expires_at,
            waits: false,
        })
    );
    cards.set(e.id, shown);
    // The window is over: the card goes, the change stands.
    setTimeout(() => closeAll(e.id), Math.max(0, e.expires_at - now) + 1000);
}

let started = false;

/** Show every model-server change as a card in the agent's panes. Call once per window. */
export function startEndpointNoticeCards(): void {
    if (started) return;
    started = true;
    startEndpointNotices();
    onEndpointNotice((e) => showNotice(e));
}
