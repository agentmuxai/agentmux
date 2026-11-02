// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Two small notes on a Swarm agent row that say why an agent is quiet
 * (docs/specs/SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md §4.4):
 *
 * - `UnavailableChip`: the agent can't start a turn, because its plan is out of quota until a
 *   reset or its sign-in was refused. It takes the place of the status chip, whose green "idle"
 *   would read as an agent that is simply resting.
 * - `BuildSlotNote`: this agent holds the machine's build slot, so other builds wait for it
 *   (SPEC_BUILD_SLOT_ENFORCEMENT_2026_10_20.md §6). Information only.
 * - `LongToolNote`: a tool call has been running for minutes. Information only: nothing is
 *   killed or flagged as failed (the August removal of hung-turn detection stands,
 *   SPEC_REMOVE_AGENT_UNRESPONSIVE_DETECTION_2026_08_25.md).
 * - `HeldNote`: "2 held", messages typed to the agent while its model API was out, which go out in
 *   order once it answers (and, for a stopped agent, once it starts). Information only
 *   (SPEC_OFFLINE_RESILIENCE_2026_10_20.md §4.4a).
 * - `McpHealthNote`: the agent's AgentMux tools (Message, Memory, Tasks...) are not connected, and
 *   whether AgentMux is repairing them or has given up (SPEC_AGENT_MCP_SELF_HEAL_2026_10_10.md §4.2).
 */

import { unavailableChipText, unavailableTitle, type AgentUnavailable } from "@/app/store/agent-unavailable";
import { Tooltip } from "@/app/element/tooltip";
import { HELD_INPUT_META_KEY } from "@/app/view/agent/offline/useOfflineInput";
import { Show, type Accessor, type JSX } from "solid-js";
import { buildSlotText, buildSlotTitle, useBuildSlotHolder } from "./swarm-build-slot";
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

/** The note on the row of the agent holding the build slot. Reads `now` so the minutes count up. */
export function BuildSlotNote(props: { blockId: string | null; now: Accessor<number> }): JSX.Element {
    const holder = useBuildSlotHolder();
    const mine = () => {
        const h = holder();
        return h && props.blockId && h.block_id === props.blockId ? h : null;
    };
    return (
        <Show when={mine()}>
            {(h) => (
                <span class="swarm-build-slot" title={buildSlotTitle(h())}>
                    {buildSlotText(h(), props.now())}
                </span>
            )}
        </Show>
    );
}

/**
 * How many messages wait in a block's held input for the agent it belongs to: the list is the block's
 * own only when it was typed to the agent the block runs now, as srv and the pane both read it.
 */
export function heldCount(meta: Record<string, any> | undefined): number {
    const held = meta?.[HELD_INPUT_META_KEY];
    if (!held || !held.agentId || held.agentId !== meta?.agentId || !Array.isArray(held.messages)) return 0;
    return held.messages.length;
}

/** The block meta key srv stores the state of the agent's AgentMux tools under (`mcp_health_meta`). */
export const MCP_HEALTH_META_KEY = "agent:mcp_health";

/** The state of the agent's AgentMux tools while they are down or being repaired, else `null`. */
export function mcpHealthState(meta: Record<string, any> | undefined): "repairing" | "down" | null {
    const state = meta?.[MCP_HEALTH_META_KEY]?.state;
    if (state === "healing" || state === "restarting") return "repairing";
    return state === "gave_up" ? "down" : null;
}

/** "Tools down" or "Repairing tools" on an agent's row, when its AgentMux tools are not connected. */
export function McpHealthNote(props: { meta: Record<string, any> | undefined }): JSX.Element {
    const state = () => mcpHealthState(props.meta);
    return (
        <Show when={state()}>
            {(s) => (
                <Tooltip
                    divClassName={`swarm-mcp-health swarm-mcp-health--${s()}`}
                    content={
                        s() === "repairing"
                            ? "This agent's AgentMux tools (Message, Memory, Tasks...) are not connected. AgentMux is repairing them."
                            : "This agent's AgentMux tools (Message, Memory, Tasks...) are not connected and AgentMux could not repair them. Run `agentmux-mcp doctor` in its shell to see why."
                    }
                >
                    {s() === "repairing" ? "Repairing tools" : "Tools down"}
                </Tooltip>
            )}
        </Show>
    );
}

/** What an agent's row says about its state beyond the status chip: messages held, tools not connected. */
export function RowStateNotes(props: { meta: Record<string, any> | undefined }): JSX.Element {
    return (
        <>
            <HeldNote meta={props.meta} />
            <McpHealthNote meta={props.meta} />
        </>
    );
}

/** "N held" on an agent's row, when messages wait for its model API or its start. */
export function HeldNote(props: { meta: Record<string, any> | undefined }): JSX.Element {
    const count = () => heldCount(props.meta);
    return (
        <Show when={count() > 0}>
            <Tooltip
                divClassName="swarm-held"
                content={`${count()} message${count() === 1 ? "" : "s"} typed to this agent wait to be sent, in order, once its model API answers and the agent is running.`}
            >
                {count()} held
            </Tooltip>
        </Show>
    );
}
