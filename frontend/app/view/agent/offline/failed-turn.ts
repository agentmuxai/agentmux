// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A turn that failed because the model API couldn't be reached, once the API
 * answers again (docs/specs/SPEC_OFFLINE_RESILIENCE_2026_10_20.md §4.3).
 *
 * For a provider whose API srv tracks (Claude's always), a network failure
 * waits for the API instead of the failure row's timed retry, unless
 * `agent:retryonreconnect` is off: then the row's timed retry stays. When the API is
 * back, the turn is retried once by itself if nothing argues against it: the
 * operator started the turn from this pane while it was open (a failure read
 * back on mount, from `agent:last_failure` or the event's replay, may have
 * been dismissed before), no tool ran in it (a retry would run the tool
 * again), it isn't itself an automatic retry that failed, the operator hasn't
 * typed in the pane since (their message goes first, and the turn is offered
 * under the composer), and `agent:retryonreconnect` is on. Otherwise the
 * failure row's Retry is the offer.
 *
 * Every retry of the turn goes through here: it re-sends the message with its
 * attachments, and stores the attempt before it goes, so a second click, a
 * click during the automatic one, or a remount sends nothing more. One whose
 * answer never comes stays possibly sent: once the pane is idle, the failure
 * row comes back with its Retry and says the retry may have gone already,
 * unless the turn it started completed, which settles it as sent. Dismissing the row drops
 * the turn.
 *
 * {@link createFailedTurnRecovery} holds the rules over injected inputs, so
 * tests drive it with plain signals; `useOfflineInput.ts` wires the real ones.
 */

import { type Accessor, createEffect, createSignal, onCleanup, type Setter, untrack } from "solid-js";
import type { PaneFailure } from "@/app/store/agent-pane-state/types";
import type { AgentFailure } from "@/types/rpc/AgentFailure";
import type { AttachmentRef } from "@/types/rpc/AttachmentRef";
import type { Reachability } from "@/types/rpc/Reachability";
import { isTurnNote } from "../../../agent-model/cli-wrappers";
import type { DocumentNode, UserMessageNode } from "../../../agent-model/types";
import { type DeliveryOutcome, newNoticeId, type OfflineInputDeps, type OfflineNotice, UNKNOWN_HOLD_MS } from "./offline-input";

/** A message as a retry re-sends it. */
export interface TurnMessage {
    text: string;
    /** The attachments it was sent with that can still be sent: an entry
     *  without an id was no longer available then either. */
    attachments?: AttachmentRef[];
    /** It was AgentMux's own message (the start-up document), not the person's. */
    system?: boolean;
}

export function turnMessage(node: UserMessageNode): TurnMessage | null {
    const attachments = (node.attachments ?? []).flatMap((a) => (a.id ? [{ id: a.id, name: a.name }] : []));
    if (!node.message && attachments.length === 0) return null;
    return { text: node.message, ...(attachments.length > 0 ? { attachments } : {}), ...(node.system ? { system: true } : {}) };
}

/** The last message sent in the pane, as the failure row's Retry re-sends it. */
export function lastUserMessage(doc: readonly DocumentNode[]): TurnMessage | null {
    for (let i = doc.length - 1; i >= 0; i--) {
        const node = doc[i];
        if (node.type === "user_message") return turnMessage(node);
    }
    return null;
}

/** The failed turn as a retry would re-send it. */
export interface FailedTurnSnapshot {
    /** The operator's message that started it, or null when something else
     *  did (another agent, the Swarm Supervisor, the startup context). */
    message: TurnMessage | null;
    /** A tool ran after that message. */
    toolRan: boolean;
    /** When that message was sent, when the document knows. */
    sentAt?: number;
}

/** Read the turn off the document: what started it, and whether a tool ran after. */
export function failedTurnSnapshot(doc: readonly DocumentNode[]): FailedTurnSnapshot {
    let toolRan = false;
    for (let i = doc.length - 1; i >= 0; i--) {
        const node = doc[i];
        if (node.type === "tool" || node.type === "shell") toolRan = true;
        else if (node.type === "jekt_message" || node.type === "agent_message") {
            // An outgoing one is the agent's own tool call; an incoming one started the turn.
            if (node.direction === "outgoing") toolRan = true;
            else return { message: null, toolRan };
        } else if (isTurnNote(node)) {
            // A background task's notice, a schedule, a Supervisor turn or an older record started the turn, not the operator.
            return { message: null, toolRan, ...(node.timestamp ? { sentAt: node.timestamp } : {}) };
        } else if (node.type === "user_message") {
            const operator = !node.isStartup && !node.supervisor;
            return {
                message: operator ? turnMessage(node) : null,
                toolRan,
                ...(node.timestamp ? { sentAt: node.timestamp } : {}),
            };
        }
    }
    return { message: null, toolRan };
}

/** The latest retry of a failed turn, stored before it goes. */
export interface RetryMark {
    text: string;
    /** When it was sent, in ms since the epoch. */
    at: number;
    /** AgentMux sent it by itself, not the operator's Retry. */
    auto: boolean;
    /** `sending`: no answer yet; `sent`: srv took it; `unknown`: no answer
     *  came, so it may have gone. */
    state: "sending" | "sent" | "unknown";
}

export interface RetryMarkStorage {
    /** Shared by every mount of the pane. Reactive. One a previous load
     *  stored as `sending` reads `unknown`: nothing is sending it any more. */
    mark(): RetryMark | null;
    saveMark(mark: RetryMark | null): void;
}

export type ReconnectDecision =
    | { kind: "retry" }
    | { kind: "offer"; reason: "nothing_to_resend" | "typed" | "read_back" | "tool_ran" | "setting_off" | "retried_once" };

/** What to do with a network-failed turn once its API answers again. */
export function decideOnReconnect(input: {
    turn: FailedTurnSnapshot;
    typedSince: boolean;
    /** The operator started it from this pane while it was open: not read
     *  back on mount, nor started by anything else. */
    live: boolean;
    autoRetryOn: boolean;
    mark: RetryMark | null;
}): ReconnectDecision {
    const message = input.turn.message;
    if (!message) return { kind: "offer", reason: "nothing_to_resend" };
    if (input.typedSince) return { kind: "offer", reason: "typed" };
    if (!input.live) return { kind: "offer", reason: "read_back" };
    if (input.turn.toolRan) return { kind: "offer", reason: "tool_ran" };
    if (!input.autoRetryOn) return { kind: "offer", reason: "setting_off" };
    // An automatic retry that failed, or one whose fate isn't known.
    const mark = input.mark;
    if (mark?.text === message.text && (mark.auto || mark.state !== "sent")) return { kind: "offer", reason: "retried_once" };
    return { kind: "retry" };
}

/** A turn this pane started: its send, its queue, or a hidden memory turn. */
export interface TurnStarted {
    at: number;
    /** The message it started with. */
    content: string;
    /** Not the operator's message (a memory re-injection). */
    hidden: boolean;
}

export interface FailedTurnDeps {
    /** The pane's failure (`state.failure`). */
    failure: Accessor<PaneFailure | null>;
    /** The last turn this mount of the pane started. A failure that follows
     *  one, with the same message, is the operator's turn seen live; any
     *  other was read back, replayed, or started by something else. */
    started: Accessor<TurnStarted | null>;
    /** srv tracks the provider's API ({@link isTrackedService}), decided
     *  without a status: Claude's always counts. */
    tracked: Accessor<boolean>;
    api: Accessor<Reachability | null>;
    /** The pane's turn is retrying an API request. */
    retrying: Accessor<boolean>;
    ready: Accessor<boolean>;
    working: Accessor<boolean>;
    /** Held messages not sent yet: typed since, and they go first. */
    held: Accessor<number>;
    document: () => readonly DocumentNode[];
    autoRetryOn: Accessor<boolean>;
    deliver: OfflineInputDeps["deliver"];
    /** Put the failure back on the row: a retry didn't go, or its answer never came. */
    restoreFailure: (failure: PaneFailure) => void;
    /** Take it off: a retry whose answer was late did go. */
    clearFailure: () => void;
    storage: RetryMarkStorage;
    /** The pane's send chain, shared with held input. */
    lock: OfflineInputDeps["lock"];
    now?: () => number;
    /** The pause between the API answering and an automatic retry: a single
     *  answer can still be a flap, and a fleet shouldn't all retry on the
     *  same second. */
    settleMs?: () => number;
    clockTime?: (ms: number) => string;
}

export interface FailedTurnRecovery {
    /** What it did by itself, and the offered retry. */
    notices: Accessor<OfflineNotice[]>;
    dismissNotice: (id: number) => void;
    /** The failure row arms no timed retry for this: retrying into an
     *  unreachable API only waits out another round of the CLI's retries.
     *  Only with the retry on reconnect on: with it off, the row's timed
     *  retry is what retries a network failure by itself. */
    waitsForReconnect: (code: AgentFailure["code"]) => boolean;
    /** For the failure row's meta line. */
    reconnectHint: Accessor<string | null>;
    /** The failure row's Retry, for the turn this tracks. False when it
     *  tracks none or has nothing to re-send: the row's own retry runs. */
    retry: () => boolean;
    /** A retry of the turn is on its way: Retry sends nothing more. */
    retryBusy: Accessor<boolean>;
    /** The operator dismissed the failure row: nothing is retried or offered. */
    failureDismissed: () => void;
    /** The operator sent something from the composer. */
    typed: () => void;
    /** A turn completed in the pane: a retry it started went. */
    turnCompleted: () => void;
    /** A live failure waits for the API, or a retry is on its way: a reload would lose it. */
    pending: Accessor<boolean>;
}

interface FailedTurn {
    failure: PaneFailure;
    live: boolean;
    typedSince: boolean;
    snapshot: FailedTurnSnapshot | null;
    /** Retried or offered: never decided again. */
    decided: boolean;
    /** Offered under the composer: the operator's send took the row away. */
    offered: boolean;
    /** The retry an earlier mount had on its way when this one read the failure back. */
    priorAttempt?: number;
}

const defaultSettleMs = () => 2_000 + Math.floor(Math.random() * 3_000);

const defaultClock = (ms: number) => new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false });

