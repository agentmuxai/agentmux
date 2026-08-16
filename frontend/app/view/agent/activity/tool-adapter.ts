// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tool adapter — promotes an in-flight Bash `ToolNode` to a `PinnedActivity`
 * once it has been running past `TOOL_PROMOTION_MS`, mirroring
 * shell-adapter.ts's pattern. Unlike shells (an explicit, agent-opt-in MCP
 * tool already tracked end-to-end), an ordinary Bash tool call is otherwise
 * completely invisible to the dock — this is what closes that gap.
 *
 * Classification is duration-first, not text-pattern-first: any Bash call
 * still running past the threshold promotes, regardless of what its command
 * text looks like. This sidesteps false-positive risk from trying to
 * pattern-match `sleep`/`wait`/etc against arbitrary shell one-liners.
 * TOOL_PROMOTION_MS reuses the figure from
 * SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15.md §5's "overrun promotion".
 *
 * Once promoted, a call keeps its dock row through the normal done/error
 * retention window instead of vanishing the instant ToolEnd lands — same
 * "linger forever in the adapter's output, let the dock's own RETENTION_MS
 * filtering decide visibility" shape shell-adapter.ts uses (see
 * ActivityDock.tsx's `visible()` and its comment on `hasExpiring`).
 * `ToolNode` has no explicit "ended at" field, so a finished call's
 * `endedAt` is derived from `timestamp + duration` (duration is only ever
 * populated once the call is terminal).
 *
 * See docs/specs/REPORT_LONGRUNNING_TOOLCALL_AUTODETECT_STATUS_2026_07_26.md
 * §3 step 1, §4.1-4.3.
 */

import { extractToolDetail } from "../stream-parser";
import type { BashParams, BashResult, DocumentNode, ToolNode } from "../types";
import type { ActivityStatus, PinnedActivity } from "./types";

export const TOOL_PROMOTION_MS = 30_000;

/** The harness's literal acceptance-message prefix for a genuinely detached
 *  launch (see BashParams.run_in_background's doc comment). Exported so any
 *  other code that needs to recognize the same signal doesn't redefine it. */
export const BACKGROUND_LAUNCH_ACCEPTED_PREFIX = "Command running in background with ID:";

function isBashToolNode(n: DocumentNode): n is ToolNode {
    return n.type === "tool" && n.tool === "Bash" && n.timestamp != null;
}

/**
 * The result's raw text, whichever field it landed in. claude-translator.ts's
 * `buildToolResults` normally puts it in `stdout` (the structured
 * `{ stdout, stderr, interrupted }` sibling), but falls back to a plain
 * `{ content: string }` shape when Claude omits a terminal-shaped
 * `tool_use_result` or returns multiple tool_result blocks (the structured
 * sibling is then unattributable to any one block) — codex P1 on this PR:
 * checking `stdout` alone missed that fallback shape, so a genuinely
 * detached launch whose acceptance text arrived via `content` was rejected
 * outright and vanished from the dock entirely instead of just being
 * misclassified.
 */
function resultText(result: ToolNode["result"]): string | undefined {
    const r = result as (BashResult & { content?: unknown }) | undefined;
    if (typeof r?.stdout === "string") return r.stdout;
    if (typeof r?.content === "string") return r.content;
    return undefined;
}

/**
 * True for a Bash call the harness ran detached (issue #2490): its
 * `tool_use.input` carried `run_in_background: true` AND the tool_result's
 * own text actually is the acceptance message ("Command running in
 * background with ID: …"), not the command's real output. The ToolNode is
 * terminal within ~a second, but the real process tree keeps running until
 * a `<task-notification>` lands — so, unlike an ordinary call,
 * terminal-with-success here means STARTED, not finished.
 *
 * The `params.run_in_background === true` flag alone is NOT sufficient —
 * issue #2518: the harness decides per-call whether a command actually
 * gets detached; a command that finishes fast enough is returned
 * synchronously (result text `<exited N in Ts>\n<real output>`, the SAME
 * shape an ordinary call gets) even though the caller asked for
 * `run_in_background: true`. Treating that case as "launch accepted, wait
 * for a `<task-notification>` that will never come" left the dock row
 * `running` forever with a growing timer — confirmed live: a single
 * session with 17 backgrounded calls left 11 stuck this way, each one
 * exactly the fast-finishing case. A failed launch (`status: "failed"` —
 * the harness refused the command) is also not a live background task and
 * falls through to the ordinary duration rules, same as before.
 *
 * Exported so `pushDockNodeStatus`/the orphan-scrub push (issue #2520)
 * reuse this exact classification instead of the raw `params` flag —
 * codex P2 / reagent P1 on PR #2520: forwarding the raw flag tagged the
 * MAJORITY of backgrounded calls `bg` in `muxspect dock` (11 of the 17 in
 * #2518's own motivating session were fast-finishing, i.e. NOT accepted),
 * making the column noisy on the common case instead of a signal.
 */
