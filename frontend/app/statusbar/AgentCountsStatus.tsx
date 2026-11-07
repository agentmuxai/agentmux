// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { attentionItems, type AttentionItem } from "@/app/notification/attention-list";
import { runtimeCounts, type AgentRuntimeCounts } from "@/app/store/agent-runtime";
import { sharedAgentRuntime } from "@/app/store/agent-runtime-live";
import { openOrFocusPaneByView } from "@/app/store/block-component-registry";
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { createMemo, createSignal, Show, type Accessor, type JSX } from "solid-js";
import { AttentionListPopover } from "./AttentionListPopover";

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

export function agentCountsTip(c: AgentRuntimeCounts): string {
    const running = `${plural(c.running, "agent is", "agents are")} running`;
    return c.waiting > 0
        ? `${running}, ${c.waiting} waiting on you. Click to see who`
        : `${running}. Click to open the Swarm`;
}

/**
 * How many agents have a process, and how many of those wait on the user. Closing a pane no longer
 * stops an agent, so this is where a running one that costs shows. While anything waits, a click opens
 * the list of who (attention-list.ts) with a button to go to each; otherwise it opens the Swarm. It stays
 * as long as anything waits, so the toast that named it can go
 * (SPEC_ATTENTION_TOAST_WITH_LINK_TO_AGENT_2026_11_02 §3.6).
 */
export function AgentCountsStatus(props: {
    rows?: Accessor<AgentRuntime[]>;
    items?: Accessor<AttentionItem[]>;
}): JSX.Element {
    const rows = () => (props.rows ?? sharedAgentRuntime().rows)();
    const items = () => (props.items ?? attentionItems)();
    const counts = createMemo(() => {
        const c = runtimeCounts(rows());
        // The list names panes: one that only this window's registry knows (an ask the rows do not carry) counts too.
        return { running: c.running, waiting: Math.max(c.waiting, items().length) };
    });
    const [open, setOpen] = createSignal(false);
    let ref: HTMLDivElement | undefined;
    const openSwarm = () => void openOrFocusPaneByView("swarm");
    return (
        <Show when={counts().running > 0 || counts().waiting > 0}>
            <div
                ref={ref}
                class="status-bar-item clickable status-agent-counts"
                onClick={() => (counts().waiting > 0 ? setOpen(!open()) : openSwarm())}
                data-tip={agentCountsTip(counts())}
                aria-label={agentCountsTip(counts())}
                aria-haspopup={counts().waiting > 0 ? "dialog" : undefined}
                aria-expanded={counts().waiting > 0 ? open() : undefined}
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
            </div>
            <Show when={open() && counts().waiting > 0}>
                <AttentionListPopover anchor={ref} items={items} onClose={() => setOpen(false)} onOpenSwarm={openSwarm} />
            </Show>
        </Show>
    );
}

AgentCountsStatus.displayName = "AgentCountsStatus";
