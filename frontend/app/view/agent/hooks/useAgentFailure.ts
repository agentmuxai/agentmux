// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentFailure — owns the agent-pane failure-recovery surface.
 *
 * Subscribes to the per-block `agentfailure` wave event (the classified
 * `AgentFailure` from a non-zero exit) and forwards it into the pane
 * reducer as `FailureObserved` — which is what makes an authoritative
 * backend failure classification unconditionally end a working turn,
 * closing the "stuck Waiting after a rate-limit interruption" bug (see
 * docs/analysis/ANALYSIS_AGENT_INPUT_LIFECYCLE_RATELIMIT_SENDNOW_2026_07_06.md).
 * The *active failure itself* (`state.failure`) now lives in
 * `AgentPaneState` — this hook reads it back via the `failure` accessor
 * passed in, instead of holding its own local copy that nothing else could
 * agree with. See docs/specs/SPEC_AGENT_PANE_UNIFIED_FAILURE_REDUCER_2026_07_06.md.
 *
 * What stays hook-local: the expanded-body toggle, the `retrying` flag, and
 * the auto-retry countdown/budget. None of these are facts anything
 * else in the app needs to agree on — they're pure view-presentation timing,
 * the same class as a `<Show>` toggle — so there's no drift risk in keeping
 * them here (same rationale the spec used to leave `expanded` local).
 *
 * The actual recovery *effects* (re-run the turn, re-auth, open Connectors) are
 * passed in by the caller so this hook stays presentation-only otherwise.
 *
 * Auto-retry: for transient classes (rate-limit / overload / network) a
 * countdown arms; clicking Retry fires immediately, reaching 0 fires
 * automatically, Dismiss cancels. Bounded by `AUTO_RETRY_BACKOFF_S` —
 * 5 rungs (5 s → 15 s → 30 s → 60 s → 120 s, each ±20% jittered), ~3.9 min
 * of coverage, then manual-only. See that constant for why the ladder is
 * this long and why it is nonetheless finite. The countdown runs whether or
 * not the pane is visible, like the question timeout, so a transient failure in
 * a background pane recovers on its own. It counts down to a fixed `retryAt`
 * rather than decrementing per tick, so a throttled background window fires on
 * time when its timer finally runs instead of falling behind. A network
 * failure whose API srv tracks arms no countdown while the retry on
 * reconnect is on: it waits for the API to answer. Its Retry goes through
 * the pane's failed-turn recovery either way (`reconnect`,
 * offline/failed-turn.ts).
 *
 * Spec: docs/specs/SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16.md §4–§6,
 * docs/analysis/REPORT_TIMERS_DEADLINES_AND_CLOCKS_AUDIT_2026_10_17.md §4.2,
 * docs/specs/SPEC_OFFLINE_RESILIENCE_2026_10_20.md §4.3.
 */

import { createEffect, createMemo, createSignal, onCleanup, onMount, type Accessor } from "solid-js";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import * as MOS from "@/app/store/mos";
import { getBlockMetaKeyAtom } from "@/app/store/global";
import { setBlockMeta } from "@/app/store/block-meta";
import { addEventListener as addPaneEventListener } from "@/app/store/agent-pane-state-store";
import type { AgentPaneModel } from "@/app/store/agent-pane-model";
import type { PaneFailure } from "@/app/store/agent-pane-state/types";
import type { FailedTurnRecovery } from "../offline/failed-turn";
import { failureToRow, isTransient, type FailureRow } from "../failure/failure-accessory";
import { agoLabel, failureVerdict, isAccountCause, type AccountState, type FailureVerdict } from "../failure/failure-verdict";
import { forgetRefusedSend, refusedSendOf } from "../failure/refused-send";
import { restoreRefusedSend } from "../failure/refused-send-restore";
import { registerRecycleBlocker } from "@/app/store/recycle-blockers";
import { copyErrorReport } from "@/app/errors/CopyErrorButton";
import { formatErrorReport } from "@/app/errors/error-report";
import { sourceOf } from "@/app/store/agent-pane-state/failure-precedence";
import type { FailureClearReason } from "@/app/store/agent-pane-state/types";
import { clearFailure, raiseFailure } from "../failure/failure-writer";

