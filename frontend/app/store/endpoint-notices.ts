// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The notices for an agent whose model server was changed through `Agents env_set`
 * (`crates/srv/src/backend/endpoint_notice.rs`): who pointed it where, and an Undo for a short while. The
 * event comes from srv's own record, never from an agent's words. The agent's pane shows it as a card
 * (`view/agent/endpoint-notice-cards.tsx`) and My Agents' Environment panel as a line.
 */

import { createSignal } from "solid-js";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { EndpointNoticeEvent } from "@/types/rpc/EndpointNoticeEvent";

const [notices, setNotices] = createSignal<Record<string, EndpointNoticeEvent>>({});
const listeners = new Set<(e: EndpointNoticeEvent) => void>();

/** The latest notice for the agent definition `agentId`, as srv last said. */
export function endpointNoticeOf(agentId: string): EndpointNoticeEvent | undefined {
    return notices()[agentId];
}

/** Calls `fn` for every notice event from now on. Returns the way to stop. */
export function onEndpointNotice(fn: (e: EndpointNoticeEvent) => void): () => void {
    listeners.add(fn);
    return () => listeners.delete(fn);
}

/** What the notice says, in a line. */
export function endpointNoticeText(e: EndpointNoticeEvent): string {
    if (e.cleared) return `${e.by} pointed ${e.agent_name} back at Anthropic.`;
    const model = e.model ? ` (model ${e.model})` : "";
    return `${e.by} pointed ${e.agent_name} at ${e.base_url ?? "another server"}${model}.`;
}

/** Whether the notice can still be undone at `now`. */
export function canUndoEndpoint(e: EndpointNoticeEvent, now: number): boolean {
    return e.state === "open" && now <= e.expires_at;
}

/** Puts the environment back as it was; the reason when srv says no. */
export async function undoEndpoint(noticeId: string): Promise<string | null> {
    const result = await RpcApi.UndoAgentEndpointCommand(TabRpcClient, { notice_id: noticeId });
    return result.restored ? null : (result.reason ?? "it could not be undone");
}

let started = false;

/** Follow srv's notices. Call once per window. */
export function startEndpointNotices(): void {
    if (started) return;
    started = true;
    onMuxEvent(WpsEvent.AgentEndpointNotice, (e) => {
        setNotices((all) => ({ ...all, [e.agent_id]: e }));
        for (const fn of [...listeners]) fn(e);
    });
}
