// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Seats in the Swarm (docs/specs/SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md §4):
 * the role badge after an agent's name, the "Group by" control, and the list
 * of rows grouped by lead (swarm-seat-groups.ts).
 */

import { createMemo, For, Show, untrack, type Accessor, type JSX } from "solid-js";

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { Pill, SegmentedControl } from "@/app/element/ui";
import { MOS } from "@/app/store/global";
import { readSeatMeta, seatBadgeText, seatTooltipLine } from "@/app/view/agent/seat-format";
import type { SeatSummary } from "@/types/rpc/SeatSummary";
import type { RemoteRow } from "./swarm-remote";
import {
    GROUP_BY_META_KEY,
    groupRowsByLead,
    readGroupBy,
    remoteSeatInfo,
    type SeatListEntry,
    type SeatRowInfo,
    type SwarmGroupBy,
} from "./swarm-seat-groups";

import "./swarm-seats.scss";

/** The role badge, from the pane's `agent:seat`; nothing for an agent with no seat. */
export function SwarmSeatBadge(props: { meta: Record<string, unknown> | undefined }): JSX.Element {
    return <SwarmSeatPill seat={readSeatMeta(props.meta)} />;
}

/** The role badge for a seat in hand: an other instance's agent shows the seat it reports. */
export function SwarmSeatPill(props: { seat: SeatSummary | null | undefined }): JSX.Element {
    return (
        <Show when={props.seat}>
            {(s) => (
                <Pill class="swarm-seat-badge" title={seatTooltipLine(s())}>
                    {seatBadgeText(s())}
                </Pill>
            )}
        </Show>
    );
}

const GROUP_BY_OPTIONS: { value: SwarmGroupBy; label: string; title: string }[] = [
    { value: "none", label: "None", title: "One list: working agents first, then by last activity" },
    { value: "lead", label: "Lead", title: "Under the operator and each lead, by whom each agent reports to" },
];

/**
 * "Group by: None / Lead", kept in the pane's meta. Shown once an agent here has
 * a seat, or while the list is grouped, so it can be turned off.
 */
export function SwarmGroupByControl(props: { seats: SwarmSeats }): JSX.Element {
    const shown = () => props.seats.groupBy() === "lead" || [...props.seats.info().values()].some((i) => i.seat);
    return (
        <Show when={shown()}>
            <div class="swarm-group-by">
                <span class="swarm-group-by__label">Group by</span>
                <SegmentedControl<SwarmGroupBy>
                    density="compact"
                    ariaLabel="Group by"
                    options={GROUP_BY_OPTIONS}
                    value={props.seats.groupBy()}
                    onChange={props.seats.setGroupBy}
                />
            </div>
        </Show>
    );
}

/**
 * The Swarm's rows, one list or grouped by lead. Rows stay direct children of
 * the list element, which is how the slide finds them (swarm-order-motion.ts);
 * a row one step in follows a marker the stylesheet indents it by.
 */
export function SwarmSeatList(props: {
    seats: SwarmSeats;
    order: Accessor<string[]>;
    children: (key: string) => JSX.Element;
}): JSX.Element {
    const grouped = () => props.seats.groupBy() === "lead";
    const entries = createMemo(() => (grouped() ? groupRowsByLead(props.order(), props.seats.info()) : []));
    // Keyed by string so a regroup moves the rows instead of building them again.
    const byId = createMemo(() => new Map(entries().map((e) => [entryId(e), e] as const)));
    return (
        <Show when={grouped()} fallback={<For each={props.order()}>{(key) => props.children(key)}</For>}>
            <For each={entries().map(entryId)}>
                {(id) => {
                    const entry = () => byId().get(id);
                    return (
                        <Show
                            when={id.startsWith("h:")}
                            fallback={
                                <>
                                    <Show when={depthOf(entry()) > 0}>
                                        <div
                                            class={`swarm-seat-indent swarm-seat-indent--d${depthOf(entry())}`}
                                            aria-hidden="true"
                                        />
                                    </Show>
                                    {/* Built once: the row follows its own data from here. */}
                                    {untrack(() => props.children(id.slice(2)))}
                                </>
                            }
                        >
                            <Show when={entry()?.kind === "heading" ? (entry() as HeadingEntry) : null}>
                                {(h) => (
                                    <div class="swarm-seat-heading" role="heading" aria-level={3} title={h().title}>
                                        <span class="swarm-seat-heading__label">{h().label}</span>
                                        <span class="swarm-seat-heading__count">{h().count}</span>
                                    </div>
                                )}
                            </Show>
                        </Show>
                    );
                }}
            </For>
        </Show>
    );
}

type HeadingEntry = Extract<SeatListEntry, { kind: "heading" }>;

function entryId(e: SeatListEntry): string {
    return e.kind === "heading" ? `h:${e.id}` : `r:${e.key}`;
}

function depthOf(e: SeatListEntry | undefined): number {
    return e?.kind === "row" ? e.depth : 0;
}

/** A Swarm pane's grouping: the choice, kept in its meta, and what grouping needs to know about each row. */
export interface SwarmSeats {
    groupBy: Accessor<SwarmGroupBy>;
    setGroupBy: (v: SwarmGroupBy) => void;
    info: Accessor<ReadonlyMap<string, SeatRowInfo>>;
}

/** `rows` are this instance's rows by key; `remote` the other instances', with the seats they report. */
export function useSwarmSeats(
    ctx: Pick<PaneTabHostContext, "meta" | "setMeta">,
    rows: Accessor<readonly (readonly [string, { blockId: string | null }])[]>,
    remote: Accessor<ReadonlyMap<string, RemoteRow>>
): SwarmSeats {
    return {
        groupBy: createMemo(() => readGroupBy(ctx.meta())),
        setGroupBy: (v) => void ctx.setMeta({ [GROUP_BY_META_KEY]: v === "lead" ? "lead" : null }),
        info: createMemo(() => seatRowInfo(rows(), remote())),
    };
}

/** This instance's rows from their panes' meta, the others' as remote. */
function seatRowInfo(
    rows: readonly (readonly [string, { blockId: string | null }])[],
    remote: ReadonlyMap<string, RemoteRow>
): Map<string, SeatRowInfo> {
    const info = new Map<string, SeatRowInfo>();
    for (const [key, node] of rows) {
        const meta = node.blockId
            ? MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", node.blockId))()?.meta
            : undefined;
        const agentId = typeof meta?.["agentId"] === "string" && meta["agentId"] ? (meta["agentId"] as string) : null;
        info.set(key, { agentId, seat: readSeatMeta(meta), remote: false });
    }
    for (const [key, row] of remote) info.set(key, remoteSeatInfo(row));
    return info;
}
