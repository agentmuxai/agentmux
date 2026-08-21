// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentRegistrationPanel — the "Registration" tab body inside
 * AgentStashModal. Read-only view of this agent's live jekt/muxbus
 * delivery status: which block/instance owns it locally, whether any
 * OTHER instance/channel on this host also currently claims the same
 * agent_id (the actual risk signal — issue #2694's root cause was exactly
 * two panes racing to hold the same agent_id), and whether a recent
 * delivery hit the #2695 identity-mismatch guard. See issue #2696.
 *
 * No live-update event exists for registration changes (unlike
 * AgentIdentityLinksPanel's `agentidentities:changed:<id>` WPS event) —
 * this is a manual-refresh snapshot, matching the "check occasionally,
 * not a persistent dashboard" way this panel is actually used.
 */

import { createEffect, createSignal, Show, type JSX } from "solid-js";
import { RpcApi, type ReactiveRegistrationsResult } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import "./AgentRegistrationPanel.scss";

interface AgentRegistrationPanelProps {
    agentId: string;
}

type BadgeState = "registered-here" | "registered-elsewhere-too" | "recent-mismatch" | "not-registered";

function badgeFor(result: ReactiveRegistrationsResult | null): {
    state: BadgeState;
    dot: "valid" | "expired" | "needs_reauth" | "unknown";
    label: string;
} {
    if (!result) {
        return { state: "not-registered", dot: "unknown", label: "Unknown" };
    }
    if (result.recent_mismatch) {
        return { state: "recent-mismatch", dot: "expired", label: "Recent mismatch detected" };
    }
    if (result.remote.length > 0) {
        return { state: "registered-elsewhere-too", dot: "needs_reauth", label: "Also registered elsewhere" };
    }
    if (result.local) {
        return { state: "registered-here", dot: "valid", label: "Registered here" };
    }
    return { state: "not-registered", dot: "unknown", label: "Not registered" };
}

function formatTimestamp(ms: number): string {
    return new Date(ms).toLocaleString();
}

export const AgentRegistrationPanel = (props: AgentRegistrationPanelProps): JSX.Element => {
    const [result, setResult] = createSignal<ReactiveRegistrationsResult | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    const [loading, setLoading] = createSignal(false);

    const refresh = async (): Promise<void> => {
        setLoading(true);
        try {
            const r = await RpcApi.GetReactiveRegistrationsCommand(TabRpcClient, { agent_id: props.agentId });
            setResult(r);
            setError(null);
        } catch (e: any) {
            setError(e?.message ?? "Failed to load registration status");
        } finally {
            setLoading(false);
        }
    };

    // Re-fetch whenever the agentId prop changes (mirrors
    // AgentIdentityLinksPanel's re-subscribe-on-agentId-change effect).
    createEffect(() => {
        if (!props.agentId) return;
        void refresh();
    });

    const badge = () => badgeFor(result());

    return (
        <div class="agent-registration-panel">
            <Show when={error()}>
                <div class="agent-registration-panel-error">{error()}</div>
            </Show>

            <div class="agent-registration-panel-header">
                <span class="agent-registration-panel-status">
                    <span class={`agent-registration-panel-dot is-${badge().dot}`} aria-hidden="true" />
                    <span class="agent-registration-panel-status-label">{badge().label}</span>
                </span>
                <button
                    type="button"
                    class="agent-registration-panel-refresh"
                    disabled={loading()}
                    onClick={() => void refresh()}
                >
                    {loading() ? "Refreshing…" : "Refresh"}
                </button>
            </div>

            <Show when={result()?.local}>
                {(local) => (
                    <div class="agent-registration-panel-section">
                        <h3>This instance</h3>
                        <dl>
                            <dt>Block</dt>
                            <dd>{local().block_id}</dd>
                            <dt>Registered</dt>
                            <dd>{formatTimestamp(local().registered_at)}</dd>
                        </dl>
                    </div>
                )}
            </Show>

            <Show when={(result()?.remote.length ?? 0) > 0}>
                <div class="agent-registration-panel-section is-warning">
                    <h3>Also registered on this host</h3>
                    <p class="agent-registration-panel-hint">
                        Another AgentMux instance on this machine currently claims this same agent
                        identity. Messages addressed to this agent may be delivered to whichever
                        instance registered most recently instead of this one.
                    </p>
                    <ul>
                        {result()!.remote.map((entry) => (
                            <li>
                                channel <strong>{entry.channel}</strong> (pid {entry.pid}), updated{" "}
                                {formatTimestamp(entry.updated_at)}
                            </li>
                        ))}
                    </ul>
                </div>
            </Show>

            <Show when={result()?.recent_mismatch}>
                {(mismatch) => (
                    <div class="agent-registration-panel-section is-error">
                        <h3>Recent identity mismatch</h3>
                        <p class="agent-registration-panel-hint">
                            A message addressed to this agent was rejected because the block it
                            resolved to reported a different live identity — delivery was refused
                            rather than sent to the wrong recipient.
                        </p>
                        <dl>
                            <dt>When</dt>
                            <dd>{formatTimestamp(mismatch().timestamp)}</dd>
                            <dt>Block</dt>
                            <dd>{mismatch().block_id}</dd>
                            <Show when={mismatch().error_message}>
                                <dt>Detail</dt>
                                <dd>{mismatch().error_message}</dd>
                            </Show>
                        </dl>
                    </div>
                )}
            </Show>

            <Show when={!result()?.local && !loading() && !error()}>
                <div class="agent-registration-panel-hint">
                    This agent has no live jekt registration on this instance. It may not be
                    running, or it may not be an agent-addressable pane.
                </div>
            </Show>
        </div>
    );
};

AgentRegistrationPanel.displayName = "AgentRegistrationPanel";