export function isAcceptedBackgroundLaunch(n: ToolNode): boolean {
    if ((n.params as BashParams | undefined)?.run_in_background !== true || n.status !== "success") return false;
    const text = resultText(n.result);
    return typeof text === "string" && text.startsWith(BACKGROUND_LAUNCH_ACCEPTED_PREFIX);
}

/**
 * Parses a single `<task-notification>` user message into the
 * `tool_use_id` it belongs to and its terminal status. Exported so both
 * `backgroundCompletions` (the dock's own display computation) and
 * `stream-flush-queue.ts`'s `pushDockNodeStatus` (the srv-side push, issue
 * #2492's Background Task Registry) parse this exact wire shape identically
 * instead of two independently-drifting regexes — same "one classifier,
 * every consumer reuses it" discipline `isAcceptedBackgroundLaunch` above
 * already established after the #2518 incident.
 *
 * The harness reports a background task's ACTUAL completion as a plain
 * user message whose whole payload is a `<task-notification>` block naming
 * the `<tool-use-id>` it belongs to and a `<status>` — that message (not
 * the instant tool_result) is the background task's real end-of-life
 * signal. Parsed leniently: a notification without a recognizable status
 * still ends the task (as "stopped") rather than leaving a finished
 * process shown as running forever. Returns `null` for anything that
 * isn't a task-notification, or is one with no parseable tool-use-id.
 */
export function parseTaskNotification(message: string): { toolUseId: string; status: ActivityStatus } | null {
    if (!message.includes("<task-notification>")) return null;
    const toolUseId = /<tool-use-id>([^<]+)<\/tool-use-id>/.exec(message)?.[1];
    if (!toolUseId) return null;
    const rawStatus = /<status>([^<]+)<\/status>/.exec(message)?.[1];
    const status: ActivityStatus =
        rawStatus === "completed" ? "done" : rawStatus === "failed" ? "error" : "stopped";
    return { toolUseId, status };
}

/**
 * Terminal outcomes of backgrounded calls, keyed by originating
 * `tool_use_id`. See `parseTaskNotification` above for the wire shape.
 */
function backgroundCompletions(nodes: ReadonlyArray<DocumentNode>): Map<string, { status: ActivityStatus; endedAt?: number }> {
    const out = new Map<string, { status: ActivityStatus; endedAt?: number }>();
    for (const n of nodes) {
        if (n.type !== "user_message") continue;
        const parsed = parseTaskNotification(n.message);
        if (!parsed) continue;
        out.set(parsed.toolUseId, { status: parsed.status, endedAt: n.timestamp });
    }
    return out;
}

/** True while `n` is running and has already crossed the promotion
 *  threshold as of `now` — the "should suppress AgentWorkingRow / should
 *  schedule a promotion timer" condition. Deliberately narrower than
 *  `everCrossedThreshold` below: it must NOT go true for a call that has
 *  already finished (that case is handled entirely by its retained,
 *  terminal-status dock row, not by anything time-gated). */
function isRunningPastThreshold(n: ToolNode, now: number): boolean {
    return n.status === "running" && now - n.timestamp! >= TOOL_PROMOTION_MS;
}

/** True once `n` is known to have run for at least `TOOL_PROMOTION_MS`,
 *  whether it's still running now or has already finished. A call that
 *  finished in under the threshold never appears in the dock at all — this
 *  is the "did this call ever deserve a dock row" gate for `toolActivities`. */
function everCrossedThreshold(n: ToolNode, now: number): boolean {
    if (n.status === "running") return isRunningPastThreshold(n, now);
    if (n.duration != null) return n.duration * 1000 >= TOOL_PROMOTION_MS;
    return false;
}

function toolActivityStatus(status: ToolNode["status"]): ActivityStatus {
    switch (status) {
        case "running": return "running";
        case "success": return "done";
        case "failed": return "error";
        // denied/canceled/pending_approval/awaiting_answer — cut off or
        // never actually ran long, not a failure signal. Same bucket as
        // subagent-adapter.ts's "abandoned" → "stopped".
        default: return "stopped";
    }
}

