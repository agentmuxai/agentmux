// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The status bar's cloud indicator: quiet while cloud messages arrive, amber
 * "MuxBus reconnecting…" once the relay has been unreachable a while, red
 * "MuxBus: sign in" when the sign-in stopped working (click to sign in).
 * Nothing for a channel that isn't signed in. State: `@/store/muxbus-delivery`.
 */

import { createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";

import { useMuxBusStatus } from "@/app/view/accounts/AgentMuxConnectPanel";
import { muxbusDelivery, muxbusIndicator } from "@/store/muxbus-delivery";

/** How often the indicator re-reads the clock, for the reconnecting delay. */
export const INDICATOR_CLOCK_MS = 15_000;

export function MuxBusIndicator(): JSX.Element {
    const muxbus = useMuxBusStatus();
    const [now, setNow] = createSignal(Date.now());
    onMount(() => {
        if (muxbusDelivery() === null) void muxbus.refresh();
        const timer = window.setInterval(() => setNow(Date.now()), INDICATOR_CLOCK_MS);
        onCleanup(() => window.clearInterval(timer));
    });
    const indicator = () => muxbusIndicator(muxbusDelivery(), now());
    const signIn = () => {
        if (!indicator()?.signIn || muxbus.loading()) return;
        void muxbus.connect();
    };

    return (
        <Show when={indicator()}>
            {(ind) => (
                <div
                    class="status-bar-item status-muxbus"
                    classList={{
                        clickable: ind().signIn,
                        "status-muxbus--warn": ind().tone === "warn",
                        "status-muxbus--error": ind().tone === "error",
                    }}
                    role={ind().signIn ? "button" : "status"}
                    tabIndex={ind().signIn ? 0 : undefined}
                    data-tip={ind().tip}
                    aria-label={ind().tip}
                    onClick={signIn}
                    onKeyDown={(e) => {
                        if (e.key === "Enter" || e.key === " ") {
                            e.preventDefault();
                            signIn();
                        }
                    }}
                >
                    <span class="status-icon" aria-hidden="true">
                        <i class="fa fa-cloud" />
                    </span>
                    <span>{muxbus.loading() ? "Signing in…" : ind().label}</span>
                </div>
            )}
        </Show>
    );
}

MuxBusIndicator.displayName = "MuxBusIndicator";
