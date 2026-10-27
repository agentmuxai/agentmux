// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// One task on the Kanban (SPEC_WARDEN_KANBAN_2026_10_20.md §6.7), and the dashed card of an agent
// working with no task (§6.6).

import { draggable } from "@atlaskit/pragmatic-drag-and-drop/element/adapter";
import { createMemo, For, onCleanup, onMount, Show, type JSX } from "solid-js";

import { kanbanCardItemType } from "@/app/drag/drag-types";
import { Button, Pill, StatusDot, type StatusTone } from "@/app/element/ui";
import { MOS } from "@/app/store/global";
import { seatTooltipLine } from "@/app/view/agent/seat-format";
import { formatAge } from "@/app/view/warden-shared/warden-shared";

import { agentColor } from "./kanban-colors";
import {
    checklistProgress,
    columnOf,
    isResearchKind,
    needsReason,
    prLabel,
    requestedBy,
    stageWord,
    waitingOn,
} from "./kanban-columns";
import { CardMedia } from "./kanban-media";
import { seatOf, useSeatIndex } from "./kanban-seats";
import type { KanbanAfter, KanbanAgent, KanbanAgentState, KanbanTask } from "./kanban-types";

/** A linked task's state, as a card's pill says it. */
const LINK_WORDS: Record<string, string> = {
    open: "waiting",
    held: "on hold",
    claimed: "in progress",
    blocked: "needs you",
    review: "in review",
    done: "accepted",
    failed: "failed",
    cancelled: "cancelled",
};

function linkWord(a: KanbanAfter): string {
    return LINK_WORDS[a.state] ?? a.state;
}

