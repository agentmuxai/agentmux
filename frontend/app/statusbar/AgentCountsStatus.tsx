// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { attentionItems, sharedAttentionRows, type AttentionItem } from "@/app/notification/attention-list";
import { runtimeCounts, type AgentRuntimeCounts } from "@/app/store/agent-runtime";
import { openOrFocusPaneByView } from "@/app/store/block-component-registry";
import { useKanbanNeeds } from "@/app/view/kanban/kanban-held";
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { createEffect, createMemo, createSignal, onCleanup, Show, type Accessor, type JSX } from "solid-js";
import { AttentionListPopover } from "./AttentionListPopover";

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

export function agentCountsTip(c: AgentRuntimeCounts, tasks = 0): string {
    const running = `${plural(c.running, "agent is", "agents are")} running`;
    const kanban = tasks > 0 ? `${plural(tasks, "Kanban task needs", "Kanban tasks need")} you` : "";
    if (c.waiting > 0) {
        return `${running}, ${c.waiting} waiting on you${kanban ? `, ${kanban}` : ""}. Click to see who`;
    }
    if (tasks > 0) return `${running}, ${kanban}. Click to see which`;
    return `${running}. Click to open the Swarm`;
}

/**
 * How many agents have a process, and how many of those wait on the user. Closing a pane no longer
 * stops an agent, so this is where a running one that costs shows. While anything waits, a click opens
 * the list of who (attention-list.ts) with a button to go to each; otherwise it opens the Swarm. It stays
 * as long as anything waits, so the toast that named it can go
 * (SPEC_ATTENTION_TOAST_WITH_LINK_TO_AGENT_2026_11_02 §3.6). The Kanban's Needs-you tasks are in it too, as a
 * group of their own: they make no sound and no toast, so the count and the list are where they show (§2, D8).
 */
export function AgentCountsStatus(props: {
    rows?: Accessor<AgentRuntime[]>;
    items?: Accessor<AttentionItem[]>;
    /** How many Kanban tasks need the person (the Kanban pane's own count); default is the live one. */
    tasks?: Accessor<number>;
}): JSX.Element {
    const rows = () => (props.rows ?? sharedAttentionRows)();
    const items = () => (props.items ?? attentionItems)();
    const tasks = props.tasks ?? useKanbanNeeds();
    const counts = createMemo(() => {
        const c = runtimeCounts(rows());
        // The list names panes: one that only this window's registry knows (an ask the rows do not carry) counts too.
        return { running: c.running, waiting: Math.max(c.waiting, items().length) };
    });
    // Anything that waits on the person: an agent's ask, or a Kanban task.
    const anything = () => counts().waiting + tasks() > 0;
    const [open, setOpen] = createSignal(false);
    // The list is for what waits: with nothing left it is closed for good, so the next request does not open it unasked.
    createEffect(() => {
        if (!anything()) setOpen(false);
    });
    let ref: HTMLDivElement | undefined;
    // A narrow bar can hide the entry while its list is open; the list has no anchor then and would jump to a corner.
    const watchAnchor = (el: HTMLDivElement) => {
        ref = el;
        if (typeof ResizeObserver === "undefined") return;
        const watch = new ResizeObserver(() => {
            if (el.getClientRects().length === 0) setOpen(false);
        });
        watch.observe(el);
        onCleanup(() => watch.disconnect());
    };
    const openSwarm = () => void openOrFocusPaneByView("swarm");
    const openKanban = () => void openOrFocusPaneByView("kanban");
    const tip = () => agentCountsTip(counts(), tasks());
    return (
        <Show when={counts().running > 0 || anything()}>
            <div
                ref={watchAnchor}
                class="status-bar-item clickable status-agent-counts"
                onClick={() => (anything() ? setOpen(!open()) : openSwarm())}
                data-tip={tip()}
                aria-label={tip()}
                aria-haspopup={anything() ? "dialog" : undefined}
                aria-expanded={anything() ? open() : undefined}
                role="button"
            >
                <i class="fa-solid fa-robot status-icon" />
                <Show when={counts().running > 0}>
                    <span class="status-agent-counts__running">{counts().running} running</span>
                </Show>
                <Show when={counts().waiting > 0}>
                    <span class="status-agent-counts__waiting" style={{ color: "var(--warning-text-color)" }}>
                        {counts().waiting} waiting on you
                    </span>
                </Show>
                <Show when={tasks() > 0}>
                    <span class="status-agent-counts__tasks" style={{ color: "var(--warning-text-color)" }}>
                        {counts().waiting > 0 ? `· ${plural(tasks(), "task", "tasks")}` : `${plural(tasks(), "task needs", "tasks need")} you`}
                    </span>
                </Show>
            </div>
            <Show when={open() && anything()}>
                <AttentionListPopover
                    anchor={ref}
                    items={items}
                    tasks={tasks}
                    onClose={() => setOpen(false)}
                    onOpenSwarm={openSwarm}
                    onOpenKanban={openKanban}
                />
            </Show>
        </Show>
    );
}

AgentCountsStatus.displayName = "AgentCountsStatus";
