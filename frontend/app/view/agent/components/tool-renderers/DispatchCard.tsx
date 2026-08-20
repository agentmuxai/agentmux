// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * DispatchCard — renders an Agent/Task/Workflow tool call as a compact card
 * showing the live dispatch state (name, running/done, member count for a
 * Workflow) that the Swarm panel already tracks for the same call, instead
 * of the generic CompactResult JSON dump.
 *
 * The dispatch is supplied via `ToolRenderContext.dispatchMatch` — an
 * ordinal match (see `activity/dispatch-correlation.ts`), not an exact id
 * link. When no confident match was found, falls back to `CompactResult`,
 * unchanged from today's behavior.
 */

import { Show, type JSX } from "solid-js";
import { createBlock } from "@/app/store/global";
import type { AgentDispatch } from "../../../swarm/swarm-model";
import type { ToolNode } from "../../types";
import { CompactResult } from "../CompactResult";
import { byKind, registerToolRenderer, type ToolRenderContext } from "./registry";

function openSwarmPane(): void {
    createBlock({ meta: { view: "swarm" } });
}

function DispatchCardView(props: { dispatch: AgentDispatch }): JSX.Element {
    const isWorkflow = () => props.dispatch.kind === "workflow";
    const isRunning = () => props.dispatch.status === "running";
    const name = () => props.dispatch.dispatch_name ?? (isWorkflow() ? "Workflow" : "Subagent");

    return (
        <div
            class="agent-dispatch-card"
            classList={{ "agent-dispatch-card-running": isRunning(), "agent-dispatch-card-done": !isRunning() }}
            role="button"
            tabindex="0"
            onClick={openSwarmPane}
            onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    openSwarmPane();
                }
            }}
        >
            <span class="agent-dispatch-card-status-icon">{isRunning() ? "⏳" : "✓"}</span>
            <span class="agent-dispatch-card-name">{name()}</span>
            <Show when={isWorkflow()}>
                <span class="agent-dispatch-card-members">
                    {props.dispatch.members_done}/{props.dispatch.member_count} done
                </span>
            </Show>
            <span class="agent-dispatch-card-view-link">View in Swarm →</span>
        </div>
    );
}

export function DispatchCard(props: { node: ToolNode; ctx?: ToolRenderContext }): JSX.Element {
    return (
        <Show
            when={props.ctx?.dispatchMatch}
            fallback={
                <CompactResult tool={props.node.tool} params={props.node.params as any} result={props.node.result} />
            }
        >
            {(d) => <DispatchCardView dispatch={d()} />}
        </Show>
    );
}

DispatchCard.displayName = "DispatchCard";

// Registered above the priority-0 Agent/Task/Workflow built-ins in
// ToolOverlayLog.tsx — wins whenever a confident dispatch match exists;
// its own internal fallback covers the no-match case, so the priority-0
// builtins remain reachable only as defense-in-depth if this entry were
// ever removed.
registerToolRenderer({
    priority: 10,
    label: "dispatch:card",
    match: byKind("Agent", "Task", "Workflow"),
    render: (node, ctx) => <DispatchCard node={node} ctx={ctx} />,
});