/** What one recovery's functions share: its inputs, the turn it tracks and its timers. */
interface RecoveryState {
    deps: FailedTurnDeps;
    now: () => number;
    clock: (ms: number) => string;
    turn: Accessor<FailedTurn | null>;
    setTurn: Setter<FailedTurn | null>;
    notices: Accessor<OfflineNotice[]>;
    setNotices: Setter<OfflineNotice[]>;
    offerId: number;
    disposed: boolean;
    settleTimer: ReturnType<typeof setTimeout> | null;
    /** The API has been not checked for longer than {@link UNKNOWN_HOLD_MS}:
     *  its status isn't reaching the pane, which mustn't keep the turn
     *  waiting for good (held input stops waiting on it too). */
    unknownTooLong: Accessor<boolean>;
    setUnknownTooLong: Setter<boolean>;
    unknownTimer: ReturnType<typeof setTimeout> | null;
    /** When the last failure this mount saw landed: a turn started before it isn't the next one's. */
    lastFailureAt: number;
    /** A retry that didn't go, or whose answer never came, waits here for the pane to be idle. */
    putBackDue: Accessor<FailedTurn | null>;
    setPutBackDue: Setter<FailedTurn | null>;
}

function watchUnknownApi(r: RecoveryState): void {
    createEffect(() => {
        const unknown = r.deps.api() === "unknown";
        if (r.unknownTimer) clearTimeout(r.unknownTimer);
        r.unknownTimer = null;
        r.setUnknownTooLong(false);
        if (unknown) r.unknownTimer = setTimeout(() => r.setUnknownTooLong(true), UNKNOWN_HOLD_MS);
    });
}

