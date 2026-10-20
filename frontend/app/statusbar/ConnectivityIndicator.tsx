// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The status bar's Offline indicator: nothing while every service answers,
 * red "Offline" when one can't be reached at all, amber "Connection issues"
 * when some of its requests keep failing. The tooltip names each service
 * and since when. State: `@/store/connectivity`.
 */

import { createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";

import { connectivity, connectivityIndicator, loadConnectivity } from "@/store/connectivity";

/** How often the indicator re-reads the clock, for the degraded delay. */
const INDICATOR_CLOCK_MS = 15_000;

export function ConnectivityIndicator(): JSX.Element {
    const [now, setNow] = createSignal(Date.now());
    onMount(() => {
        if (connectivity() === null) void loadConnectivity();
        const timer = window.setInterval(() => setNow(Date.now()), INDICATOR_CLOCK_MS);
        onCleanup(() => window.clearInterval(timer));
    });
    const indicator = () => connectivityIndicator(connectivity(), now());

    return (
        <Show when={indicator()}>
            {(ind) => (
                <div
                    class="status-bar-item status-connectivity"
                    classList={{
                        "status-connectivity--warn": ind().tone === "warn",
                        "status-connectivity--error": ind().tone === "error",
                    }}
                    role="status"
                    data-tip={ind().tip}
                    aria-label={ind().tip}
                >
                    <span class="status-icon" aria-hidden="true">
                        <i class="fa fa-wifi" />
                    </span>
                    <span>{ind().label}</span>
                </div>
            )}
        </Show>
    );
}

ConnectivityIndicator.displayName = "ConnectivityIndicator";