export function agentTone(state: KanbanAgentState | undefined): StatusTone {
    switch (state) {
        case "working":
            return "busy";
        case "waiting":
        case "limited":
            return "warn";
        case "error":
        case "signed_out":
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
    limited: "out of quota",
    signed_out: "signed out",
    gone: "closed",
};

export function agentStateWords(state: KanbanAgentState | undefined): string {
    // A state srv added that this build doesn't word yet reads as itself.
    return state ? (STATE_WORDS[state] ?? state) : "not here";
}

/** An agent's name with its live state, the name in its colour when it has one. */
export function AgentChip(props: {
    name: string;
    agent?: KanbanAgent | null;
    uid?: string;
    colors?: Record<string, string>;
    /** Its seat as the tooltip: a card's owner (SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md §4). */
    showSeat?: boolean;
}): JSX.Element {
    // The theme's readable variant of its hue, the one made for text and icons.
    const nameColor = () => agentColor(props.agent, props.uid, props.colors, "identity");
    const seats = useSeatIndex();
    const seat = () => (props.showSeat ? seatOf(seats(), props.uid || props.agent?.uid, props.name) : null);
    return (
        <span class="kanban-agent-chip" title={seat() ? `${props.name}: ${seatTooltipLine(seat()!)}` : undefined}>
            <StatusDot
                tone={agentTone(props.agent?.state)}
                label={`${props.name}: ${agentStateWords(props.agent?.state)}`}
            />
            <span class="kanban-agent-chip__name" style={nameColor() ? { "--kanban-agent-text": nameColor() } : undefined}>
                {props.name}
            </span>
        </span>
    );
}

export function KanbanCard(props: {
    task: KanbanTask;
    now: number;
    agents: KanbanAgent[];
    /** The colours of agents not running here, by uid (`agent_colors`). */
    colors?: Record<string, string>;
    onOpen: (t: KanbanTask) => void;
}): JSX.Element {
    let el: HTMLDivElement | undefined;
    const t = () => props.task;
    const column = createMemo(() => columnOf(t()));
    const reason = createMemo(() => needsReason(t()));
    const who = createMemo(() => {
        const task = t();
        if (task.claimed_by) return { name: task.claimed_by, agent: task.holder, uid: task.claimed_by_uid };
        if (task.target_agent)
            return {
                name: task.target_agent,
                agent: props.agents.find((a) => a.name === task.target_agent) ?? null,
                uid: task.target_agent_uid,
            };
        return null;
    });
    // The card's left edge is its agent's colour; a task for no one keeps the plain border.
    const stripe = createMemo(() => {
        const w = who();
        return w ? agentColor(w.agent, w.uid, props.colors) : undefined;
    });
    onMount(() => {
        // Only a waiting task moves by drag: to another agent, to Anyone, or onto an agent to start it.
        if (!el || (column() !== "backlog" && column() !== "specd")) return;
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
                "kanban-card--cancelled": t().state === "cancelled",
            }}
            style={stripe() ? { "--kanban-agent-color": stripe() } : undefined}
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
            <Show when={reason()}>
                {(r) => (
                    <div class="kanban-card__reason">
                        <i class="fa-solid fa-circle-exclamation" aria-hidden="true" />
                        {r()}
                    </div>
                )}
            </Show>
            <Show when={!reason() && t().card.note}>
                <div class="kanban-card__note">{t().card.note}</div>
            </Show>
            <CardMedia task={t()} />
            <div class="kanban-card__meta">
                <Show when={who()}>
                    {(w) => (
                        <AgentChip name={w().name} agent={w().agent} uid={w().uid} colors={props.colors} showSeat />
                    )}
                </Show>
                <Show when={t().state === "held"}>
                    <Pill icon="pause">on hold</Pill>
                </Show>
                {/* Its spec: "spec #1103 · in review", then "· accepted" once it is Spec'd. */}
                <For each={(t().card.after ?? []).filter((a) => isResearchKind(a.kind))}>
                    {(a) => (
                        <Pill icon="file-lines" title={`${a.kind} #${a.num} ${a.title}`}>
                            {`${a.kind?.trim().toLowerCase()} #${a.num} · ${linkWord(a)}`}
                        </Pill>
                    )}
                </For>
                <Show
                    when={
                        (column() === "backlog" || column() === "specd") &&
                        waitingOn(t()).filter((a) => !isResearchKind(a.kind)).length > 0
                    }
                >
                    <Pill
                        icon="hourglass-half"
                        title={`Starts after ${waitingOn(t())
                            .filter((a) => !isResearchKind(a.kind))
                            .map((a) => `#${a.num} ${a.title}`)
                            .join(", ")}: nobody takes it until then`}
                    >
                        {`after #${waitingOn(t())
                            .filter((a) => !isResearchKind(a.kind))
                            .map((a) => a.num)
                            .join(", #")}`}
                    </Pill>
                </Show>
                {/* A spec's build: what accepting it unlocks. */}
                <Show when={isResearchKind(t().kind)}>
                    <For each={t().card.followers ?? []}>
                        {(f) => (
                            <Pill icon="arrow-right" title={`#${f.num} ${f.title}: ${linkWord(f)}`}>
                                {`build #${f.num}`}
                            </Pill>
                        )}
                    </For>
                </Show>
                <Show when={column() === "needs" || column() === "review"}>
                    <Pill title="What this task is">{stageWord(t())}</Pill>
                </Show>
                <Show when={t().card.pr?.released_in}>
                    {(tag) => (
                        <Pill icon="box" tone="ok" title="Shipped in this release">
                            {tag()}
                        </Pill>
                    )}
                </Show>
                <Show when={column() === "merged" && t().card.pr?.merged_at}>
                    <Pill icon="code-merge" title="Its PR merged">
                        merged
                    </Pill>
                </Show>
                <Show when={t().state === "cancelled"}>
                    <Pill icon="ban">cancelled</Pill>
                </Show>
                <Show when={t().accepted_by}>
                    {(a) => (
                        <Pill icon="user-check" title={`Accepted by ${a().name}.${a().said ? ` ${a().said}` : ""}`}>
                            {`accepted by ${a().name}`}
                        </Pill>
                    )}
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