function answers(r: RecoveryState): boolean {
    return r.deps.api() === "online" || (r.deps.api() === "unknown" && r.unknownTooLong());
}

function flying(r: RecoveryState): boolean {
    return r.deps.storage.mark()?.state === "sending";
}

function snapshotOf(r: RecoveryState, t: FailedTurn): FailedTurnSnapshot {
    return t.snapshot ?? failedTurnSnapshot(r.deps.document());
}

function isTurn(t: FailedTurn | null, at: number): boolean {
    return t?.failure.at === at;
}

function notice(r: RecoveryState, text: string): void {
    r.setNotices((list) => [...list, { id: newNoticeId(), text }]);
}

function which(r: RecoveryState, snap: FailedTurnSnapshot): string {
    return snap.sentAt ? `your message from ${r.clock(snap.sentAt)}` : "your last message";
}

/** A retry of this message is on its way, or its answer never came. */
function unsure(r: RecoveryState, snap: FailedTurnSnapshot): "sending" | "unknown" | null {
    const m = r.deps.storage.mark();
    return m && m.state !== "sent" && m.text === snap.message?.text ? m.state : null;
}

    // A network failure from a tracked API starts waiting; any other failure
    // ends the wait, and so does the row going away, unless the operator's
    // own send or a retry took it (the turn is then offered, or settled).
