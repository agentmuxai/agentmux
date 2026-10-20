// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { readMuxEventHistory } from "@/app/store/mps-history";
import type { ApiRetryState } from "@/types/rpc/ApiRetryState";
import { createSignal, onCleanup, type Accessor } from "solid-js";

/**
 * The API request this pane's Claude CLI is retrying, or `null`: srv's
 * `agent-api-retry` event (`agent_api_retry.rs`). Reads the persisted latest
 * on mount as well as subscribing, because a same-connection remount gets no
 * replay (see useResumeRetryStream.ts); a live event always wins over that read.
 */
export function useApiRetryState(blockId: string): Accessor<ApiRetryState | null> {
    const [retry, setRetry] = createSignal<ApiRetryState | null>(null);
    let receivedLiveEvent = false;
    const unsub = onMuxEvent(
        WpsEvent.AgentApiRetry,
        (data) => {
            receivedLiveEvent = true;
            setRetry(data.retry ?? null);
        },
        `block:${blockId}`
    );
    void readMuxEventHistory(WpsEvent.AgentApiRetry, `block:${blockId}`, 1)
        .then((history) => {
            const latest = history[history.length - 1];
            if (!receivedLiveEvent && latest) setRetry(latest.retry ?? null);
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
