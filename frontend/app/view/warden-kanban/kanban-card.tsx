// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// One task on the Kanban (SPEC_WARDEN_KANBAN_2026_10_20.md §6.7), and the dashed card of an agent
// working with no task (§6.6).

import { draggable } from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import { createMemo, onCleanup, onMount, Show, type JSX } from "solid-js";

import { kanbanCardItemType } from "@/app/drag/drag-types";
import { Button, Pill, StatusDot, type StatusTone } from "@/app/element/ui";
import { MOS } from "@/app/store/global";
import { formatAge } from "@/app/view/warden-shared/warden-shared";

import { checklistProgress, columnOf, needsReason, prLabel, requestedBy } from "./kanban-columns";
import type { KanbanAgent, KanbanAgentState, KanbanTask } from "./kanban-types";

export function agentTone(state: KanbanAgentState | undefined): StatusTone {
    switch (state) {
        case "working":
            return "busy";
        case "waiting":
            return "warn";
        case "error":
            return "error";
        case "idle":
            return "ok";
        default:
            return "idle";
    }
}

const STATE_WORDS: Record<KanbanAgentState, string> = {
    working: "working",
    waiting: "waiting on a question",
    idle: "idle",
    stopped: "stopped",
    error: "errored",
    gone: "closed",
};

export function agentStateWords(state: KanbanAgentState | undefined): string {
    return state ? STATE_WORDS[state] : "not here";
}

/** An agent's name with its live state. */
export function AgentChip(props: { name: string; agent?: KanbanAgent | null }): JSX.Element {
    return (
        <span class="kanban-agent-chip">
            <StatusDot
                tone={agentTone(props.agent?.state)}
                label={`${props.name}: ${agentStateWords(props.agent?.state)}`}
            />
            <span class="kanban-agent-chip__name">{props.name}</span>
        </span>
    );
}

export function KanbanCard(props: {
    task: KanbanTask;
    now: number;
    agents: KanbanAgent[];
    onOpen: (t: KanbanTask) => void;
}): JSX.Element {
    let el: HTMLDivElement | undefined;
    const t = () => props.task;
    const column = createMemo(() => columnOf(t()));
    const reason = createMemo(() => needsReason(t()));
    const who = createMemo(() => {
        const task = t();
        if (task.claimed_by) return { name: task.claimed_by, agent: task.holder };
        if (task.target_agent)
            return { name: task.target_agent, agent: props.agents.find((a) => a.name === task.target_agent) ?? null };
        return null;
    });
    onMount(() => {
        // Only a waiting task moves by drag: to another agent, to Anyone, or onto an agent to start it.
        if (!el || column() !== "backlog") return;
        const cleanup = draggable({
            element: el,
            getInitialData: () => ({ type: kanbanCardItemType, id: t().id, num: t().card.num }),
        });
        onCleanup(cleanup);
    });
    const open = () => props.onOpen(t());
    return (
        <div
            ref={el}
            class="kanban-card"
            classList={{
                "kanban-card--held": t().state === "held",
                "kanban-card--needs": column() === "needs",
            }}
            role="button"
            tabindex={0}
            aria-label={`Task ${t().card.num}: ${t().title}`}
            onClick={open}
            onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    open();
                }
            }}
        >
            <div class="kanban-card__title">
                <span class="kanban-card__num">#{t().card.num}</span>
                <span class="kanban-card__text">{t().title}</span>
            </div>
            <Show when={reason()}>{(r) => <div class="kanban-card__reason">{r()}</div>}</Show>
            <Show when={!reason() && t().card.note}>
                <div class="kanban-card__note">{t().card.note}</div>
            </Show>
            <div class="kanban-card__meta">
                <Show when={who()}>{(w) => <AgentChip name={w().name} agent={w().agent} />}</Show>
                <Show when={t().state === "held"}>
                    <Pill icon="pause">on hold</Pill>
                </Show>
                <Show when={checklistProgress(t())}>{(p) => <Pill icon="list-check">{p()}</Pill>}</Show>
                <Show when={t().card.pr_url}>
                    <Pill icon="code-pull-request" tone="accent" title={t().card.pr_url}>
                        {prLabel(t().card.pr_url)}
                    </Pill>
                </Show>
                <Show when={t().attempts > 1}>
                    <Pill title="How many times it has been taken">{`${t().attempts}/${t().max_attempts}`}</Pill>
                </Show>
                <Show when={t().card.repo}>
                    <span class="kanban-card__repo">{t().card.repo.replace(/^local:.*[\\/]/, "")}</span>
                </Show>
            </div>
            <div class="kanban-card__foot">
                <span>from {requestedBy(t())}</span>
                <span title="Since it last moved">{formatAge(Math.max(0, props.now - t().updated_at))}</span>
            </div>
        </div>
    );
}

/** An agent at work with no task (§6.6): its summary, and Track to make it one. */
export function UntrackedCard(props: {
    agent: KanbanAgent;
    onTrack: (a: KanbanAgent) => void;
    onOpenAgent: (a: KanbanAgent) => void;
}): JSX.Element {
    const meta = createMemo(() => MOS.getMuxObjectAtom<Block>(`block:${props.agent.block_id}`)()?.meta ?? {});
    const summary = createMemo(() => {
        const m = meta() as Record<string, unknown>;
        const pick = (k: string) => (typeof m[k] === "string" ? (m[k] as string).trim() : "");
        return pick("term:ambient_summary") || pick("term:last_prompt") || "working";
    });
    return (
        <div class="kanban-card kanban-card--untracked">
            <div class="kanban-card__title">
                <span class="kanban-card__text">{summary()}</span>
            </div>
            <div class="kanban-card__meta">
                <AgentChip name={props.agent.name} agent={props.agent} />
                <span class="kanban-card__untracked-label">untracked</span>
            </div>
            <div class="kanban-card__actions">
                <Button density="compact" icon="thumbtack" onClick={() => props.onTrack(props.agent)}>
                    Track
                </Button>
                <Button
                    density="compact"
                    tone="quiet"
                    icon="arrow-up-right-from-square"
                    onClick={() => props.onOpenAgent(props.agent)}
                >
                    Open
                </Button>
            </div>
        </div>
    );
}
