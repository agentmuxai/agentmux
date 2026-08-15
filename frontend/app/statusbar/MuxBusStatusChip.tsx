// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MuxBusStatusChip — always-visible red chip in the status bar when this
 * instance has no valid MuxBus session. The Instance Panel's pill (#2580)
 * proved insufficient on its own: a popover-only indicator still let an
 * instance sit for hours with WAN jekt delivery silently dead, because
 * nobody opens a popover they don't already suspect. Renders nothing while
 * the session is valid (or status hasn't loaded yet); clicking runs the
 * same browser PKCE login as the network popover's Sign in button.
 */

import { onCleanup, onMount, Show, type JSX } from "solid-js";
import { useMuxBusStatus } from "@/app/view/accounts/AgentMuxConnectPanel";

const REFRESH_INTERVAL_MS = 60_000;

export const MuxBusStatusChip = (): JSX.Element => {
    const muxbus = useMuxBusStatus();

    onMount(() => {
        void muxbus.refresh();
        const timer = window.setInterval(() => void muxbus.refresh(), REFRESH_INTERVAL_MS);
        onCleanup(() => window.clearInterval(timer));
    });

    const disconnected = () => {
        const s = muxbus.status();
        return s !== null && !(s.connected && s.valid);
    };

    return (
        <Show when={muxbus.isConfigured() && disconnected()}>
            <button
                type="button"
                class="muxbus-status-chip"
                classList={{ "muxbus-status-chip--busy": muxbus.loading() }}
                title="No valid MuxBus session — cloud/WAN jekt delivery will not work until you sign in. Click to sign in."
                aria-label="MuxBus not connected — click to sign in"
                disabled={muxbus.loading()}
                onClick={() => void muxbus.connect()}
            >
                <span class="muxbus-status-chip-dot" aria-hidden="true" />
                MuxBus
            </button>
        </Show>
    );
};

MuxBusStatusChip.displayName = "MuxBusStatusChip";
