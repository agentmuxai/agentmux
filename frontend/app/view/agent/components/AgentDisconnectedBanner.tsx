// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentDisconnectedBanner — surfaces the agent-pane `Disconnected`
 * phase (PR F of the turn-phase migration). The banner appears
 * whenever `turnPhase.kind === "Disconnected"` and offers a manual
 * "Reconnect" button that re-subscribes to the stream. The reducer
 * resets the phase to `Idle` on `StreamSubscribe` — see
 * `frontend/app/store/agent-pane-state/reducer.ts` §StreamSubscribe.
 *
 * Spec: docs/specs/SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md §6.4.
 *
 * The pane is NOT in the "working" set while Disconnected
 * (`isWorking(state) === false` — verified by
 * `frontend/app/store/agent-pane-state/reducer.test.ts` PR F suite),
 * so the working-spinner animation is already suppressed. The banner
 * is the only disconnect-aware UI surface PR F introduces; the rest
 * of the view ignores the phase except through the existing
 * `isWorking` projection.
 */

import { Show, type Accessor, type JSX } from "solid-js";
import type { TurnPhase } from "@/app/store/agent-pane-state/types";

interface AgentDisconnectedBannerProps {
    /** Live phase accessor — banner only renders when kind=Disconnected. */
    phase: Accessor<TurnPhase>;
    /**
     * Manual-reconnect handler. The view passes the standard
     * stream-resubscribe path here (see `agent-view.tsx` wiring).
     * The reducer's `StreamSubscribe` arm clears the disconnect.
     * No-op-safe if the system auto-reconnects between render and
     * click — the second subscribe still lands in Idle.
     */
    onReconnect: () => void;
}

function formatLastConnectedAge(lastConnectedAt: number): string {
    const ageMs = Date.now() - lastConnectedAt;
    if (ageMs < 1_000) return "just now";
    const seconds = Math.floor(ageMs / 1_000);
    if (seconds < 60) return `${seconds}s ago`;
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) return `${minutes}m ago`;
    const hours = Math.floor(minutes / 60);
    return `${hours}h ago`;
}

export const AgentDisconnectedBanner = (
    props: AgentDisconnectedBannerProps,
): JSX.Element => {
    return (
        <Show when={props.phase().kind === "Disconnected"}>
            {(() => {
                const p = props.phase();
                if (p.kind !== "Disconnected") return null;
                return (
                    <div
                        class="agent-disconnected-banner"
                        role="status"
                        aria-live="polite"
                    >
                        <span
                            class="agent-disconnected-banner-icon"
                            aria-hidden="true"
                        >
                            {"⚠"}
                        </span>
                        <span class="agent-disconnected-banner-message">
                            <span class="agent-disconnected-banner-title">
                                Disconnected from stream
                            </span>
                            <span class="agent-disconnected-banner-detail">
                                {" · "}was {p.lastKind.toLowerCase()}
                                {", "}
                                {formatLastConnectedAge(p.lastConnectedAt)}
                            </span>
                        </span>
                        <button
                            type="button"
                            class="agent-disconnected-banner-action"
                            onClick={props.onReconnect}
                            title="Reconnect to the agent stream"
                        >
                            Reconnect
                        </button>
                    </div>
                );
            })()}
        </Show>
    );
};

AgentDisconnectedBanner.displayName = "AgentDisconnectedBanner";
