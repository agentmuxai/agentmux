// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The status chip on a Swarm row (moved out of swarm-view.tsx, which is near the size limit). */
import { AsciiSpinner } from "@/app/element/AsciiSpinner";
import { isActiveStatus, rankAgentStatus } from "@/app/store/agent-status";
import { questionCountdown } from "@/app/store/question-timer";
import { Show, type JSX } from "solid-js";
import type { AgentDisplayStatus } from "./swarm-display-status";

/**
 * The chip collapses the finer AgentDisplayStatus to three states:
 * **working**, the agent is mid-turn, whether generating, running tools, or
 * stopping, in red (user, 2026-09-27); **question**, mid-turn and waiting on
 * the user's answer, in the theme's primary colour; and **idle**, everything
 * else, in green. The finer-grained AgentDisplayStatus is still computed and
 * still drives other logic (e.g. which subagent rows can be retired); only
 * what the chip shows collapses.
 * docs/specs/SPEC_SWARM_QUESTION_STATE_AND_QUESTION_TIMEOUT_ACTIVITY_2026_10_02.md §3.
 */
export type ChipStatus = "working" | "question" | "idle";

/** `awaitingUser` is `isAwaitingUser(meta, agentStatus)` from swarm-line.ts,
 *  the same check as the row's "Waiting for you" line. */
export function chipStatus(status: AgentDisplayStatus, awaitingUser = false): ChipStatus {
    const ranked = rankAgentStatus({
        live: status === "working" || status === "tools" || status === "stopping",
        needsYou: awaitingUser,
        held: { stopping: status === "stopping" },
    });
    if (ranked.kind === "needs-you") return "question";
    return isActiveStatus(ranked) ? "working" : "idle";
}

// Reads `props.status`, not a destructured `status`: in Solid a destructured
// prop is read once at creation, so the chip would never follow the agent from
// working to idle.
//
// A question shows the one question timer's countdown (store/question-timer.ts),
// the same seconds as the question panel: "question 23s", or "question · paused"
// while the user is active. No timer state (the pane isn't mounted anywhere, or
// a remote row) shows plain "question".
export function AgentStatusChip(props: {
    status: AgentDisplayStatus;
    awaitingUser?: boolean;
    blockId?: string;
}): JSX.Element {
    const chip = () => chipStatus(props.status, props.awaitingUser);
    const countdown = () => (chip() === "question" && props.blockId ? questionCountdown(props.blockId) : null);
    return (
        <span class={`swarm-status-chip swarm-status-chip--${chip()}`}>
            {/* The two animated states carry the ASCII spinner (the composer's
                mark, one shared timer); the resting state keeps a still dot. */}
            <Show when={chip() !== "idle"} fallback={<span class="swarm-status-dot" />}>
                <AsciiSpinner class="swarm-status-spinner" />
            </Show>
            <span class="swarm-status-label">{chip()}</span>
            <Show when={countdown()}>
                {(cd) => (
                    <Show when={!cd().paused} fallback={<span class="swarm-status-countdown">· paused</span>}>
                        <span class={`swarm-status-countdown swarm-status-countdown--${cd().band}`}>
                            {cd().seconds}s
                        </span>
                    </Show>
                )}
            </Show>
        </span>
    );
}