function watchFailure(r: RecoveryState): void {
    createEffect(() => {
        const f = r.deps.failure();
        const tracked = r.deps.tracked();
        untrack(() => {
            const t = r.turn();
            if (!f) {
                if (!t || t.offered || flying(r)) return;
                if (!t.typedSince) r.setTurn(null);
                // Its Retry went with the row: decide again, to offer it once their message has gone.
                else if (t.decided) r.setTurn({ ...t, decided: false });
                return;
            }
            const since = r.lastFailureAt;
            // One put back after a retry keeps its own, earlier time.
            if (f.at > r.lastFailureAt) r.lastFailureAt = f.at;
            if (f.data.code !== "network" || !tracked) {
                r.setTurn(null);
                return;
            }
            if (isTurn(t, f.at)) return;
            const mark = r.deps.storage.mark();
            const started = r.deps.started();
            const snapshot = r.deps.ready() ? failedTurnSnapshot(r.deps.document()) : null;
            r.setTurn({
                failure: f,
                live:
                    !!started &&
                    !started.hidden &&
                    started.at > since &&
                    started.at <= f.at &&
                    !!snapshot &&
                    snapshot.message?.text === started.content,
                typedSince: r.deps.held() > 0,
                snapshot,
                decided: false,
                offered: false,
                ...(mark?.state === "sending" ? { priorAttempt: mark.at } : {}),
            });
        });
    });
}

/** The retry went: the turn is done with, and so is a row put back for it. */
function went(r: RecoveryState, t: FailedTurn, mark: RetryMark, prev: RetryMark | null, byItself: boolean): void {
    if (isTurn(r.turn(), t.failure.at)) r.setTurn(null);
    if (r.deps.failure()?.at === t.failure.at) r.deps.clearFailure();
    if (prev?.text === mark.text && prev.state !== "sent") notice(r, "Sent your message again. It may have gone once already.");
    else if (byItself) notice(r, `Back online: sent ${which(r, snapshotOf(r, t))} again, since its turn failed while the API was unreachable.`);
}

/** It didn't go, or may not have: the row (or the offer) comes back with
 *  its Retry, once the pane is idle. Putting a failure back ends a turn
 *  that is running, and an unanswered retry's own turn may be. */
function putBack(r: RecoveryState, t: FailedTurn): void {
    r.setPutBackDue(t);
}

function watchPutBack(r: RecoveryState): void {
    createEffect(() => {
        const t = r.putBackDue();
        if (!t || r.deps.working()) return;
        untrack(() => {
            r.setPutBackDue(null);
            const cur = r.turn();
            if (r.disposed || !isTurn(cur, t.failure.at) || cur!.offered || r.deps.failure()) return;
            // Settled while the turn ran: a late answer, the transcript, or the turn completing.
            const m = r.deps.storage.mark();
            if (m?.state === "sent" && m.text === snapshotOf(r, t).message?.text) return;
            r.deps.restoreFailure(t.failure);
        });
    });
}

function isCurrent(r: RecoveryState, mark: RetryMark): boolean {
    const m = r.deps.storage.mark();
    return m?.at === mark.at && m.text === mark.text;
}

function settle(r: RecoveryState, t: FailedTurn, mark: RetryMark, prev: RetryMark | null, byItself: boolean, outcome: DeliveryOutcome): void {
    // Already settled: the turn it started completed first.
    if (!isCurrent(r, mark) || r.deps.storage.mark()?.state === "sent") return;
    if (outcome === "accepted") {
        r.deps.storage.saveMark({ ...mark, state: "sent" });
        went(r, t, mark, prev, byItself);
        return;
    }
    // Rejected: this one didn't go; an earlier one may have.
    r.deps.storage.saveMark(outcome === "rejected" ? prev : { ...mark, state: "unknown" });
    putBack(r, t);
}

/** srv's answer after the send stopped waiting for it. */
function settleLate(r: RecoveryState, t: FailedTurn, mark: RetryMark, prev: RetryMark | null, byItself: boolean, ok: boolean): void {
    if (!isCurrent(r, mark) || r.deps.storage.mark()?.state !== "unknown") return;
    if (!ok) {
        r.deps.storage.saveMark(prev);
        return;
    }
    r.deps.storage.saveMark({ ...mark, state: "sent" });
    went(r, t, mark, prev, byItself);
}