export function toolToActivity(n: ToolNode): PinnedActivity {
    const detail = extractToolDetail(n.tool, (n.params as Record<string, any>) ?? {});
    return {
        id: n.id,
        kind: "tool",
        title: detail || n.toolName || n.tool,
        status: toolActivityStatus(n.status),
        startedAt: n.timestamp!,
        endedAt: n.status !== "running" && n.duration != null ? n.timestamp! + n.duration * 1000 : undefined,
        // No cancel path exists for a single in-flight tool call today (only
        // a whole-turn interrupt) — same reasoning as subagent-adapter.ts's
        // canStop: false.
        canStop: false,
        tool: n,
    };
}

/** Every Bash tool call that has ever crossed `TOOL_PROMOTION_MS` — running
 *  ones past the threshold, plus finished ones whose total duration cleared
 *  it (still returned after they finish so the dock's own RETENTION_MS
 *  window, not this function, decides when the row actually disappears).
 *
 *  PLUS every accepted backgrounded call (issue #2490): those are
 *  agent-declared long-running work, so they get a dock row immediately
 *  (no 30s threshold — the duration heuristic exists to auto-detect
 *  undeclared long work, and a declared background task needs no
 *  detection) that stays `running` until the harness's
 *  `<task-notification>` for this exact `tool_use_id` lands. The
 *  attached-task axis (attached-task.ts) picks these up with no changes,
 *  by construction — spec §4 of
 *  SPEC_ATTACHED_TASK_STATUS_AXIS_2026_08_02.md. */
export function toolActivities(nodes: ReadonlyArray<DocumentNode>, now: number): PinnedActivity[] {
    const out: PinnedActivity[] = [];
    let completions: Map<string, { status: ActivityStatus; endedAt?: number }> | null = null;
    for (const n of nodes) {
        if (!isBashToolNode(n)) continue;
        if (isAcceptedBackgroundLaunch(n)) {
            // Built lazily: the vast majority of documents contain no
            // backgrounded calls, and scanning every user message for
            // notifications on every recompute would be pure waste there.
            completions ??= backgroundCompletions(nodes);
            const completion = completions.get(n.id);
            const activity = toolToActivity(n);
            if (completion) {
                out.push({ ...activity, status: completion.status, endedAt: completion.endedAt });
            } else {
                // Still running for real, whatever the instant
                // tool_result said — override the terminal status the
                // plain mapping derived, and drop the endedAt derived
                // from the launch acknowledgment's duration.
                out.push({ ...activity, status: "running", endedAt: undefined });
            }
            continue;
        }
        if (!everCrossedThreshold(n, now)) continue;
        out.push(toolToActivity(n));
    }
    return out;
}

/**
 * Earliest wall-clock time at which some currently-running, not-yet-promoted
 * Bash call will cross the promotion threshold — or null if none are
 * pending. Lets a caller schedule a single one-shot timer for exactly when
 * the next promotion should happen, instead of a continuous per-second tick
 * (see ActivityDock.tsx's `hasExpiring` for the same discipline applied to
 * retention expiry). Scoped to running calls only — a finished call's dock
 * row is decided synchronously from its final `duration`, nothing to
 * schedule for it.
 */
export function nextToolPromotionAt(nodes: ReadonlyArray<DocumentNode>, now: number): number | null {
    let next: number | null = null;
    for (const n of nodes) {
        if (!isBashToolNode(n) || n.status !== "running") continue;
        const promotesAt = n.timestamp! + TOOL_PROMOTION_MS;
        if (promotesAt <= now) continue;
        if (next == null || promotesAt < next) next = promotesAt;
    }
    return next;
}

/**
 * True if the pane currently has a live, already-promoted Bash call —
 * used by AgentWorkingRow to suppress its own "tool · arg" text once the
 * dock is already showing it (report §4.3). Deliberately narrower than
 * `toolActivities`: a *finished* call still lingering in the dock during
 * its retention window must not suppress a different, newly-started tool
 * call's own working-row text.
 */
export function hasRunningPromotedTool(nodes: ReadonlyArray<DocumentNode>, now: number): boolean {
    for (const n of nodes) {
        if (isBashToolNode(n) && isRunningPastThreshold(n, now)) return true;
    }
    return false;
}
