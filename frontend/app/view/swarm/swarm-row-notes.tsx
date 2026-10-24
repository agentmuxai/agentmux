// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Two small notes on a Swarm agent row that say why an agent is quiet
 * (docs/specs/SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md §4.4):
 *
 * - `UnavailableChip`: the agent can't start a turn, because its plan is out of quota until a
 *   reset or its sign-in was refused. It takes the place of the status chip, whose green "idle"
 *   would read as an agent that is simply resting.
 * - `LongToolNote`: a tool call has been running for minutes. Information only: nothing is
 *   killed or flagged as failed (the August removal of hung-turn detection stands,
 *   SPEC_REMOVE_AGENT_UNRESPONSIVE_DETECTION_2026_08_25.md).
 */

import { unavailableChipText, unavailableTitle, type AgentUnavailable } from "@/app/store/agent-unavailable";
import { Show, type Accessor, type JSX } from "solid-js";
import type { LongRunningToolRow } from "./swarm-longrunning";

/** The chip for an agent that can't start a turn, with the reset time when it is known. */
export function UnavailableChip(props: { unavailable: AgentUnavailable; now?: Accessor<number> }): JSX.Element {
    const now = () => props.now?.() ?? Date.now();
    return (
        <span
            class={`swarm-unavailable-chip swarm-unavailable-chip--${props.unavailable.reason}`}
            title={unavailableTitle(props.unavailable)}
        >
            <i class="fa-solid fa-circle-exclamation swarm-unavailable-icon" />
            <span class="swarm-unavailable-label">{unavailableChipText(props.unavailable, now())}</span>
        </span>
    );
}

/** The longest a command is shown in the note before it is cut. */
const MAX_COMMAND_CHARS = 28;

/** Whole minutes a call has run, or 0 under a minute. */
export function minutesRunning(startedAt: number, now: number): number {
    return Math.max(0, Math.floor((now - startedAt) / 60_000));
}

/** `Running <command> · 12 min` for the longest-running call, or null when none has run a minute. */
export function longToolText(rows: readonly LongRunningToolRow[], now: number): string | null {
    let oldest: LongRunningToolRow | undefined;
    for (const row of rows) if (!oldest || row.startedAt < oldest.startedAt) oldest = row;
    if (!oldest) return null;
    const minutes = minutesRunning(oldest.startedAt, now);
    if (minutes < 1) return null;
    const title = oldest.title.trim().replace(/\s+/g, " ");
    const command = title.length > MAX_COMMAND_CHARS ? `${title.slice(0, MAX_COMMAND_CHARS - 1)}…` : title;
    return `Running ${command} · ${minutes} min`;
}

/** The note for an agent mid-turn on one long tool call. Reads `now` so it counts up with the row's tick. */
export function LongToolNote(props: {
    rows: Accessor<readonly LongRunningToolRow[]>;
    live: Accessor<boolean>;
    now: Accessor<number>;
}): JSX.Element {
    const text = () => (props.live() ? longToolText(props.rows(), props.now()) : null);
    return (
        <Show when={text()}>
            {(t) => (
                <span
                    class="swarm-long-tool"
                    title="A tool call that has been running a while. Nothing is wrong with it by itself."
                >
                    {t()}
                </span>
            )}
        </Show>
    );
}