/** Send the turn's message again, once: stored as on its way before it goes. */
async function attempt(r: RecoveryState, t: FailedTurn, auto: boolean): Promise<void> {
    const snapshot = snapshotOf(r, t);
    const message = snapshot.message;
    if (!message || flying(r)) return;
    const prev = r.deps.storage.mark();
    // Retrying a failed automatic retry by hand doesn't earn it another automatic one.
    const wasAuto = prev?.text === message.text && prev.auto;
    const mark: RetryMark = { text: message.text, at: r.now(), auto: auto || wasAuto, state: "sending" };
    r.deps.storage.saveMark(mark);
    const sending = { ...t, snapshot, decided: true };
    r.setTurn(sending);
    const outcome = await r.deps.lock.run(() =>
        r.deps.deliver(message.text, message.attachments, (ok) => settleLate(r, sending, mark, prev, auto, ok))
    );
    settle(r, sending, mark, prev, auto, outcome);
}

function decide(r: RecoveryState, t: FailedTurn): void {
    const snapshot = snapshotOf(r, t);
    const decision = decideOnReconnect({
        turn: snapshot,
        typedSince: t.typedSince,
        live: t.live,
        autoRetryOn: r.deps.autoRetryOn(),
        mark: r.deps.storage.mark(),
    });
    if (decision.kind === "retry") {
        void attempt(r, { ...t, snapshot }, true);
        return;
    }
    // The row still offers Retry unless the operator's own send took it;
    // then the offer is under the composer.
    const offered = decision.reason === "typed" && !r.deps.failure();
    r.setTurn({ ...t, snapshot, decided: true, offered });
}

// Decide once the API answers, the pane is idle and held messages have gone.
function watchSettleDue(r: RecoveryState): void {
    createEffect(() => {
        const t = r.turn();
        const due =
            !!t && !t.decided && answers(r) && !r.deps.retrying() && !r.deps.working() && r.deps.ready() && r.deps.held() === 0 && !flying(r);
        if (r.settleTimer) clearTimeout(r.settleTimer);
        r.settleTimer = null;
        if (!due) return;
        r.settleTimer = setTimeout(() => {
            r.settleTimer = null;
            if (!r.disposed && untrack(r.turn) === t) untrack(() => decide(r, t));
        }, (r.deps.settleMs ?? defaultSettleMs)());
    });
}

// A retry an earlier mount had on its way went: so did this failure.
function watchPriorAttempt(r: RecoveryState): void {
    createEffect(() => {
        const m = r.deps.storage.mark();
        untrack(() => {
            const t = r.turn();
            if (t?.priorAttempt !== undefined && m?.at === t.priorAttempt && m.state === "sent") went(r, t, m, null, m.auto);
        });
    });
}

// One whose answer never came, found in the transcript, did go.
function watchTranscript(r: RecoveryState): void {
    createEffect(() => {
        const m = r.deps.storage.mark();
        if (m?.state !== "unknown" || !r.deps.ready()) return;
        const doc = r.deps.document();
        const found = doc.some((n) => n.type === "user_message" && n.message === m.text && typeof n.timestamp === "number" && n.timestamp >= m.at);
        if (!found) return;
        untrack(() => {
            r.deps.storage.saveMark({ ...m, state: "sent" });
            const t = r.turn();
            if (t && snapshotOf(r, t).message?.text === m.text) went(r, t, m, null, false);
        });
    });
}

function retry(r: RecoveryState): boolean {
    const t = untrack(r.turn);
    const f = untrack(r.deps.failure);
    if (!t || (f && f.at !== t.failure.at)) return false;
    if (untrack(() => flying(r))) return true;
    if (!untrack(() => snapshotOf(r, t)).message) return false;
    void untrack(() => attempt(r, t, false));
    return true;
}

function drop(r: RecoveryState): void {
    r.setTurn(null);
    if (!flying(r)) r.deps.storage.saveMark(null);
}

function offerText(r: RecoveryState, t: FailedTurn): string {
    const s = snapshotOf(r, t);
    const what = which(r, s);
    const base = `Back online. ${what[0].toUpperCase()}${what.slice(1)} failed while the API was unreachable; it wasn't sent again, since you wrote after it.`;
    return unsure(r, s) === "unknown" ? `${base} A retry may have gone already.` : base;
}

