// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Swarm's agents that have no row of their own: one row each, from the agent's definition
// (name, provider) and its runtime state (store/agent-runtime.ts). They are listed apart from the live
// agents' rows: those are keyed by block and carry a Supervisor, a selection and a stream of
// activity, none of which these have. Two groups: the ones that run with no pane (started with no
// pane, or whose block the Swarm does not list), and the stopped ones. Start runs a stopped one with no
// pane and Open starts it in one; a running one only needs Open.

import { Button } from "@/app/element/ui/Button";
import { MOS } from "@/app/store/global";
import { hasProcess } from "@/app/store/agent-runtime";
import { AgentRuntimeChip } from "@/app/view/agent/components/AgentRuntimeChip";
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { createMemo, createSignal, For, Show, type Accessor, type JSX } from "solid-js";
import { LOCAL_PROVIDER_UNKNOWN, SwarmProviderIcon } from "./swarm-provider-icon";
import { HeldNote } from "./swarm-row-notes";
import { tip } from "@/app/element/tip";

/** The meta of the block a stopped agent keeps its desired state and held input in, once it has one. */
function blockMeta(blockId: string | undefined): Record<string, any> | undefined {
    return blockId ? MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", blockId))()?.meta : undefined;
}

/** An agent that has a process (or a limit or sign-in problem that stops its turns, not its life): it runs, with no
 *  pane the Swarm lists it by. The same states the status bar counts as running (store/agent-runtime.ts). */
export function runsWithoutPane(agent: Pick<AgentRuntime, "state">): boolean {
    return hasProcess(agent.state);
}

type Action = (a: AgentRuntime) => Promise<string | undefined> | void;

/** What a pressed Start or Open is doing: the agents with one out, and why srv refused one. */
function createRowRunner() {
    const [refused, setRefused] = createSignal<Record<string, string>>({});
    // The agents with a start or an open out: the row says so, and a second press waits for the first.
    const [busy, setBusy] = createSignal<ReadonlySet<string>>(new Set());
    const run = async (agent: AgentRuntime, action: Action) => {
        if (busy().has(agent.agent_id)) return;
        setRefused((prev) => ({ ...prev, [agent.agent_id]: "" }));
        setBusy((prev) => new Set(prev).add(agent.agent_id));
        try {
            const why = await action(agent);
            if (why) setRefused((prev) => ({ ...prev, [agent.agent_id]: why }));
        } finally {
            setBusy((prev) => {
                const next = new Set(prev);
                next.delete(agent.agent_id);
                return next;
            });
        }
    };
    return { refused, busy, run };
}

type Runner = ReturnType<typeof createRowRunner>;

/** One agent's row: a click on it is its Open button (a press on a control inside it is that control's own). */
function NoPaneRow(props: { agent: AgentRuntime; runner: Runner; onOpen: Action; onStart: Action }): JSX.Element {
    const runner = props.runner;
    const agent = () => props.agent;
    return (
        <>
            <div
                class="swarm-stopped__row"
                data-agent-id={agent().agent_id}
                role="button"
                tabIndex={0}
                onClick={(e) => {
                    if ((e.target as Element).closest("button")) return;
                    void runner.run(agent(), props.onOpen);
                }}
                onKeyDown={(e) => {
                    if (e.target !== e.currentTarget || (e.key !== "Enter" && e.key !== " ")) return;
                    e.preventDefault();
                    void runner.run(agent(), props.onOpen);
                }}
            >
                <SwarmProviderIcon provider={agent().provider} unknownTitle={LOCAL_PROVIDER_UNKNOWN} />
                <span class="swarm-stopped__name" {...tip(`${agent().name}: click to open`)}>
                    {agent().name}
                </span>
                <HeldNote meta={blockMeta(agent().block_id)} />
                <AgentRuntimeChip state={agent().state} detail={agent().detail} blocked={agent().blocked} />
                <Show when={!runsWithoutPane(agent())}>
                    <Button
                        density="compact"
                        aria-label={`Start ${agent().name}`}
                        title="Run it with no pane, resuming its session"
                        disabled={runner.busy().has(agent().agent_id)}
                        onClick={() => void runner.run(agent(), props.onStart)}
                    >
                        {runner.busy().has(agent().agent_id) ? "Starting…" : "Start"}
                    </Button>
                </Show>
                <Button
                    density="compact"
                    aria-label={`Open ${agent().name}`}
                    title={runsWithoutPane(agent()) ? "Show it in a pane" : "Open it in a pane (starts it)"}
                    disabled={runner.busy().has(agent().agent_id)}
                    onClick={() => void runner.run(agent(), props.onOpen)}
                >
                    Open
                </Button>
            </div>
            <Show when={runner.refused()[agent().agent_id]}>
                {(why) => (
                    <div class="swarm-stopped__error" role="alert">
                        {why()}
                    </div>
                )}
            </Show>
        </>
    );
}

export function StoppedAgents(props: {
    /** The agents with no row of their own: the ones that run with no pane, and the stopped ones. */
    agents: Accessor<AgentRuntime[]>;
    /** Whether live agents are listed above: the stopped list starts folded then, open when it is all there is. */
    hasLive: Accessor<boolean>;
    /** Resolves to why srv refused, shown under the row. */
    onOpen: Action;
    /** Runs the agent with no pane; resolves to why srv refused, shown under the row. */
    onStart: Action;
}): JSX.Element {
    const [folded, setFolded] = createSignal<boolean | null>(null);
    const runner = createRowRunner();
    const running = createMemo(() => props.agents().filter(runsWithoutPane));
    const stopped = createMemo(() => props.agents().filter((a) => !runsWithoutPane(a)));
    const isFolded = () => folded() ?? (props.hasLive() || running().length > 0);
    return (
        <>
            <Show when={running().length > 0}>
                <section class="swarm-stopped swarm-nopane" aria-label="Running with no pane">
                    <div class="swarm-stopped__heading swarm-nopane__heading">
                        Running with no pane
                        <span class="swarm-stopped__count">{running().length}</span>
                    </div>
                    <For each={running()}>
                        {(agent) => <NoPaneRow agent={agent} runner={runner} onOpen={props.onOpen} onStart={props.onStart} />}
                    </For>
                </section>
            </Show>
            <Show when={stopped().length > 0}>
                <section class="swarm-stopped" aria-label="Stopped agents">
                    <Button
                        tone="quiet"
                        density="compact"
                        class="swarm-stopped__heading"
                        icon={isFolded() ? "chevron-right" : "chevron-down"}
                        aria-expanded={!isFolded()}
                        onClick={() => setFolded(!isFolded())}
                    >
                        Stopped agents
                        <span class="swarm-stopped__count">{stopped().length}</span>
                    </Button>
                    <Show when={!isFolded()}>
                        <For each={stopped()}>
                            {(agent) => <NoPaneRow agent={agent} runner={runner} onOpen={props.onOpen} onStart={props.onStart} />}
                        </For>
                    </Show>
                </section>
            </Show>
        </>
    );
}
