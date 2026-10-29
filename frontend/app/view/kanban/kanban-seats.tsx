// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Seats on the Kanban (docs/specs/SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md §4): a card's owner shows its
// seat on hover, and the board can show only the tasks of agents that report to the operator or to one
// lead. Seats come from `listagents`, a spawned worker's effective seat included; the board itself
// carries none.

import { createMemo, createSignal, onCleanup, onMount, Show, type Accessor, type JSX } from "solid-js";

import { Select, type SelectOption } from "@/app/element/ui";
import type { AgentDefinition } from "@/app/store/rpc-api";
import { leadName, seatsByAgent } from "@/app/view/agent/seat-format";
import { watchAgentList } from "@/app/view/agent/use-agent-list";
import type { SeatSummary } from "@/types/rpc/SeatSummary";

import type { KanbanTask } from "./kanban-types";

import "./kanban-seats.scss";

/** The seats the board can show: by agent uid, and each agent's uid by its name for a task that names no uid. */
export interface KanbanSeatIndex {
    byUid: ReadonlyMap<string, SeatSummary>;
    uidByName: ReadonlyMap<string, string>;
}

const EMPTY: KanbanSeatIndex = { byUid: new Map(), uidByName: new Map() };

/** The index of `listagents`' agents, templates left out. */
export function seatIndex(
    defs: readonly Pick<AgentDefinition, "id" | "name" | "is_seeded" | "seat">[]
): KanbanSeatIndex {
    const agents = defs.filter((d) => !d.is_seeded);
    return {
        byUid: seatsByAgent(agents),
        uidByName: new Map(agents.map((d) => [d.name.trim().toLowerCase(), d.id])),
    };
}

// One list for the whole board, however many chips ask: loaded while any of them is mounted.
const [sharedIndex, setSharedIndex] = createSignal<KanbanSeatIndex>(EMPTY);
let watchers = 0;
let stopWatching: (() => void) | null = null;

/** This machine's agents' seats, kept current with `agents:changed`. */
export function useSeatIndex(): Accessor<KanbanSeatIndex> {
    onMount(() => {
        if (watchers++ === 0) stopWatching = watchAgentList((list) => setSharedIndex(seatIndex(list)));
        onCleanup(() => {
            if (--watchers > 0) return;
            stopWatching?.();
            stopWatching = null;
        });
    });
    return sharedIndex;
}

/** An agent's seat, by uid, or by name when the task names no uid; null when it has none. */
export function seatOf(
    index: KanbanSeatIndex,
    uid: string | null | undefined,
    name: string | null | undefined
): SeatSummary | null {
    const id = uid || (name ? index.uidByName.get(name.trim().toLowerCase()) : undefined);
    return (id && index.byUid.get(id)) || null;
}

/** Who a card names as its owner: the agent holding it, or the one it waits for. The same rule as its chip. */
function ownerOf(t: Pick<KanbanTask, "claimed_by" | "claimed_by_uid" | "target_agent" | "target_agent_uid">): {
    name: string;
    uid: string;
} | null {
    if (t.claimed_by) return { name: t.claimed_by, uid: t.claimed_by_uid };
    if (t.target_agent) return { name: t.target_agent, uid: t.target_agent_uid };
    return null;
}

/** The "Reports to" filter's value for the operator; "" is anyone, and "lead:<uid>" one lead. */
export const REPORTS_TO_OPERATOR = "operator";

/** Whether an agent with `seat` reports where `filter` says. An agent with no seat reports nowhere. */
export function reportsTo(seat: SeatSummary | null, filter: string): boolean {
    if (!filter) return true;
    if (!seat) return false;
    if (filter === REPORTS_TO_OPERATOR) return seat.lead == null;
    return seat.lead != null && filter === `lead:${seat.lead.id}`;
}

/** Whether a task's owner reports where `filter` says; a task for no one matches only "anyone". */
export function taskReportsTo(index: KanbanSeatIndex, t: Parameters<typeof ownerOf>[0], filter: string): boolean {
    if (!filter) return true;
    const owner = ownerOf(t);
    return !!owner && reportsTo(seatOf(index, owner.uid, owner.name), filter);
}

/** The choices: anyone, the operator when someone reports to it, then each lead someone reports to, by name. */
export function reportsToOptions(index: KanbanSeatIndex): SelectOption[] {
    const seats = [...index.byUid.values()];
    const leads = new Map<string, string>();
    for (const s of seats) if (s.lead) leads.set(s.lead.id, leadName(s.lead));
    const options: SelectOption[] = [{ value: "", label: "Reports to anyone" }];
    if (seats.some((s) => s.lead == null))
        options.push({ value: REPORTS_TO_OPERATOR, label: "Reports to the operator" });
    const byName = [...leads.entries()].sort((a, b) => a[1].localeCompare(b[1], undefined, { sensitivity: "base" }));
    for (const [id, name] of byName) options.push({ value: `lead:${id}`, label: `Reports to ${name}` });
    return options;
}

/** The toolbar's filter: shown once an agent has a seat, or while it is set, so it can be cleared. */
export function ReportsToSelect(props: { value: string; onChange: (v: string) => void }): JSX.Element {
    const index = useSeatIndex();
    const options = createMemo(() => {
        const list = reportsToOptions(index());
        // A lead no one reports to any more stays listed while it is chosen.
        if (props.value && !list.some((o) => o.value === props.value)) {
            list.push({ value: props.value, label: "Reports to an agent no longer leading" });
        }
        return list;
    });
    return (
        <Show when={index().byUid.size > 0 || props.value}>
            <Select
                density="compact"
                ariaLabel="Reports to"
                class="kanban-toolbar__reports"
                value={props.value}
                onChange={props.onChange}
                options={options()}
            />
        </Show>
    );
}
