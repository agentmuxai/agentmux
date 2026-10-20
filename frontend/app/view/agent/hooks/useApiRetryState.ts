// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
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

/**
 * The API request this pane's Claude CLI is retrying, or `null`: srv's
 * `agent-api-retry` event (`agent_api_retry.rs`). The event isn't replayed,
 * so a pane asks for the current retry when it mounts; a live event always
 * wins over that answer.
 */
export function useApiRetryState(blockId: string): Accessor<ApiRetryView | null> {
    const [retry, setRetry] = createSignal<ApiRetryView | null>(null);
    let receivedLiveEvent = false;
    const unsub = onMuxEvent(
        WpsEvent.AgentApiRetry,
        (data) => {
            receivedLiveEvent = true;
            setRetry(toApiRetryView(data.retry, Date.now()));
        },
        `block:${blockId}`
    );
    void RpcApi.AgentApiRetryCommand(TabRpcClient, { blockid: blockId })
        .then((current) => {
            if (!receivedLiveEvent) setRetry(toApiRetryView(current.retry, Date.now()));
        })
        .catch((e) => console.log("[useApiRetryState] failed to load the current retry", e));
    onCleanup(() => {
        try {
            unsub();
        } catch {
            /* ignore */
        }
    });
    return retry;
}