/** Not decided yet, and would be retried by itself. */
function willRetry(r: RecoveryState, t: FailedTurn): boolean {
    return (
        !t.decided &&
        decideOnReconnect({
            turn: snapshotOf(r, t),
            typedSince: t.typedSince,
            live: t.live,
            autoRetryOn: r.deps.autoRetryOn(),
            mark: r.deps.storage.mark(),
        }).kind === "retry"
    );
}

function reconnectHint(r: RecoveryState): string | null {
    const t = r.turn();
    if (!t || t.offered) return null;
    const state = unsure(r, snapshotOf(r, t));
    if (state === "sending") return "sending it again";
    if (state === "unknown") return "a retry may have gone already";
    return willRetry(r, t) ? "retries by itself when the API is back" : null;
}

function offeredNotices(r: RecoveryState): OfflineNotice[] {
    const t = r.turn();
    const list = r.notices();
    if (!t?.offered) return list;
    return [...list, { id: r.offerId, text: offerText(r, t), retry: () => void retry(r), retryDisabled: flying(r) }];
}

function dismissNotice(r: RecoveryState, id: number): void {
    if (id === r.offerId) drop(r);
    else r.setNotices((list) => list.filter((n) => n.id !== id));
}

function typed(r: RecoveryState): void {
    // What it did by itself is old news once the operator types.
    r.setNotices([]);
    const t = untrack(r.turn);
    if (t && !t.typedSince) r.setTurn({ ...t, typedSince: true, snapshot: untrack(() => snapshotOf(r, t)) });
    // A new message is a new turn: its failure may be retried, even
    // if it repeats the text an earlier automatic retry sent.
    if (untrack(r.deps.storage.mark)?.state === "sent") r.deps.storage.saveMark(null);
}

function turnCompleted(r: RecoveryState): void {
    const m = untrack(r.deps.storage.mark);
    const s = untrack(r.deps.started);
    // The retry this pane started completed: it went, whatever its answer said.
    if (m && m.state !== "sent" && s && !s.hidden && s.content === m.text && s.at >= m.at) {
        r.deps.storage.saveMark({ ...m, state: "sent" });
        const t = untrack(r.turn);
        if (t && untrack(() => snapshotOf(r, t)).message?.text === m.text) untrack(() => went(r, t, m, null, m.auto));
    }
    // A turn that completed is done with its retry.
    if (untrack(r.deps.storage.mark)?.state === "sent") r.deps.storage.saveMark(null);
}

function pending(r: RecoveryState): boolean {
    const t = r.turn();
    return flying(r) || (!!t && willRetry(r, t));
}

export function createFailedTurnRecovery(deps: FailedTurnDeps): FailedTurnRecovery {
    const [turn, setTurn] = createSignal<FailedTurn | null>(null);
    const [notices, setNotices] = createSignal<OfflineNotice[]>([]);
    const [unknownTooLong, setUnknownTooLong] = createSignal(false);
    const [putBackDue, setPutBackDue] = createSignal<FailedTurn | null>(null);
    const r: RecoveryState = {
        deps,
        now: deps.now ?? Date.now,
        clock: deps.clockTime ?? defaultClock,
        turn,
        setTurn,
        notices,
        setNotices,
        offerId: newNoticeId(),
        disposed: false,
        settleTimer: null,
        unknownTooLong,
        setUnknownTooLong,
        unknownTimer: null,
        lastFailureAt: -Infinity,
        putBackDue,
        setPutBackDue,
    };
    onCleanup(() => {
        r.disposed = true;
        if (r.settleTimer) clearTimeout(r.settleTimer);
        if (r.unknownTimer) clearTimeout(r.unknownTimer);
    });
    watchUnknownApi(r);
    watchFailure(r);
    watchPutBack(r);
    watchSettleDue(r);
    watchPriorAttempt(r);
    watchTranscript(r);

    return {
        notices: () => offeredNotices(r),
        dismissNotice: (id) => dismissNotice(r, id),
        waitsForReconnect: (code) => code === "network" && untrack(r.deps.tracked) && untrack(r.deps.autoRetryOn),
        reconnectHint: () => reconnectHint(r),
        retry: () => retry(r),
        retryBusy: () => !!r.turn() && flying(r),
        failureDismissed: () => drop(r),
        typed: () => typed(r),
        turnCompleted: () => turnCompleted(r),
        pending: () => pending(r),
    };
}