/** How long an armed Take over waits for its confirming click. */
export const TAKEOVER_CONFIRM_MS = 5000;

/**
 * Auto-retry ladder for transient provider failures (429 rate-limited, 529
 * overloaded, network). Exponential, bounded, then manual-only.
 *
 * Was `[5, 10]` — two attempts, ~15s of coverage. That is shorter than a
 * typical Anthropic 429/529 episode, so a genuinely transient overload
 * routinely exhausted the budget and dropped to manual-only while still
 * being retryable. This ladder covers ~4 minutes across 5 attempts, which
 * spans the common case without becoming an unbounded retry loop (the cap
 * is what stops a persistently-throttled turn from retrying forever — see
 * `endEpisode`).
 *
 * Deliberately NOT infinite: a turn that is still failing after ~4 minutes
 * is more likely a sustained outage or an account-level limit than a blip,
 * and at that point a human should decide. `SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16.md` §6.
 */
const AUTO_RETRY_BACKOFF_S = [5, 15, 30, 60, 120] as const; // then manual-only

/** Jitter fraction applied to each ladder rung (±20%). */
const AUTO_RETRY_JITTER = 0.2;

/**
 * Spread concurrent retries so a fleet doesn't re-hit the API in lockstep.
 *
 * A Fleet broadcast, a cron sweep, or several loops can put many agents into
 * the same failure at the same instant; without jitter every one of them
 * retries on the identical second and can re-trigger the very 529 they are
 * backing off from. Applies ±`AUTO_RETRY_JITTER` and floors at 1s so the
 * visible countdown never starts at 0.
 *
 * Pure, with an injectable source, so the ladder is testable deterministically.
 */
export function jitteredBackoffSeconds(baseSeconds: number, rand: () => number = Math.random): number {
    const factor = 1 + (rand() * 2 - 1) * AUTO_RETRY_JITTER;
    return Math.max(1, Math.round(baseSeconds * factor));
}

/** How long a persisted account failure waits for srv's state of the agent before it is shown. */
export const SEED_WAIT_MS = 1500;

export interface UseAgentFailureOptions {
    blockId: string;
    /** Per-pane dispatch handle — default-safe against post-unmount races. */
    model: AgentPaneModel;
    /** Reactive read of the canonical `state.failure` (single source of
     *  truth, set by `FailureObserved` / cleared by `FailureCleared` or the
     *  next `TurnStart`). */
    failure: Accessor<PaneFailure | null>;
    /**
     * What srv says about this agent's account (`accountStateOf` of its `agentruntime` state): the live
     * fact an `auth` or `usage_limit` row is judged against. Omitted is `unknown`: the row stays until
     * a turn completes or it is dismissed, as before.
     */
    liveAccount?: Accessor<AccountState>;
    /** Re-run the failed turn (re-send the last user message). */
    onRetry: () => void;
    /**
     * Re-authenticate this agent's provider account (P2).
     *
     * `turnAttempted` is forwarded from the failure the row was built from
     * (see {@link PaneFailure.turnAttempted}) and MUST be passed through to
     * `relogin({ retryAfterLogin })`: for a never-started agent (`false`)
     * there is no failed turn to re-run, and re-running anyway makes a
     * successful login silently re-send that agent's last OLD message.
     */
    onLoginAgain: (turnAttempted: boolean) => void;
    /**
     * Open a real terminal window for browser-based OAuth (Claude v2.1.x).
     *
     * Takes `turnAttempted` for the SAME reason {@link onLoginAgain} does, and
     * it must be forwarded the same way: this action recovers the pre-launch
     * row too, and a terminal login that "retries" a turn which never ran
     * resends the agent's last old message. Deriving it after the login
     * succeeds does not work — see loginViaTerminal's own doc comment.
     */
    onLoginViaTerminal: (turnAttempted: boolean) => void;
    /** Open Connectors → Accounts. */
    onOpenAccounts: () => void;
    /** Start a fresh agent session (context-window overflow recovery). */
    onNewSession: () => void;
    /**
     * Already-authenticated accounts for this agent's provider that could be
     * bound in one click — see `computeAccountBindCandidates`
     * (bind-account-candidates.ts). Reactive: the row re-derives its
     * "Use <name>" / "Use account…" vs. "Connectors → Accounts" action every
     * time this changes. Optional so callers that haven't wired the account
     * cache yet (e.g. tests) fall back to the pre-existing Connectors-only
     * behavior. See docs/specs/SPEC_AGENT_LOGIN_FLOW_TIGHTENING_2026_09_04.md §3.
     */
    bindCandidates?: Accessor<{ id: string; name: string }[]>;
    /** Bind one of `bindCandidates` (or open a picker for 2+) to this agent. */
    onBindAccount?: (e?: MouseEvent) => void;
    /**
     * Take this agent over from the other AgentMux instance running it
     * (`live_elsewhere`). Resolves once the agent is free here (the caller
     * then re-runs the turn when one was attempted); rejects with a
     * user-facing reason, which the row then shows. Only called after the
     * row's confirming second click. Optional — without it the row offers
     * no Take over. Spec: SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24.md §4.6.
     */
    onTakeOver?: (turnAttempted: boolean) => Promise<void>;
    /** `open_in_other_pane`: focus the pane that holds the agent. */
    onGoThere?: (holderBlockId: string) => void;
    /** `open_in_other_pane`: stop the holder pane and run the agent here, sending the refused message. */
    onMoveHere?: () => Promise<void>;
    /** The pane's failed-turn recovery (offline/failed-turn.ts): a failure
     *  it waits on gets no timed retry, its Retry goes there, and the row's
     *  meta says what happens once the API is back. */
    reconnect?: Pick<FailedTurnRecovery, "waitsForReconnect" | "reconnectHint" | "retry" | "retryBusy" | "failureDismissed">;
}

