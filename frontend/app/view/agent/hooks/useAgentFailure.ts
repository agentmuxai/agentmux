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
 * The actual recovery *effects* (re-run the turn, re-auth, open Armory) are
 * passed in by the caller so this hook stays presentation-only otherwise.
 *
 * Auto-retry: for transient classes (rate-limit / overload / network) a
 * countdown arms; clicking Retry fires immediately, reaching 0 fires
 * automatically, Dismiss cancels. Bounded by `AUTO_RETRY_BACKOFF_S` —
 * 5 rungs (5 s → 15 s → 30 s → 60 s → 120 s, each ±20% jittered), ~3.9 min
 * of coverage, then manual-only. See that constant for why the ladder is
 * this long and why it is nonetheless finite. The countdown pauses (not
 * resets) while this agent's own tab is dormant (backgrounded under
 * pane-tab-strip keep-alive) — see `isDormant` below — so a hidden tab never
 * auto-resends a turn nobody can see.
 *
 * Spec: docs/specs/SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16.md §4–§6,
 * docs/specs/SPEC_AGENT_PANE_TAB_KEEPALIVE_2026_09_18.md.
 */

import { createEffect, createSignal, onCleanup, onMount, type Accessor } from "solid-js";
import { muxEventSubscribe } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import * as MOS from "@/app/store/mos";
import { getBlockMetaKeyAtom } from "@/app/store/global";
import { addEventListener as addPaneEventListener } from "@/app/store/agent-pane-state-store";
import type { AgentPaneModel } from "@/app/store/agent-pane-model";
import type { PaneFailure } from "@/app/store/agent-pane-state/types";
import { failureToRow, isTransient, type FailureRow } from "../failure/failure-accessory";

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

export interface UseAgentFailureOptions {
    blockId: string;
    /** Per-pane dispatch handle — default-safe against post-unmount races. */
    model: AgentPaneModel;
    /**
     * True while this agent's own tab is a hidden, kept-alive pane-tab-strip
     * member (`block-component-registry.ts`'s `isBlockDormant`) rather than
     * the one the user is currently looking at. Optional — defaults to
     * "never dormant" for every pre-existing call site/test.
     *
     * Pauses the auto-retry countdown's ticking (see `armAutoRetry` below):
     * a backgrounded tab must not silently re-send a turn to the CLI while
     * nobody can see or cancel it. Ticking resumes exactly where it left off
     * once the tab is visible again — this is a fixed backoff budget, not a
     * "give the user a chance to notice" window like AgentQuestionPanel's
     * auto-timeout, so there's no reason to restart it from scratch. See
     * docs/specs/SPEC_AGENT_PANE_TAB_KEEPALIVE_2026_09_18.md.
     */
    isDormant?: Accessor<boolean>;
    /** Reactive read of the canonical `state.failure` (single source of
     *  truth, set by `FailureObserved` / cleared by `FailureCleared` or the
     *  next `TurnStart`). */
    failure: Accessor<PaneFailure | null>;
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
    /** Open Armory → Accounts. */
    onOpenArmory: () => void;
    /** Start a fresh agent session (context-window overflow recovery). */
    onNewSession: () => void;
    /**
     * Already-authenticated accounts for this agent's provider that could be
     * bound in one click — see `computeAccountBindCandidates`
     * (bind-account-candidates.ts). Reactive: the row re-derives its
     * "Bind: <name>" / "Bind account" vs. "Armory → Accounts" action every
     * time this changes. Optional so callers that haven't wired the account
     * cache yet (e.g. tests) fall back to the pre-existing Armory-only
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
}

export interface UseAgentFailureResult {
    /** The PaneRow descriptor for the current failure, or null when clear. */
    row: Accessor<FailureRow | null>;
}

