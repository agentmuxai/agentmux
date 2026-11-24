// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "quiet 22 min" on a Swarm row: the agent holds work and has done nothing for the quiet time
 * (docs/specs/SPEC_LEADER_TOLD_WHEN_AN_AGENT_GOES_QUIET_2026_11_21.md §6). Information only, muted, from the
 * runtime row's `quiet_since_ms` (srv's last quiet sweep), so the row's own state (idle, working) is unchanged and
 * the Kanban card reads the same fact.
 */

import { tip } from "@/app/element/tip";
import { t as tr } from "@/app/i18n/i18n";
import { sharedAgentRuntime } from "@/app/store/agent-runtime-live";
import { Show, type Accessor, type JSX } from "solid-js";
import { agentUidOfBlockMeta } from "./swarm-open-parked";

/** Whole minutes quiet at `now`, never under one: the note is shown only for an agent that is already quiet. */
export function quietMinutes(sinceMs: number, now: number): number {
    return Math.max(1, Math.floor((now - sinceMs) / 60_000));
}

/** When the agent in the pane with `meta` went quiet, or null: it is not quiet, has no pane, or there is no store. */
export function quietSinceForRow(meta: Record<string, unknown> | undefined): number | null {
    const agentId = agentUidOfBlockMeta(meta);
    if (!agentId) return null;
    try {
        return sharedAgentRuntime().rowOf(agentId)?.quiet_since_ms ?? null;
    } catch {
        return null;
    }
}

export function QuietNote(props: { meta: Record<string, unknown> | undefined; now: Accessor<number> }): JSX.Element {
    const since = () => quietSinceForRow(props.meta);
    return (
        <Show when={since()}>
            {(at) => {
                const minutes = () => quietMinutes(at(), props.now());
                return (
                    <span class="swarm-quiet" {...tip(tr("swarm.quiet.title", { minutes: minutes() }))}>
                        {tr("swarm.quiet.label", { minutes: minutes() })}
                    </span>
                );
            }}
        </Show>
    );
}
