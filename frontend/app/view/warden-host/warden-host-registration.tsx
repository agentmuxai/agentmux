// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What a Host row expands to: whether another AgentMux instance on this host
// also claims the agent's id (issue #2694's root cause was two panes racing
// for one id), and whether a recent jekt to it was refused by the identity
// guard (#2695). Read from `reactive.registrations` by the Host list, which
// re-reads it on each refresh while a row is open, since nothing pushes a change.

import { Show, type JSX } from "solid-js";

import { RpcApi, type ReactiveRegistrationsResult } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { ageMs, formatAge } from "@/app/view/warden-shared/warden-shared";

export interface HostRegistration {
    result: ReactiveRegistrationsResult | null;
    error: string | null;
}

export async function readHostRegistration(agentId: string): Promise<HostRegistration> {
    try {
        return {
            result: await RpcApi.GetReactiveRegistrationsCommand(TabRpcClient, { agent_id: agentId }),
            error: null,
        };
    } catch (e) {
        return { result: null, error: `Couldn't read registration: ${e instanceof Error ? e.message : e}` };
    }
}

interface HostRegistrationDetailProps {
    /** Undefined until the first read returns. */
    registration: HostRegistration | undefined;
    now: number;
}

export const HostRegistrationDetail = (props: HostRegistrationDetailProps): JSX.Element => {
    const ago = (ts: number) => `${formatAge(ageMs(ts, props.now))} ago`;

    return (
        <div class="warden-host-detail">
            <Show when={props.registration?.error}>
                <div class="warden-manager-error">⚠ {props.registration?.error}</div>
            </Show>
            <Show when={props.registration?.result}>
                {(r) => (
                    <>
                        <Show when={r().remote.length > 0}>
                            <section class="warden-host-detail-section" data-kind="claims">
                                <h4>Also claimed by another instance on this host</h4>
                                <p class="warden-host-detail-hint">
                                    A jekt for this agent may go to whichever instance registered most recently instead
                                    of this one.
                                </p>
                                <ul>
                                    {r().remote.map((entry) => (
                                        <li>
                                            channel <strong>{entry.channel}</strong> (pid {entry.pid}), updated{" "}
                                            {ago(entry.updated_at)}
                                        </li>
                                    ))}
                                </ul>
                            </section>
                        </Show>
                        <Show when={r().recent_mismatch}>
                            {(mismatch) => (
                                <section class="warden-host-detail-section" data-kind="mismatch">
                                    <h4>Recent identity mismatch</h4>
                                    <p class="warden-host-detail-hint">
                                        A jekt for this agent was refused because the block it resolved to reported a
                                        different live identity.
                                    </p>
                                    <ul>
                                        <li>
                                            {ago(mismatch().timestamp)}, block{" "}
                                            <span class="warden-manager-mono">{mismatch().block_id.slice(0, 8)}</span>
                                            <Show when={mismatch().error_message}>: {mismatch().error_message}</Show>
                                        </li>
                                    </ul>
                                </section>
                            )}
                        </Show>
                        <Show when={r().remote.length === 0 && !r().recent_mismatch}>
                            <div class="warden-host-detail-hint">
                                No other instance claims this agent, and no jekt to it was recently refused.
                            </div>
                        </Show>
                    </>
                )}
            </Show>
        </div>
    );
};

HostRegistrationDetail.displayName = "HostRegistrationDetail";
