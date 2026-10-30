// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The working row under the transcript: what the agent is doing now, or how the last turn went. */

import { Show, createEffect, createMemo, createSignal, on, onCleanup, untrack, type JSX } from "solid-js";
import { useTick } from "@/app/hook/useTick";
import { shouldReduceMotion } from "@/app/motion/motion";
import { formatElapsedCompact } from "@/util/format-time";
import type { ActivityState, CompactionState, ResumeRetryState, TurnCarry } from "@/app/store/agent-pane-state/types";
import { presentStatus, TIMING, type StatusMemory } from "../status/present-status";
import { AsciiSpinner } from "@/app/element/AsciiSpinner";
import { compactionEstimate, compactionProgress, readCompactionSamples, samplesForModel } from "../compaction-estimate";
import { turnEndedAt, turnLiveOutput, turnOpen, turnSettling, type TurnLedger } from "@/app/store/agent-pane-state/turn-ledger";
import type { SessionStats, TurnTokens } from "../types";
import type { LaunchPhase } from "../flows/launch-phase";
import { rowStatusWords } from "../status/row-status";
import { absorbedSummary, TRIGGER_LEAD_IN_MS, triggerLeadIn, workedVerb } from "../turn-trigger-text";
import { Button } from "@/app/element/ui/Button";
import { fmtOutputTokens, fmtTurnTokens, ingToEd, pickThinkingPhrase } from "./status-formatters";

// ── AgentWorkingRow ───────────────────────────────────────────────────────────
// Rendered immediately below AgentDocumentView in the conversation area.
// Shows spinner + "Working… · Ns" while loading, "✓ Worked · 42s" on completion.
// Returns null when neither loading nor has stats — no idle placeholder.
// Stays visible as a turn delimiter until the user sends the next message.

interface AgentWorkingRowProps {
    loading: boolean;
    stopping?: boolean;
    /** The pane's ambient (Haiku-written) session summary, or the CLI's own
     *  window-title topic when there isn't one yet — `readSwarmSummary` of the
     *  block meta, the same line the Swarm row shows. Shown in the left zone
     *  in place of the old "tool · arg" text; the phrase cycle covers the gap
     *  until one exists. */
    activitySummary?: string | null;
    sessionStats?: SessionStats | null;
    /** Live token counts for the CLI pass in progress; the right zone shows
     *  the turn's output so far, with ↑/↓ for the request phase, next to the
     *  elapsed time. */
    turnTokens?: TurnTokens | null;
    /** srv's turn ledger (`agentturn`): the turn as the user sees it, over
     *  however many CLI passes. While it is open the row times and counts the
     *  whole turn, and stays up (dimmed) between two of its passes; once it
     *  ends, the Worked line reports it. Absent (older srv): per pass, as before.
     *  SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §4.4. */
    turnLedger?: TurnLedger | null;
    /** Each pass's figures summed over the turn, for providers srv doesn't
     *  count (`AgentPaneState.turnCarry`). */
    turnCarry?: TurnCarry | null;
    /** What the agent is doing right now (`AgentPaneState.activity`): the
     *  live status's facts. SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §6. */
    activity?: ActivityState | null;
    /** A question or approval waiting on the user, in words: it outranks
     *  everything else the row could say. */
    needsYou?: string | null;
    /** Set when the provider is rate-limited; shows "Rate limited…" in place of thinking phrase. */
    waitingReason?: "rate_limited" | null;
    /** When the provider's next retry is due (epoch ms, from Retry-After). The
     *  row counts down to it while waitingReason is set. */
    retryAtMs?: number | null;
    /** What the launch/login flow is doing right now — shown in place of the
     *  generic thinking phrase so a timed wait (login link, login completion)
     *  never renders as bare "Working…". See launch-phase.ts. */
    launchPhase?: LaunchPhase | null;
    /** Cancel the in-flight login. Shown as a small button while launchPhase
     *  is a pre-authUrl wait (opening-login-terminal /
     *  waiting-for-login-completion) — AuthUrlBox has its own cancel button
     *  once a URL exists, but before that this is the only affordance. */
    onCancelLogin?: () => void;
    /** True while AuthUrlBox is already showing its own Cancel button (a URL
     *  was captured — reagent P2 on PR #2300: tier 1's "opened" outcome sets
     *  launchPhase to "waiting-for-login-completion" too, so without this the
     *  row rendered a second, redundant Cancel button alongside AuthUrlBox's. */
    hasAuthUrl?: boolean;
    /** Live "compaction in progress" state, or null — relocated here from
     *  AgentComposerStrip (2026-08-27, Part 2 of
     *  SPEC_REMOVE_AGENT_UNRESPONSIVE_DETECTION_2026_08_25.md) so a user has
     *  exactly one place to check "is something happening right now." Takes
     *  priority over the normal Working/tool display. */
    compacting?: CompactionState | null;
    /** Tokens in the pane's context (`state.context`, validated). While
     *  `compacting`, the row estimates how long this compaction will take from
     *  earlier ones (`compaction-estimate.ts`) scaled by this, and shows an
     *  ESTIMATE bar — Claude Code reports no real compaction progress, so the
     *  bar is labeled as an estimate. Read once when the compaction starts, so
     *  the estimate doesn't move mid-run. No earlier compactions → the built-in
     *  default (`DEFAULT_COMPACTION_MS`). */
    compactionContextTokens?: number | null;
    /** The pane's model key (`compactionModelKey`), to estimate from that model's past compactions. */
    compactionModel?: string | null;
    /** Live "recovering from a stale --resume session id" state, or null —
     *  same relocation as `compacting` above. Fires ONLY after the
     *  underlying process has already crashed/exited, so this can be set
     *  while `loading` is false — the row must render for this alone. Takes
     *  priority over `compacting` (mirrors the old composer-strip priority:
     *  a silent-gap recovery is more easily mistaken for a hang). */
    reconnecting?: ResumeRetryState | null;
}

