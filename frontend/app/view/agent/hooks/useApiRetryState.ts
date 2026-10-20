// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { ApiRetryEvent } from "@/types/rpc/ApiRetryEvent";
import type { ApiRetryState } from "@/types/rpc/ApiRetryState";
import { createSignal, onCleanup, type Accessor } from "solid-js";

/** A retry with its next attempt on this renderer's clock. */
export type ApiRetryView = Omit<ApiRetryState, "retryInMs"> & { nextRetryAt: number };

/** srv sends the time remaining, not a deadline: the two clocks need not agree. */
export function toApiRetryView(retry: ApiRetryState | null, receivedAt: number): ApiRetryView | null {
    if (!retry) return null;
    const { retryInMs, ...rest } = retry;
    return { ...rest, nextRetryAt: receivedAt + retryInMs };
}

/** What the pane shows, and the revision it came from. */
export interface AppliedApiRetry {
    revision: number;
    view: ApiRetryView | null;
}

/**
 * `event` applied over `applied`, or `null` when it is no newer: events and
 * the mount-time answer can arrive out of order, and srv's revision says
 * which is current.
 */
export function applyApiRetryEvent(
    applied: AppliedApiRetry | null,
    event: ApiRetryEvent,
    receivedAt: number
): AppliedApiRetry | null {
    if (applied && event.revision <= applied.revision) return null;
    return { revision: event.revision, view: toApiRetryView(event.retry, receivedAt) };
}

/**
 * The API request this pane's Claude CLI is retrying, or `null`: srv's
 * `agent-api-retry` event (`agent_api_retry.rs`). The event isn't replayed,
 * so a pane also asks for the current retry when it mounts.
 */
export function useApiRetryState(blockId: string): Accessor<ApiRetryView | null> {
    const [applied, setApplied] = createSignal<AppliedApiRetry | null>(null);
    const apply = (event: ApiRetryEvent) => {
        const next = applyApiRetryEvent(applied(), event, Date.now());
        if (next) setApplied(next);
    };
    const unsub = onMuxEvent(WpsEvent.AgentApiRetry, apply, `block:${blockId}`);
    void RpcApi.AgentApiRetryCommand(TabRpcClient, { blockid: blockId })
        .then(apply)
        .catch((e) => console.log("[useApiRetryState] failed to load the current retry", e));
    onCleanup(() => {
        try {
            unsub();
        } catch {
            /* ignore */
        }
    });
    return () => applied()?.view ?? null;
}
