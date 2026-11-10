// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Follows srv's `agent:notice` events and shows each as a card or a line in the pane of the agent
 * that is acting (docs/specs/SPEC_AGENT_SPAWN_NOTICE_IN_THE_PANE_2026_11_08.md §2), the way the
 * Vault and desktop requests are. The person's answer goes to srv the one way it can, through
 * `agent.notice.decide`, which srv accepts only from the AgentMux window, so no agent can stop,
 * hurry or grant on the person's behalf. A card for a pane this window doesn't show waits for it
 * (`paneRequests`), and the tone and notification come from srv's attention event.
 */

import { startAgentGrants } from "@/app/store/agent-grants";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { paneRequests, type PaneRequestHandle } from "@/app/store/pane-requests";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { NoticeView } from "@/types/rpc/NoticeView";
import { AgentNoticeCard, AgentNoticeLine, type NoticeChoice } from "./agent-notice-card";

/** How long a one-line notice stays. */
export const LINE_SHOWN_MS = 12_000;

const shown = new Map<string, { handle: PaneRequestHandle; timer?: ReturnType<typeof setTimeout> }>();

function decide(n: NoticeView, choice: NoticeChoice): Promise<void> {
    return RpcApi.AgentNoticeDecideCommand(TabRpcClient, { request_id: n.id, choice }).then(() => {});
}

function drop(id: string): void {
    const s = shown.get(id);
    if (!s) return;
    shown.delete(id);
    if (s.timer) clearTimeout(s.timer);
    s.handle.close();
}

function show(n: NoticeView): void {
    if (!n.block_id || shown.has(n.id)) return;
    if (n.status === "line") {
        const handle = paneRequests.push(n.block_id, {
            source: "agent-notice-line",
            Component: AgentNoticeLine,
            props: { notice: n },
            summary: n.headline,
            waits: false,
        });
        shown.set(n.id, { handle, timer: setTimeout(() => drop(n.id), LINE_SHOWN_MS) });
        return;
    }
    const handle = paneRequests.push(n.block_id, {
        source: "agent-notice",
        Component: AgentNoticeCard,
        props: { notice: n, decide: (choice: NoticeChoice) => decide(n, choice) },
        deadlineMs: n.deadline_ms,
        summary: n.headline,
        waits: false,
        // The pane was closed with the countdown running: nothing is left to start it for.
        onGone: () => void decide(n, "veto").catch(() => {}),
    });
    shown.set(n.id, { handle });
}

let started = false;

/** Show each agent notice in its pane, and follow the agents the person said Always to. Call once per window. */
export function startAgentNotices(): void {
    if (started) return;
    started = true;
    startAgentGrants();
    onMuxEvent(WpsEvent.AgentNotice, (n) => {
        if (n.status === "cleared") drop(n.id);
        else show(n);
    });
}

/** Tests only. */
export function __resetAgentNotices(): void {
    for (const id of [...shown.keys()]) drop(id);
    started = false;
}