// reagent P2 on PR #2304: "waiting-for-login-link" (tier 1's own up-to-15s
// countdown) was missing here, so that phase's real timed wait had no cancel
// affordance anywhere — AuthUrlBox can't show one yet (no URL captured), and
// the row's own button was withheld for exactly this phase.
const CANCELLABLE_LAUNCH_PHASES = new Set([
    "first-login",
    "auth-expired",
    "waiting-for-login-link",
    "opening-login-terminal",
    "waiting-for-login-completion",
]);

/** A turn something other than the user started opens by saying so. */
function leadInText(props: AgentWorkingRowProps, nowMs: number): string | null {
    const l = props.turnLedger;
    if (!l || !turnOpen(l, nowMs) || nowMs - l.startedAtMs >= TRIGGER_LEAD_IN_MS) return null;
    return triggerLeadIn(l.trigger);
}

export const AgentWorkingRow = (props: AgentWorkingRowProps): JSX.Element => {
    const [phrase, setPhrase] = createSignal(pickThinkingPhrase());
    const [lastPhrase, setLastPhrase] = createSignal(pickThinkingPhrase());
    const tick = useTick(1000);

    // ── Type-out ────────────────────────────────────────────────────────
    // First display of a new left-zone string (a phrase, summary, or status
    // change) reveals character-by-character. The gradient highlight that
    // used to sweep over it afterwards is gone: the text is one solid color
    // now (SPEC_AGENT_WORKING_ROW_MONO_SUMMARY_2026_10_02.md); the ASCII
    // spinner beside it carries the "still working" signal. Originally
    // SPEC_AGENT_WORKING_INDICATOR_SHIMMER_AND_MIC_RELOCATION_2026_07_08.md §2.
    //
    // When the motion policy reduces `transition` (motion/motion.ts; dormant
    // today), skips the one-shot type-out and jumps straight to the full text.
    const reducedMotion = () => shouldReduceMotion("transition");
    // Between two passes of one turn: no pass runs, the next is expected. The
    // row keeps its live form (dimmed) rather than flash "Worked" and take it
    // back a moment later. Display only: the composer's busy gate is not this.
    const settling = createMemo(() => turnSettling(props.turnLedger, (tick(), Date.now())));
    const live = createMemo(() => props.loading || settling());
    // tick() re-runs this memo every second so a phase's "up to Ys" countdown
    // (formatPhaseLabel) stays live — see useTick.ts's "always-on tick" pattern.
    const [loadStartMs, setLoadStartMs] = createSignal<number | null>(null);
    // A turn's first line types out too (operator, 2026-10-10; it showed whole
    // since 2026-08-10). Going live forgets the presenter's memory, so that line
    // is new to it and its dwell counts from the end of its own type-out.
    let wasLive = false;
    // The live status (status/present-status.ts): what to say, and when, from
    // what the agent is doing and what the row showed last.
    let statusMemory: StatusMemory | null = null;
    const status = createMemo(() => {
        const now = (tick(), Date.now());
        if (live() && !wasLive) statusMemory = null;
        wasLive = live();
        const l = props.turnLedger;
        const words = rowStatusWords(
            {
                live: live() || !!props.compacting || !!props.reconnecting,
                needsYou: props.needsYou ?? null,
                reconnecting: !!props.reconnecting,
                compacting: !!props.compacting,
                stopping: !!props.stopping,
                waitingReason: props.waitingReason,
                retryAfterMs: props.retryAtMs != null ? Math.max(0, props.retryAtMs - now) : null,
                launchPhase: props.launchPhase,
            },
            now,
        );
        const r = presentStatus(
            {
                nowMs: now,
                ...words,
                leadIn: leadInText(props, now),
                activity: props.activity ?? null,
                turnStartedAt: l && turnOpen(l, now) ? l.startedAtMs : untrack(loadStartMs),
                goal: props.activitySummary?.trim() || null,
                phrase: phrase(),
                instantReveal: untrack(reducedMotion),
            },
            statusMemory,
        );
        statusMemory = r.memory;
        return r.line;
    });
    const leftText = () => status().text;
    // Deduplicated: `on()` re-runs whenever its source fires, and `status` is a
    // new object every tick, so `on(() => status().key)` re-typed it each second.
    const statusKey = createMemo(() => status().key);
    // How much of the line is typed out; Infinity once it is all there, so a
    // counter that grows inside the same line shows in full.
    const [revealed, setRevealed] = createSignal(Number.POSITIVE_INFINITY);


    // Every new line types out, and so does the line on screen when the row
    // goes live; a moved counter (same key) never re-types it. The presenter's
    // dwell counts from the end of this type-out.
    createEffect(
        on(
            [statusKey, live],
            () => {
                const text = untrack(leftText);
                if (untrack(reducedMotion) || !text || !untrack(live)) {
                    setRevealed(Number.POSITIVE_INFINITY);
                    return;
                }
                setRevealed(0);
                // By the clock, so a throttled timer catches up (the dwell's clock too).
                const startedAt = Date.now();
                const id = setInterval(() => {
                    const n = Math.floor((Date.now() - startedAt) / TIMING.revealCharMs);
                    if (n < untrack(leftText).length) return void setRevealed(n);
                    clearInterval(id);
                    setRevealed(Number.POSITIVE_INFINITY);
                }, TIMING.revealCharMs);
                onCleanup(() => clearInterval(id));
            },
        ),
    );
    // From the turn's start while it is open, so the clock runs across its
    // passes and survives a remount; else from when this row went live.
    const elapsedMs = createMemo(() => {
        const now = (tick(), Date.now());
        const l = props.turnLedger;
        if (l && turnOpen(l, now)) return Math.max(0, now - l.startedAtMs);
        const s = loadStartMs();
        return s != null ? now - s : 0;
    });
    // The turn's output so far, never shown going down within one turn (a
    // pass's exact count can land under its streamed estimate).
    let shownOutput: { turnId: number | null; output: number } = { turnId: null, output: 0 };
    const liveOutput = createMemo((): number | undefined => {
        const output = turnLiveOutput(props.turnLedger, props.turnTokens, (tick(), Date.now()));
        if (output == null) return undefined;
        const turnId = props.turnLedger?.turnId ?? null;
        const floor = turnId != null && shownOutput.turnId === turnId ? shownOutput.output : 0;
        shownOutput = { turnId, output: Math.max(floor, output) };
        return shownOutput.output;
    });

    // Live elapsed time since compaction/reconnecting-retry started —
    // relocated from AgentComposerStrip (2026-08-27, Part 2 of
    // SPEC_REMOVE_AGENT_UNRESPONSIVE_DETECTION_2026_08_25.md). Same
    // stopwatch pattern as elapsedMs above.
    const compactingElapsedMs = createMemo(() => {
        const c = props.compacting;
        return c ? (tick(), Date.now() - c.startedAt) : 0;
    });
    const reconnectingElapsedMs = createMemo(() => {
        const r = props.reconnecting;
        return r ? (tick(), Date.now() - r.startedAt) : 0;
    });
    // Estimated compaction progress (Tier 4). Only while compacting, and never
    // while reconnecting (which takes the row over). With no earlier
    // compactions the estimate is the built-in default.
    // The estimate is keyed on `startedAt` and reads the context size untracked,
    // so it is computed once per compaction and stays put while it runs.
    const compactStartedAt = createMemo(() => props.compacting?.startedAt ?? null);
    const compactionEstimateFor = createMemo(() =>
        compactStartedAt() == null
            ? null
            : compactionEstimate(
                  samplesForModel(readCompactionSamples(), untrack(() => props.compactionModel)),
                  untrack(() => props.compactionContextTokens)
              )
    );
    const compactionBar = createMemo(() => {
        const est = compactionEstimateFor();
        if (!props.compacting || props.reconnecting || !est) return null;
        return { ...est, ...compactionProgress(compactingElapsedMs(), est.estimateMs) };
    });
    const compactionTitle = (): string | undefined => {
        const bar = compactionBar();
        if (!bar) return undefined;
        return bar.fromHistory
            ? "Estimate from your earlier compactions. Claude Code doesn't report compaction progress."
            : "Estimate: a compaction usually takes about a minute. Claude Code doesn't report compaction progress.";
    };

    createEffect(() => {
        if (!live()) return;
        setPhrase(pickThinkingPhrase());
        const id = setInterval(() => {
            setPhrase((prev) => {
                const next = pickThinkingPhrase(prev);
                setLastPhrase(next);
                return next;
            });
        }, 30000);
        onCleanup(() => clearInterval(id));
    });

    createEffect(() => {
        if (live()) {
            setLoadStartMs((prev) => prev ?? Date.now());
        } else {
            setLoadStartMs(null);
        }
    });

    createEffect(() => {
        if (live()) setLastPhrase(phrase());
    });

    // The turn that just ended, when srv reported it: its figures cover every
    // pass, where `sessionStats` is the last pass's alone.
    const endedTurn = createMemo(() => {
        const l = props.turnLedger;
        if (!l) return null;
        const endedAt = turnEndedAt(l, (tick(), Date.now()));
        return endedAt != null ? { l, durationMs: endedAt - l.startedAtMs } : null;
    });

    // The whole turn's figures: srv's, when it counted the passes; else the
    // pane's own sum of each pass's stats, for the same turn.
    const turnFigures = createMemo((): { outputTokens: number; costUsd: number; steps: number } | null => {
        const turn = endedTurn();
        if (!turn) return null;
        if (turn.l.countedPasses > 0) return turn.l;
        const carry = props.turnCarry;
        return carry && carry.turnId === turn.l.turnId && carry.passes > 1 ? carry : null;
    });

    const workedSummary = createMemo((): string | null => {
        const stats = props.sessionStats;
        if (!stats) return null;
        const turn = endedTurn();
        // Named for what started it when that wasn't the user.
        const parts: string[] = ["✓ " + (turn ? workedVerb(turn.l.trigger) : ingToEd(lastPhrase()))];
        const durationMs = turn?.durationMs ?? stats.duration_ms;
        if (durationMs != null) parts.push(formatElapsedCompact(Math.max(1_000, durationMs)));
        // The turn's output: the results' exact figures (summed over its
        // calls and passes), where the live row showed it growing.
        const figures = turnFigures();
        const output = figures ? figures.outputTokens : stats.output_tokens;
        if (output != null) parts.push(fmtOutputTokens(output));
        return parts.join("  ·  ");
    });

    const workedSecondary = createMemo((): string | null => {
        const stats = props.sessionStats;
        if (!stats) return null;
        const turn = endedTurn();
        const counted = turnFigures();
        const parts: string[] = [];
        // A provider that reports no cost (Codex) counts 0: show its own, if any.
        const cost = counted && counted.costUsd > 0 ? counted.costUsd : stats.cost_usd;
        if (cost != null) parts.push(`$${cost.toFixed(3)}`);
        // Model calls, which is what `result.num_turns` counts: steps, not turns.
        const steps = counted ? counted.steps : stats.num_turns;
        if (steps) parts.push(`${steps} ${steps === 1 ? "step" : "steps"}`);
        if (turn && turn.l.passes > 1) parts.push(`${turn.l.passes} passes`);
        const joined = turn ? absorbedSummary(turn.l.absorbed, turn.l.inputs) : null;
        if (joined) parts.push(joined);
        return parts.length ? parts.join("  ·  ") : null;
    });

    const rightText = createMemo((): string => {
        // reconnecting/compacting show only the elapsed-time readout, no
        // token counts — matches the old composer-strip behavior these
        // states were relocated from. `reconnecting` takes priority (see
        // its prop doc comment above).
        if (props.reconnecting) return formatElapsedCompact(reconnectingElapsedMs());
        if (props.compacting) {
            const elapsed = formatElapsedCompact(compactingElapsedMs());
            const bar = compactionBar();
            if (!bar) return elapsed;
            return bar.over
                ? `${elapsed} · longer than usual`
                : `${elapsed} / ~${formatElapsedCompact(bar.estimateMs)}`;
        }
        const right: string[] = [];
        const output = liveOutput();
        if (output != null) right.push(fmtTurnTokens(output, props.loading ? props.turnTokens : null));
        right.push(formatElapsedCompact(elapsedMs()));
        return right.join("  \u00b7  ");
    });

    const showCancelLogin = createMemo(
        () =>
            !!props.onCancelLogin &&
            !props.hasAuthUrl &&
            !!props.launchPhase &&
            CANCELLABLE_LAUNCH_PHASES.has(props.launchPhase.kind),
    );

    // No dedicated "Running in background" state here — the ActivityDock's
    // own running row (pinned above the composer, same data source) IS the
    // indicator for a live attached task; a footer copy of it was redundant
    // (user feedback 2026-08-10, reverting the render half of #2489 — the
    // reducer axis itself stays, for the watchdog/Swarm consumers).
    return (
        <Show
            when={live() || !!props.compacting || !!props.reconnecting}
            fallback={
                <Show when={workedSummary()}>
                    <span class="agent-working-row agent-working-row--worked">
                        <span class="agent-working-row-left">{workedSummary()}</span>
                        <span class="agent-working-row-right">
                            <Show when={workedSecondary()}>
                                <span class="agent-working-row-secondary">{workedSecondary()}</span>
                            </Show>
                        </span>
                    </span>
                </Show>
            }
        >
            <span
                class="agent-working-row agent-working-row--loading"
                classList={{ "is-settling": !props.loading && settling() }}
            >
                <AsciiSpinner />
                <span class="agent-working-row-left">
                    <span class="agent-working-row-primary">{leftText().slice(0, revealed())}</span>
                </span>
                <span
                    class="agent-working-row-right"
                    title={compactionTitle()}
                >
                    {rightText()}
                    <Show when={showCancelLogin()}>
                        <Button
                            variant="link"
                            class="agent-working-row-cancel-login"
                            onClick={() => props.onCancelLogin?.()}
                        >
                            Cancel
                        </Button>
                    </Show>
                </span>
                <Show when={compactionBar()}>
                    {(bar) => (
                        <span
                            class="agent-working-row-progress"
                            classList={{ "is-over": bar().over }}
                            role="progressbar"
                            aria-label="Estimated compaction progress"
                            aria-valuemin={0}
                            aria-valuemax={100}
                            aria-valuenow={bar().over ? undefined : Math.round(bar().fraction * 100)}
                            aria-valuetext={
                                bar().over
                                    ? "Taking longer than the estimate"
                                    : `About ${Math.round(bar().fraction * 100)} percent, estimated`
                            }
                        >
                            <span class="agent-working-row-progress-fill" style={{ width: `${bar().fraction * 100}%` }} />
                        </span>
                    )}
                </Show>
            </span>
        </Show>
    );
};

AgentWorkingRow.displayName = "AgentWorkingRow";
