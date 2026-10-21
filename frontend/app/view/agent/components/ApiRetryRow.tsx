// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Button } from "@/app/element/ui";
import { Show, createSignal, onCleanup, type JSX } from "solid-js";
import type { ApiRetryView } from "../hooks/useApiRetryState";

function retryReason(status: number | null): string {
    if (status == null) return "Can't reach the API";
    if (status === 429) return "Rate-limited by the API";
    if (status === 529) return "The API is overloaded";
    return `The API returned an error (HTTP ${status})`;
}

/** "Can't reach the API. Retrying: attempt 3 of 10, next in 18 s." */
export function apiRetryText(retry: ApiRetryView, nowMs: number): string {
    const attempt =
        retry.maxRetries > 0 ? `attempt ${retry.attempt} of ${retry.maxRetries}` : `attempt ${retry.attempt}`;
    const seconds = Math.ceil((retry.nextRetryAt - nowMs) / 1000);
    const reason = retryReason(retry.errorStatus);
    return seconds > 0
        ? `${reason}. Retrying: ${attempt}, next in ${seconds} s.`
        : `${reason}. Retrying now: ${attempt}.`;
}

/**
 * While the CLI retries a failed API request, one line above the composer
 * says so, counts down to the next attempt, and offers Stop: otherwise the
 * pane just looks busy for the minutes the CLI's retries can take.
 */
export function ApiRetryRow(props: { retry: ApiRetryView | null; onStop: () => void }): JSX.Element {
    return (
        <Show when={props.retry}>
            {(retry) => {
                const [now, setNow] = createSignal(Date.now());
                const timer = setInterval(() => setNow(Date.now()), 1000);
                onCleanup(() => clearInterval(timer));
                return (
                    <div class="agent-api-retry" role="status">
                        <span class="agent-api-retry-text">{apiRetryText(retry(), now())}</span>
                        <Button density="compact" icon="stop" onClick={() => props.onStop()}>
                            Stop
                        </Button>
                    </div>
                );
            }}
        </Show>
    );
}
