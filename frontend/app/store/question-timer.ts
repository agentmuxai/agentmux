// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * One timer per pending AskUserQuestion, shown wherever a countdown is: the
 * question panel ("Auto-selects recommended in 23s") and the Swarm row's chip
 * ("question 23s"). Both read `questionCountdown()`; neither keeps a timer of
 * its own, so the two can't disagree.
 *
 * Owner side (the agent pane holding the question): `startQuestionTimer`,
 * `noteQuestionActivity`, `setQuestionTimerDormant`, `endQuestionTimer`, and
 * `releaseQuestionTimer` on unmount. Only
 * the owner schedules the expiry, so a question can't be auto-answered twice.
 *
 * Reader side: `questionTimer` and `questionCountdown`. In the owner's renderer
 * they read the local entry; elsewhere (another window's Swarm) the copy the
 * owner publishes to block meta on every state change, never per tick.
 *
 * Activity (`noteQuestionActivity`) restarts a 15s quiet window: the countdown
 * is paused while the user is active and resumes at the full duration 15s after
 * the last activity. Activity only ever defers the timeout, never cancels it
 * (auto-timeout spec §5.1).
 *
 * docs/specs/SPEC_SWARM_QUESTION_STATE_AND_QUESTION_TIMEOUT_ACTIVITY_2026_10_02.md §2, §4.
 */

import { createSignal } from "solid-js";
import { MOS } from "@/app/store/global";
import { makeORef } from "@/app/store/mos";
import { ObjectService } from "@/app/store/services";

export type QuestionTimerState =
    | { kind: "counting"; endsAt: number }
    | { kind: "paused"; reason: "activity" | "dormant" }
    | null;

export type CountdownBand = "default" | "warning" | "critical";

export interface QuestionCountdown {
    seconds: number;
    band: CountdownBand;
    paused: boolean;
}

/** Block meta key the owner publishes its state under; `null` removes it. */
export const META_QUESTION_TIMER = "term:question_timer";

/** How long after the last activity the countdown resumes. */
export const QUESTION_QUIET_MS = 15_000;

interface Owner {
    durationMs: number;
    onExpire: () => void;
    publish: boolean;
    /** Whether a state was ever written to block meta, so `end` clears it. */
    published: boolean;
    dormant: boolean;
    expireId?: ReturnType<typeof setTimeout>;
    quietId?: ReturnType<typeof setTimeout>;
}

const owners = new Map<string, Owner>();
// Present key = this renderer owns the block's timer; its value wins over the
// published copy.
const [local, setLocal] = createSignal<Record<string, QuestionTimerState>>({});

// ── The shared clock ─────────────────────────────────────────────────────
// One 1s tick per renderer. It runs while some reader is showing a counting
// timer: each read refreshes `lastDemand`, and the tick stops itself once
// nobody has read for a while.
const [now, setNow] = createSignal(Date.now());
let clockId: ReturnType<typeof setInterval> | undefined;
let lastDemand = 0;
const DEMAND_GRACE_MS = 2_500;

function stopClock() {
    if (clockId !== undefined) clearInterval(clockId);
    clockId = undefined;
}

/** (Re)start the tick now, so a fresh countdown reads whole seconds. */
function restartClock() {
    stopClock();
    setNow(Date.now());
    lastDemand = Date.now();
    clockId = setInterval(() => {
        if (Date.now() - lastDemand > DEMAND_GRACE_MS) {
            stopClock();
            return;
        }
        setNow(Date.now());
    }, 1_000);
}

// ── Owner side ───────────────────────────────────────────────────────────

function sameState(a: QuestionTimerState | undefined, b: QuestionTimerState): boolean {
    if (!a || !b) return a === b;
    if (a.kind === "counting" && b.kind === "counting") return a.endsAt === b.endsAt;
    if (a.kind === "paused" && b.kind === "paused") return a.reason === b.reason;
    return false;
}

// One write in flight per block, then the latest state: separate requests can
// commit out of order, and a late `null` from the question that just ended
// would wipe the next question's countdown in other windows (Codex P2, #4250).
const writes = new Map<string, { next?: { state: QuestionTimerState } }>();

function writeMeta(blockId: string, state: QuestionTimerState) {
    const queued = writes.get(blockId);
    if (queued) {
        queued.next = { state };
        return;
    }
    const q: { next?: { state: QuestionTimerState } } = { next: { state } };
    writes.set(blockId, q);
    void (async () => {
        while (q.next) {
            const { state: latest } = q.next;
            q.next = undefined;
            try {
                await ObjectService.UpdateObjectMeta(makeORef("block", blockId), {
                    [META_QUESTION_TIMER]: latest,
                } as MetaType);
            } catch (e) {
                console.log("question-timer: meta write failed", e);
            }
        }
        writes.delete(blockId);
    })();
}

function setState(blockId: string, owner: Owner | undefined, state: QuestionTimerState) {
    if (blockId in local() && sameState(local()[blockId], state)) return;
    setLocal((prev) => ({ ...prev, [blockId]: state }));
    if (owner?.publish) {
        owner.published = true;
        writeMeta(blockId, state);
    }
}

function clearTimers(owner: Owner) {
    if (owner.expireId !== undefined) clearTimeout(owner.expireId);
    if (owner.quietId !== undefined) clearTimeout(owner.quietId);
    owner.expireId = owner.quietId = undefined;
}

/** Stop owning the block's timer. The local entry goes too, so this renderer
 *  reads whatever a later owner (the pane moved to another window) publishes
 *  (Codex P2, #4250). `clear` also removes the published copy: only when the
 *  question is over, never on an unmount (see `releaseQuestionTimer`). */
function release(blockId: string, owner: Owner, clear: boolean) {
    clearTimers(owner);
    owners.delete(blockId);
    setLocal((prev) => {
        const rest = { ...prev };
        delete rest[blockId];
        return rest;
    });
    if (clear && owner.published) writeMeta(blockId, null);
}

function expire(blockId: string, owner: Owner) {
    release(blockId, owner, true);
    owner.onExpire();
}

/** Count down from the full duration, or stay paused while dormant. */
function arm(blockId: string, owner: Owner) {
    clearTimers(owner);
    if (owner.dormant) {
        setState(blockId, owner, { kind: "paused", reason: "dormant" });
        return;
    }
    const endsAt = Date.now() + owner.durationMs;
    owner.expireId = setTimeout(() => expire(blockId, owner), owner.durationMs);
    restartClock();
    setState(blockId, owner, { kind: "counting", endsAt });
}

/**
 * Start (or restart, for the next question in the queue) the timer for
 * `blockId`. `publish` writes the state to block meta for other windows; leave
 * it off for a key that isn't a real block.
 */
export function startQuestionTimer(
    blockId: string,
    opts: { durationMs: number; onExpire: () => void; dormant?: boolean; publish?: boolean }
): void {
    const prev = owners.get(blockId);
    if (prev) clearTimers(prev);
    const owner: Owner = {
        durationMs: opts.durationMs,
        onExpire: opts.onExpire,
        publish: opts.publish ?? false,
        published: prev?.published ?? false,
        dormant: opts.dormant ?? false,
    };
    owners.set(blockId, owner);
    arm(blockId, owner);
}

/** The user did something: pause, and resume at the full duration
 *  `QUESTION_QUIET_MS` after the last call. Ignored while dormant. */
export function noteQuestionActivity(blockId: string): void {
    const owner = owners.get(blockId);
    if (!owner || owner.dormant) return;
    clearTimers(owner);
    owner.quietId = setTimeout(() => {
        owner.quietId = undefined;
        arm(blockId, owner);
    }, QUESTION_QUIET_MS);
    setState(blockId, owner, { kind: "paused", reason: "activity" });
}

/** The pane tab went to the background (or came back). Waking re-arms at the
 *  full duration. */
export function setQuestionTimerDormant(blockId: string, dormant: boolean): void {
    const owner = owners.get(blockId);
    if (!owner || owner.dormant === dormant) return;
    owner.dormant = dormant;
    arm(blockId, owner);
}

/**
 * The question was answered or cancelled. Also called by a panel that mounts
 * with nothing pending: with `publish`, it then clears a key a crashed owner
 * left in block meta.
 */
export function endQuestionTimer(blockId: string, opts?: { publish?: boolean }): void {
    const owner = owners.get(blockId);
    if (owner) {
        release(blockId, owner, true);
        return;
    }
    if (opts?.publish && publishedState(blockId) !== undefined) writeMeta(blockId, null);
}

/**
 * The panel unmounted with its question possibly still pending. Stops the
 * local timer and writes nothing: whichever window mounts the pane next writes
 * its own state on mount, so a pane moving between windows has one writer and
 * no `null` from the old window can land after the new countdown (Codex P2,
 * #4250). A key left with no owner at all is covered by the readers' stale
 * rules (spec §2.2).
 */
export function releaseQuestionTimer(blockId: string): void {
    const owner = owners.get(blockId);
    if (owner) release(blockId, owner, false);
}

/**
 * Re-publish the owner's state when block meta says something else and none of
 * this renderer's own writes is in flight. Called from a reactive scope, so it
 * re-runs when the meta changes. Covers a write the previous window already had
 * in flight when the pane moved here: nothing can order that request against
 * ours, so if it lands last, this corrects it (Codex P2, #4250).
 */
export function reconcileQuestionTimer(blockId: string): void {
    const meta = MOS.getMuxObjectAtom<Block>(`block:${blockId}`)()?.meta;
    const owner = owners.get(blockId);
    if (!owner?.published || writes.has(blockId)) return;
    const mine = local()[blockId];
    if (mine === undefined) return;
    const theirs = meta?.[META_QUESTION_TIMER];
    if (!sameState(isState(theirs) ? theirs : null, mine)) writeMeta(blockId, mine);
}

// ── Reader side ──────────────────────────────────────────────────────────

function isState(v: unknown): v is NonNullable<QuestionTimerState> {
    if (!v || typeof v !== "object") return false;
    const s = v as { kind?: unknown; endsAt?: unknown; reason?: unknown };
    if (s.kind === "counting") return typeof s.endsAt === "number";
    return s.kind === "paused" && (s.reason === "activity" || s.reason === "dormant");
}

/** The copy in block meta, or `undefined` when there is none. */
function publishedState(blockId: string): QuestionTimerState | undefined {
    const v = MOS.getMuxObjectAtom<Block>(`block:${blockId}`)()?.meta?.[META_QUESTION_TIMER];
    return isState(v) ? v : undefined;
}

/** The timer's state for `blockId`, reactive. */
export function questionTimer(blockId: string): QuestionTimerState {
    const own = local();
    if (blockId in own) return own[blockId];
    return publishedState(blockId) ?? null;
}

/** Seconds left and their colour band (auto-timeout spec §2.4). */
export function countdownBand(seconds: number): CountdownBand {
    if (seconds <= 5) return "critical";
    if (seconds <= 10) return "warning";
    return "default";
}

/**
 * How the countdown reads now: seconds, band, or paused. `null` when nothing is
 * counting, including a published `counting` whose `endsAt` has passed (left
 * by an owner that crashed), so a reader never shows `0s` or a negative.
 */
export function questionCountdown(blockId: string): QuestionCountdown | null {
    const state = questionTimer(blockId);
    if (!state) return null;
    if (state.kind === "paused") return { seconds: 0, band: "default", paused: true };
    lastDemand = Date.now();
    if (clockId === undefined) restartClock();
    const seconds = Math.ceil((state.endsAt - now()) / 1000);
    if (seconds <= 0) return null;
    return { seconds, band: countdownBand(seconds), paused: false };
}

/** Test-only: drop every timer and the clock. */
export function resetQuestionTimersForTests(): void {
    for (const owner of owners.values()) clearTimers(owner);
    owners.clear();
    writes.clear();
    stopClock();
    setLocal({});
}