export interface UseAgentFailureResult {
    /** The PaneRow descriptor for the current failure, or null when clear. */
    row: Accessor<FailureRow | null>;
    /** The current failure judged against what is true now, or null when there is none. A row is never
     *  hidden while this is `active`. */
    verdict: Accessor<FailureVerdict | null>;
}

/** The time a stored failure was recorded (`atMs`, written by srv), else `fallback`. */
function recordedAt(f: unknown, fallback: number): number {
    const at = (f as { atMs?: unknown } | null)?.atMs;
    return typeof at === "number" && Number.isFinite(at) && at > 0 ? at : fallback;
}

export function useAgentFailure(opts: UseAgentFailureOptions): UseAgentFailureResult {
    const [expanded, setExpanded] = createSignal(false);
    const [retrying, setRetrying] = createSignal(false);
    const [autoRetryIn, setAutoRetryIn] = createSignal<number | null>(null);
    // A reload doesn't re-arm the countdown (only a live failure does).
    onCleanup(registerRecycleBlocker(() => (autoRetryIn() != null ? "a failed turn is about to be retried" : null)));
    // Take over's two-step confirmation (`live_elsewhere` only). View-local,
    // like `expanded`: nothing else needs to agree on it.
    const [takeoverArmed, setTakeoverArmed] = createSignal(false);
    const [takingOver, setTakingOver] = createSignal(false);
    let disarmTimer: ReturnType<typeof setTimeout> | undefined;
    // Copy error (SPEC_ERROR_COPY_EVERYWHERE_2026_09_24.md surfaces 1/2) —
    // view-local feedback, same class as `expanded`/`retrying`: presentation
    // timing nothing else needs to agree on.
    const [copyState, setCopyState] = createSignal<"idle" | "copied" | "failed">("idle");
    let copyResetTimer: ReturnType<typeof setTimeout> | undefined;

    // What the failure is judged against, besides srv's account state (`opts.liveAccount`): the pane's last
    // completed turn, whether the auto-retry ladder is spent (a persisted failure read at mount has had no
    // ladder in this pane, so it starts spent), and a tick for a usage limit's reset passing.
    const [completedTurnAt, setCompletedTurnAt] = createSignal<number | null>(null);
    const [ladderEnded, setLadderEnded] = createSignal(false);
    const [clockTick, setClockTick] = createSignal(0);

    let countdown: ReturnType<typeof setInterval> | undefined;
    let autoRetries = 0;
    // Set synchronously by `doRetry` right before it clears the row, and
    // consumed (reset to false) by the `state.failure` transition-effect
    // below. Distinguishes "this hook itself cleared the failure via Retry"
    // (same episode — keep the budget counting toward the cap) from "the
    // failure cleared because the user composed and sent a genuinely fresh
    // message, bypassing Retry" (a new episode — reset the budget). See the
    // effect's comment for why this replaces the previous ControllerStatus-
    // based check (reagent P1 on #1987).
    let selfInitiatedClear = false;

    const cancelCountdown = () => {
        if (countdown) clearInterval(countdown);
        countdown = undefined;
        setAutoRetryIn(null);
    };

    const disarmTakeover = () => {
        if (disarmTimer) clearTimeout(disarmTimer);
        disarmTimer = undefined;
        setTakeoverArmed(false);
    };

    // ReAgent P2 on #3838: copyState is "same class as expanded/retrying" per
    // its own doc comment, so it must reset everywhere those do — otherwise
    // a Copy click on failure A can leave a stale "Copied"/"Copy failed"
    // label on failure B if B arrives within the 2s window.
    const resetCopyState = () => {
        clearTimeout(copyResetTimer);
        copyResetTimer = undefined;
        setCopyState("idle");
    };

    const clear = (reason: FailureClearReason = "dismissed") => {
        cancelCountdown();
        disarmTakeover();
        clearFailure(opts.model, { reason });
        setExpanded(false);
        setRetrying(false);
        resetCopyState();
    };

    // End the failure *episode*: clear the row AND restore the auto-retry
    // budget. Used when the user explicitly dismisses or a genuinely new turn
    // resolves the failure — NOT by `doRetry` (an auto-retry is still part of
    // the same episode and must keep counting toward the cap, else a
    // persistently-throttled turn would auto-retry forever). Spec §6.
    const endEpisode = () => {
        autoRetries = 0;
        clear("episode-ended");
    };

    const doRetry = () => {
        // This turn's retry is already on its way: nothing more to send.
        if (opts.reconnect?.retryBusy()) return;
        cancelCountdown();
        setRetrying(true);
        selfInitiatedClear = true;
        // A failed turn the recovery tracks is retried there: once, with its attachments.
        if (!opts.reconnect?.retry()) opts.onRetry();
        // The next turn's lifecycle clears the row (a new turn = no failure);
        // also clear locally so the banner goes away immediately. Keep
        // `autoRetries` — an auto-fired retry stays within the episode's cap
        // (the transition-effect below sees `selfInitiatedClear` and skips
        // the reset it would otherwise apply).
        clear("retried");
    };

    const armAutoRetry = () => {
        if (autoRetries >= AUTO_RETRY_BACKOFF_S.length) return; // capped → manual only
        const seconds = jitteredBackoffSeconds(AUTO_RETRY_BACKOFF_S[autoRetries]);
        autoRetries += 1;
        const retryAt = Date.now() + seconds * 1000;
        setAutoRetryIn(seconds);
        countdown = setInterval(() => {
            const left = Math.ceil((retryAt - Date.now()) / 1000);
            if (left <= 0) {
                doRetry();
            } else {
                setAutoRetryIn(left);
            }
        }, 1000);
    };

    onMount(() => {
        // P1.2 — Seed from persisted block meta so the recovery banner survives
        // tab switches and page reloads. Read once on mount; the MPS event
        // subscription below handles live updates for the current session.
        // (SPEC_AGENT_ERROR_FRAMEWORK_2026_06_20 §4 P1.2)
        const mountedAt = Date.now();
        const persistedAtom = getBlockMetaKeyAtom(opts.blockId, "agent:last_failure");
        const pf = persistedAtom();
        // Seed the reducer's canonical `state.failure` (not a local signal —
        // see the module doc comment). The pane is freshly mounted here, so
        // there's no working turn to force-end; FailureObserved's `turnWasEnded`
        // check is false and it just records the failure.
        //
        // Judged before it is shown (spec §3.4): a failure whose cause is over by now is not
        // raised at all. An account failure waits briefly for srv's state of the agent, so a healed
        // one never flashes; a transient one has had no ladder in this pane, so it comes back as a
        // lapsed note, not as a live retry.
        let seedTimer: ReturnType<typeof setTimeout> | undefined;
        // Set once the persisted failure has been judged, or overtaken by a live failure or a new turn.
        let seeded = false;
        const settleSeed = () => {
            seeded = true;
            clearTimeout(seedTimer);
        };
        if (pf) {
            const at = recordedAt(pf, Date.now());
            failureFromMeta = true;
            const seed = () => {
                // A live failure or a turn that began while it waited is newer than the stored one.
                if (opts.failure()) return;
                if (verdictOf({ data: pf, at }) === "healed") return;
                raiseFailure(opts.model, { source: "seed", failure: pf, at, since: mountedAt });
            };
            if (isTransient(pf.code)) setLadderEnded(true);
            if (isAccountCause(pf.code) && opts.liveAccount && opts.liveAccount() === "unknown") {
                // Until srv says, or for SEED_WAIT_MS, whichever comes first.
                const once = () => {
                    if (seeded) return;
                    seeded = true;
                    seed();
                };
                seedTimer = setTimeout(once, SEED_WAIT_MS);
                createEffect(() => {
                    if (opts.liveAccount!() !== "unknown") once();
                });
            } else {
                seed();
            }
        }

        const unsubFailure = onMuxEvent(WpsEvent.AgentFailure, (f) => {
                settleSeed();
                cancelCountdown();
                // A new failure never inherits an armed Take over: its row must
                // start at the first click again (ReAgent P1 on #3742).
                disarmTakeover();
                setExpanded(false);
                setRetrying(false);
                resetCopyState();
                // Reducer-side: records state.failure AND unconditionally ends
                // a still-working turn — see FailureObserved's reducer case.
                failureFromMeta = false;
                raiseFailure(opts.model, { source: "live", failure: f, at: Date.now() });
                // A failure that finds the ladder spent is the end of it: the row becomes the lapsed note.
                setLadderEnded(isTransient(f.code) && autoRetries >= AUTO_RETRY_BACKOFF_S.length);
                if (isTransient(f.code) && !opts.reconnect?.waitsForReconnect(f.code)) armAutoRetry();
            }, MOS.makeORef("block", opts.blockId));

        // Restore the full auto-retry budget once the LAST turn genuinely
        // succeeded — a later unrelated transient failure must get its own full
        // ladder, not inherit a stale count from turns ago. `turn-ended`
        // with outcome "completed" is the reducer's own authoritative verdict
        // (emitted only by the real `TurnEnd` command, driven by the CLI's own
        // session_end/result frame on stdout) — a strictly more reliable
        // signal than the previous approach of inferring success from
        // `ControllerStatus: done` NOT being followed by an `agentfailure`,
        // which depended on the underlying OS process actually exiting between
        // turns. Persistent-mode agents never do (the exact case this PR's
        // FailureObserved fix targets), so that inference could never fire for
        // them — reagent P1 on #1987. A failed turn goes through
        // `FailureObserved` instead (never emits `turn-ended`), so this can
        // never misfire on a failure.
        const unsubTurnEnded = addPaneEventListener((blockId, event) => {
            if (blockId !== opts.blockId) return;
            if (event.type === "turn-started") settleSeed();
            if (event.type === "turn-ended" && event.outcome === "completed") {
                autoRetries = 0;
                setCompletedTurnAt(Date.now());
            }
        });

        onCleanup(() => {
            clearTimeout(seedTimer);
            unsubFailure();
            unsubTurnEnded();
            cancelCountdown();
            disarmTakeover();
            clearTimeout(copyResetTimer);
        });
    });

    const copyError = () => {
        const pf = opts.failure();
        if (!pf) return;
        const f = pf.data;
        const metaParts: string[] = [f.code];
        if (f.signal != null) metaParts.push(`signal ${f.signal}`);
        else if (f.exitCode != null) metaParts.push(`exit ${f.exitCode}`);
        if (f.retryable) metaParts.push("retryable");
        const detailParts: string[] = [];
        if (f.detail) detailParts.push(f.detail);
        if (f.stderrTail) detailParts.push(f.stderrTail);
        const report = formatErrorReport({
            title: f.title || "Agent run failed",
            message: metaParts.join(" · "),
            where: `Agent failure · pane ${opts.blockId.slice(0, 8)}`,
            details: detailParts.length > 0 ? detailParts.join("\n\n") : undefined,
        });
        clearTimeout(copyResetTimer);
        void copyErrorReport(report).then((ok) => {
            setCopyState(ok ? "copied" : "failed");
            copyResetTimer = setTimeout(() => setCopyState("idle"), 2000);
        });
    };

    // Reset the budget when `state.failure` clears WITHOUT this hook having
    // initiated the clear itself — i.e. the user composed and sent a
    // genuinely fresh message while the failure row was still showing,
    // bypassing Retry entirely. `TurnStart` (reducer.ts) unconditionally
    // clears `state.failure` the instant that happens, which this effect
    // observes directly. An auto-fired or manually-clicked Retry ALSO clears
    // `state.failure` (via `clear()`), but must NOT reset the budget — same
    // episode, still bound by the same ladder cap (`armAutoRetry`) — so `doRetry` sets
    // `selfInitiatedClear` first; this effect consumes (and resets) that
    // flag on every transition it observes.
    //
    // This replaces a previous ControllerStatus-based check for the same
    // fresh-message case that fired on the backend's async `running` event —
    // but `TurnStart` clears `state.failure` synchronously, well before that
    // async event round-trips, so the old check never actually saw a
    // non-null failure by the time it ran (reagent P1 on #1987: the check
    // was dead code in practice). Watching the signal transition directly,
    // instead of re-deriving it from a separate, slower async event, is the
    // correct fix — and it works for persistent-mode agents too, unlike the
    // mechanism it replaces.
    let hadFailure = opts.failure() != null;
    createEffect(() => {
        const hasFailureNow = opts.failure() != null;
        if (hadFailure && !hasFailureNow && !selfInitiatedClear) {
            autoRetries = 0;
        }
        // Whatever cleared the row, an armed Take over belongs to that row.
        if (!hasFailureNow) disarmTakeover();
        hadFailure = hasFailureNow;
        selfInitiatedClear = false;
    });

    // First click arms, the second (within TAKEOVER_CONFIRM_MS) runs `run`: both Take over and Move here stop
    // the agent where it is running, so neither fires on one click.
    const confirmed = (run: () => Promise<void>, failedTitle: string) => {
        if (takingOver()) return;
        if (!takeoverArmed()) {
            setTakeoverArmed(true);
            disarmTimer = setTimeout(disarmTakeover, TAKEOVER_CONFIRM_MS);
            return;
        }
        disarmTakeover();
        setTakingOver(true);
        // A resend inside `run` starts a turn, which clears the failure; keep it to bring the row back.
        const before = opts.failure();
        run().then(
            () => {
                setTakingOver(false);
                endEpisode();
            },
            (err: unknown) => {
                setTakingOver(false);
                const pf = opts.failure() ?? before;
                if (!pf) return;
                // Keep the row, but say why it failed: written as the row's own source, so it replaces it.
                raiseFailure(opts.model, {
                    source: sourceOf(pf),
                    failure: {
                        ...pf.data,
                        title: failedTitle,
                        detail: err instanceof Error ? err.message : String(err),
                    },
                    at: Date.now(),
                });
            },
        );
    };
    const takeOver = (turnAttempted: boolean) => {
        const onTakeOver = opts.onTakeOver;
        if (onTakeOver) confirmed(() => onTakeOver(turnAttempted), "Could not take over");
    };
    const moveHere = () => {
        const onMoveHere = opts.onMoveHere;
        if (onMoveHere) confirmed(onMoveHere, "Could not move it here");
    };

    // The failure against what is true now (failure-verdict.ts).
    //
    // srv's account state can lag a failure that just arrived (the agent still reads idle for a moment
    // after an auth failure), so for a failure that arrived in this pane the state counts only once it
    // has changed since. A failure read back at mount is older than the pane, and the state is current.
    let failureFromMeta = false;
    const [accountChangedAt, setAccountChangedAt] = createSignal(0);
    let lastAccount = opts.liveAccount?.() ?? "unknown";
    createEffect(() => {
        const now = opts.liveAccount?.() ?? "unknown";
        if (now === lastAccount) return;
        lastAccount = now;
        setAccountChangedAt(Date.now());
    });
    const verdictOf = (pf: { data: PaneFailure["data"]; at: number; turnAttempted?: boolean }): FailureVerdict => {
        clockTick();
        const stateIsCurrent = failureFromMeta || accountChangedAt() > pf.at;
        return failureVerdict(
            pf,
            {
                account: stateIsCurrent ? (opts.liveAccount?.() ?? "unknown") : "unknown",
                completedTurnAt: completedTurnAt(),
                ladderEnded: ladderEnded(),
            },
            Date.now(),
        );
    };
    const verdict = createMemo((): FailureVerdict | null => {
        const pf = opts.failure();
        return pf ? verdictOf(pf) : null;
    });

    // A usage limit ends when its reset passes, with no event to say so: look again then.
    createEffect(() => {
        const reset = opts.failure()?.data.resetsAtMs;
        if (reset == null) return;
        const wait = reset - Date.now();
        if (wait <= 0 || wait > 2 ** 31 - 1) return;
        const timer = setTimeout(() => setClockTick((n) => n + 1), wait + 1000);
        onCleanup(() => clearTimeout(timer));
    });

    // A healed failure leaves without a click (spec §3.2): the cause is over, and the transcript and the
    // Swarm already show the agent working again. Silent, as the operator decided.
    createEffect(() => {
        if (verdict() === "healed") endEpisode();
    });

    const row = (): FailureRow | null => {
        const pf = opts.failure();
        if (!pf) return null;
        const f = pf.data;
        return failureToRow(
            f,
            {
                expanded: expanded(),
                autoRetryIn: autoRetryIn(),
                retrying: retrying() || (opts.reconnect?.retryBusy() ?? false),
                // Selects the auth arm's "Log in" vs "Login Again" label and
                // the row's accent — see failure-accessory's FailureViewState.
                turnAttempted: pf.turnAttempted,
                bindCandidates: opts.bindCandidates?.(),
                takeoverArmed: takeoverArmed(),
                takingOver: takingOver(),
                copyState: copyState(),
                reconnectHint: opts.reconnect?.reconnectHint() ?? null,
                notSent: f.code === "open_in_other_pane" ? refusedSendOf(opts.blockId)?.text : undefined,
                verdict: verdict() ?? undefined,
                agoLabel: agoLabel(pf.at, Date.now()),
            },
            {
                retry: doRetry,
                copyError,
                // Forwarded the SAME `turnAttempted` the row was built from, so
                // the button's label and its relogin() argument can never
                // disagree about which case this is (a never-started agent must
                // not have an old message re-sent after a successful login).
                loginAgain: () => opts.onLoginAgain(pf.turnAttempted ?? true),
                // Same single-source forwarding as loginAgain above.
                loginViaTerminal: () => opts.onLoginViaTerminal(pf.turnAttempted ?? true),
                openAccounts: opts.onOpenAccounts,
                bindAccount: opts.onBindAccount ?? (() => {}),
                newSession: opts.onNewSession,
                toggleDetails: () => setExpanded((v) => !v),
                dismiss: () => {
                    // Dismissed is done with: don't read a network failure back
                    // on the next mount, where it would be offered again.
                    // Network only: srv reads the usage-limit and sign-in
                    // classes to route work by account.
                    if (f.code === "network") {
                        setBlockMeta(opts.blockId, { "agent:last_failure": null }).catch(() => {});
                    }
                    // Before the row clears: a clear the recovery didn't hear
                    // about first reads as the operator's send, not a dismiss.
                    opts.reconnect?.failureDismissed();
                    // A message this pane refused for another pane has nobody to send it now: it goes
                    // back to the composer. Any other row is not about it, so a kept one is stale.
                    if (f.code === "open_in_other_pane") void restoreRefusedSend(opts.blockId);
                    else forgetRefusedSend(opts.blockId);
                    endEpisode();
                },
                takeOver: opts.onTakeOver ? () => takeOver(pf.turnAttempted ?? true) : undefined,
                goThere: opts.onGoThere && f.holderBlockId ? () => opts.onGoThere!(f.holderBlockId!) : undefined,
                moveHere: opts.onMoveHere ? moveHere : undefined,
            },
        );
    };

    return { row, verdict };
}