export function useAgentFailure(opts: UseAgentFailureOptions): UseAgentFailureResult {
    const [expanded, setExpanded] = createSignal(false);
    const [retrying, setRetrying] = createSignal(false);
    const [autoRetryIn, setAutoRetryIn] = createSignal<number | null>(null);
    // Take over's two-step confirmation (`live_elsewhere` only). View-local,
    // like `expanded`: nothing else needs to agree on it.
    const [takeoverArmed, setTakeoverArmed] = createSignal(false);
    const [takingOver, setTakingOver] = createSignal(false);
    let disarmTimer: ReturnType<typeof setTimeout> | undefined;

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

    const clear = () => {
        cancelCountdown();
        disarmTakeover();
        opts.model.dispatchPane({ type: "FailureCleared" });
        setExpanded(false);
        setRetrying(false);
    };

    // End the failure *episode*: clear the row AND restore the auto-retry
    // budget. Used when the user explicitly dismisses or a genuinely new turn
    // resolves the failure — NOT by `doRetry` (an auto-retry is still part of
    // the same episode and must keep counting toward the cap, else a
    // persistently-throttled turn would auto-retry forever). Spec §6.
    const endEpisode = () => {
        autoRetries = 0;
        clear();
    };

    const doRetry = () => {
        cancelCountdown();
        setRetrying(true);
        selfInitiatedClear = true;
        opts.onRetry();
        // The next turn's lifecycle clears the row (a new turn = no failure);
        // also clear locally so the banner goes away immediately. Keep
        // `autoRetries` — an auto-fired retry stays within the episode's cap
        // (the transition-effect below sees `selfInitiatedClear` and skips
        // the reset it would otherwise apply).
        clear();
    };

    const armAutoRetry = () => {
        if (autoRetries >= AUTO_RETRY_BACKOFF_S.length) return; // capped → manual only
        const seconds = jitteredBackoffSeconds(AUTO_RETRY_BACKOFF_S[autoRetries]);
        autoRetries += 1;
        setAutoRetryIn(seconds);
        countdown = setInterval(() => {
            if (opts.isDormant?.()) return; // paused while backgrounded — resumes right where it left off once visible
            const left = (autoRetryIn() ?? 1) - 1;
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
        const persistedAtom = getBlockMetaKeyAtom(opts.blockId, "agent:last_failure");
        const pf = persistedAtom();
        // Seed the reducer's canonical `state.failure` (not a local signal —
        // see the module doc comment). The pane is freshly mounted here, so
        // there's no working turn to force-end; FailureObserved's `turnWasEnded`
        // check is false and it just records the failure.
        if (pf) opts.model.dispatchPane({ type: "FailureObserved", failure: pf, at: Date.now() });

        const unsubFailure = muxEventSubscribe({
            eventType: WpsEvent.AgentFailure,
            scope: MOS.makeORef("block", opts.blockId),
            handler: (event) => {
                const f = (event as any)?.data as AgentFailure | undefined;
                if (!f) return;
                cancelCountdown();
                // A new failure never inherits an armed Take over: its row must
                // start at the first click again (ReAgent P1 on #3742).
                disarmTakeover();
                setExpanded(false);
                setRetrying(false);
                // Reducer-side: records state.failure AND unconditionally ends
                // a still-working turn — see FailureObserved's reducer case.
                opts.model.dispatchPane({ type: "FailureObserved", failure: f, at: Date.now() });
                if (isTransient(f.code)) armAutoRetry();
            },
        });

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
            if (event.type === "turn-ended" && event.outcome === "completed") {
                autoRetries = 0;
            }
        });

        onCleanup(() => {
            unsubFailure();
            unsubTurnEnded();
            cancelCountdown();
            disarmTakeover();
        });
    });

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

    // First click arms, the second (within TAKEOVER_CONFIRM_MS) takes over.
    const takeOver = (turnAttempted: boolean) => {
        if (takingOver() || !opts.onTakeOver) return;
        if (!takeoverArmed()) {
            setTakeoverArmed(true);
            disarmTimer = setTimeout(disarmTakeover, TAKEOVER_CONFIRM_MS);
            return;
        }
        disarmTakeover();
        setTakingOver(true);
        opts.onTakeOver(turnAttempted).then(
            () => {
                setTakingOver(false);
                endEpisode();
            },
            (err: unknown) => {
                setTakingOver(false);
                const pf = opts.failure();
                if (!pf) return;
                // Keep the row, but say why the takeover failed.
                opts.model.dispatchPane({
                    type: "FailureObserved",
                    failure: {
                        ...pf.data,
                        title: "Could not take over",
                        detail: err instanceof Error ? err.message : String(err),
                    },
                    at: Date.now(),
                });
            },
        );
    };

    const row = (): FailureRow | null => {
        const pf = opts.failure();
        if (!pf) return null;
        const f = pf.data;
        return failureToRow(
            f,
            {
                expanded: expanded(),
                autoRetryIn: autoRetryIn(),
                retrying: retrying(),
                // Selects the auth arm's "Log in" vs "Login Again" label and
                // the row's accent — see failure-accessory's FailureViewState.
                turnAttempted: pf.turnAttempted,
                bindCandidates: opts.bindCandidates?.(),
                takeoverArmed: takeoverArmed(),
                takingOver: takingOver(),
            },
            {
                retry: doRetry,
                // Forwarded the SAME `turnAttempted` the row was built from, so
                // the button's label and its relogin() argument can never
                // disagree about which case this is (a never-started agent must
                // not have an old message re-sent after a successful login).
                loginAgain: () => opts.onLoginAgain(pf.turnAttempted ?? true),
                // Same single-source forwarding as loginAgain above.
                loginViaTerminal: () => opts.onLoginViaTerminal(pf.turnAttempted ?? true),
                openArmory: opts.onOpenArmory,
                bindAccount: opts.onBindAccount ?? (() => {}),
                newSession: opts.onNewSession,
                toggleDetails: () => setExpanded((v) => !v),
                dismiss: endEpisode,
                takeOver: opts.onTakeOver ? () => takeOver(pf.turnAttempted ?? true) : undefined,
            },
        );
    };

    return { row };
}
