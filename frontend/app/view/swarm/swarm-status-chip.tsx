// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The status chip on a Swarm row (moved out of swarm-view.tsx, which is near the size limit). */
import { ActivityAnimation } from "@/app/element/ActivityAnimation";
import { activityBucket, type Bucket } from "@/app/element/activity-bucket";
import { AsciiSpinner } from "@/app/element/AsciiSpinner";
import { useTick } from "@/app/hook/useTick";
import { isActiveStatus, rankAgentStatus } from "@/app/store/agent-status";
import { questionCountdown } from "@/app/store/question-timer";
import { Show, createMemo, type JSX } from "solid-js";
import type { AgentDisplayStatus } from "./swarm-display-status";
import { longRunningToolRows } from "./swarm-longrunning";

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
 *  the same check as the row's "Waiting for you" line. `browserRequest` is
 *  `awaitsBrowserAnswer(meta, agentStatus)`: the agent ended its turn with a
 *  Browser request still open, which asks the person with no turn in flight. */
export function chipStatus(status: AgentDisplayStatus, awaitingUser = false, browserRequest = false): ChipStatus {
    const ranked = rankAgentStatus({
        live: status === "working" || status === "tools" || status === "stopping",
        needsYou: awaitingUser,
        browserRequest,
        held: { stopping: status === "stopping" },
    });
    if (ranked.kind === "needs-you") return "question";
    return isActiveStatus(ranked) ? "working" : "idle";
}

/** A working agent's longest-running tool call (30 s or more), as its kind of work: the chip shows it in place of the slash. */
function currentOperation(blockId: string | undefined): { bucket: Bucket; since: number } | null {
    const oldest = longRunningToolRows(blockId ?? null, Date.now()).at(-1);
    return oldest ? { bucket: activityBucket({ command: oldest.command }), since: oldest.startedAt } : null;
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
    browserRequest?: boolean;
    blockId?: string;
}): JSX.Element {
    const chip = () => chipStatus(props.status, props.awaitingUser, props.browserRequest);
    const countdown = () => (chip() === "question" && props.blockId ? questionCountdown(props.blockId) : null);
    const tick = useTick(1000);
    const operation = createMemo(() => (chip() === "working" ? (tick(), currentOperation(props.blockId)) : null));
    return (
        <span class={`swarm-status-chip swarm-status-chip--${chip()}`}>
            {/* The two animated states carry the ASCII spinner (the composer's
                mark, one shared timer); a working agent whose current tool call has run long shows that
                kind of work instead (ActivityAnimation); the resting state keeps a still dot. */}
            <Show when={chip() !== "idle"} fallback={<span class="swarm-status-dot" />}>
                <Show when={operation()} fallback={<AsciiSpinner class="swarm-status-spinner" />}>
                    {(op) => <ActivityAnimation bucket={op().bucket} since={op().since} />}
                </Show>
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
