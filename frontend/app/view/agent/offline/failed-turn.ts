// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A turn that failed because the model API couldn't be reached, once the API
 * answers again (docs/specs/SPEC_OFFLINE_RESILIENCE_2026_10_20.md §4.3).
 *
 * For a provider whose API srv tracks (Claude's always), a network failure
 * waits for the API instead of the failure row's timed retry. When the API is
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
 * answer never comes stays possibly sent: the failure row comes back with its
 * Retry and says the retry may have gone already. Dismissing the row drops
 * the turn.
 *
 * {@link createFailedTurnRecovery} holds the rules over injected inputs, so
 * tests drive it with plain signals; `useOfflineInput.ts` wires the real ones.
 */

import { type Accessor, createEffect, createSignal, onCleanup, untrack } from "solid-js";
import type { PaneFailure } from "@/app/store/agent-pane-state/types";
import type { AgentFailure } from "@/types/rpc/AgentFailure";
import type { AttachmentRef } from "@/types/rpc/AttachmentRef";
import type { Reachability } from "@/types/rpc/Reachability";
import type { DocumentNode, UserMessageNode } from "../types";
import { type DeliveryOutcome, newNoticeId, type OfflineInputDeps, type OfflineNotice, UNKNOWN_HOLD_MS } from "./offline-input";

/** A message as a retry re-sends it. */
export interface TurnMessage {
    text: string;
    /** The attachments it was sent with that can still be sent: an entry
     *  without an id was no longer available then either. */
    attachments?: AttachmentRef[];
}

export function turnMessage(node: UserMessageNode): TurnMessage | null {
    const attachments = (node.attachments ?? []).flatMap((a) => (a.id ? [{ id: a.id, name: a.name }] : []));
    if (!node.message && attachments.length === 0) return null;
    return attachments.length > 0 ? { text: node.message, attachments } : { text: node.message };
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
     *  unreachable API only waits out another round of the CLI's retries. */
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
    /** A turn completed in the pane. */
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

export function createFailedTurnRecovery(deps: FailedTurnDeps): FailedTurnRecovery {
    const now = deps.now ?? Date.now;
    const clock = deps.clockTime ?? defaultClock;
    const [turn, setTurn] = createSignal<FailedTurn | null>(null);
    const [notices, setNotices] = createSignal<OfflineNotice[]>([]);
    const offerId = newNoticeId();
    let disposed = false;
    let settleTimer: ReturnType<typeof setTimeout> | null = null;
    /** The API has been not checked for longer than {@link UNKNOWN_HOLD_MS}:
     *  its status isn't reaching the pane, which mustn't keep the turn
     *  waiting for good (held input stops waiting on it too). */
    const [unknownTooLong, setUnknownTooLong] = createSignal(false);
    let unknownTimer: ReturnType<typeof setTimeout> | null = null;
    /** When the last failure this mount saw landed: a turn started before it isn't the next one's. */
    let lastFailureAt = -Infinity;
    onCleanup(() => {
        disposed = true;
        if (settleTimer) clearTimeout(settleTimer);
        if (unknownTimer) clearTimeout(unknownTimer);
    });

    createEffect(() => {
        const unknown = deps.api() === "unknown";
        if (unknownTimer) clearTimeout(unknownTimer);
        unknownTimer = null;
        setUnknownTooLong(false);
        if (unknown) unknownTimer = setTimeout(() => setUnknownTooLong(true), UNKNOWN_HOLD_MS);
    });
    const answers = () => deps.api() === "online" || (deps.api() === "unknown" && unknownTooLong());

    const flying = () => deps.storage.mark()?.state === "sending";
    const snapshotOf = (t: FailedTurn) => t.snapshot ?? failedTurnSnapshot(deps.document());
    const isTurn = (t: FailedTurn | null, at: number) => t?.failure.at === at;
    const notice = (text: string) => setNotices((list) => [...list, { id: newNoticeId(), text }]);
    const which = (s: FailedTurnSnapshot) => (s.sentAt ? `your message from ${clock(s.sentAt)}` : "your last message");
    /** A retry of this message is on its way, or its answer never came. */
    const unsure = (s: FailedTurnSnapshot) => {
        const m = deps.storage.mark();
        return m && m.state !== "sent" && m.text === s.message?.text ? m.state : null;
    };

    // A network failure from a tracked API starts waiting; any other failure
    // ends the wait, and so does the row going away, unless the operator's
    // own send or a retry took it (the turn is then offered, or settled).
    createEffect(() => {
        const f = deps.failure();
        const tracked = deps.tracked();
        untrack(() => {
            const t = turn();
            if (!f) {
                if (!t || t.offered || flying()) return;
                if (!t.typedSince) setTurn(null);
                // Its Retry went with the row: decide again, to offer it once their message has gone.
                else if (t.decided) setTurn({ ...t, decided: false });
                return;
            }
            const since = lastFailureAt;
            // One put back after a retry keeps its own, earlier time.
            if (f.at > lastFailureAt) lastFailureAt = f.at;
            if (f.data.code !== "network" || !tracked) {
                setTurn(null);
                return;
            }
            if (isTurn(t, f.at)) return;
            const mark = deps.storage.mark();
            const started = deps.started();
            const snapshot = deps.ready() ? failedTurnSnapshot(deps.document()) : null;
            setTurn({
                failure: f,
                live:
                    !!started &&
                    !started.hidden &&
                    started.at > since &&
                    started.at <= f.at &&
                    !!snapshot &&
                    snapshot.message?.text === started.content,
                typedSince: deps.held() > 0,
                snapshot,
                decided: false,
                offered: false,
                ...(mark?.state === "sending" ? { priorAttempt: mark.at } : {}),
            });
        });
    });

    /** The retry went: the turn is done with, and so is a row put back for it. */
    const went = (t: FailedTurn, mark: RetryMark, prev: RetryMark | null, byItself: boolean) => {
        if (isTurn(turn(), t.failure.at)) setTurn(null);
        if (deps.failure()?.at === t.failure.at) deps.clearFailure();
        if (prev?.text === mark.text && prev.state !== "sent") notice("Sent your message again. It may have gone once already.");
        else if (byItself) notice(`Back online: sent ${which(snapshotOf(t))} again, since its turn failed while the API was unreachable.`);
    };

    /** It didn't go, or may not have: the row (or the offer) comes back with its Retry. */
    const putBack = (t: FailedTurn) => {
        const cur = turn();
        if (disposed || !isTurn(cur, t.failure.at) || cur!.offered || deps.failure()) return;
        deps.restoreFailure(t.failure);
    };

    const isCurrent = (mark: RetryMark) => {
        const m = deps.storage.mark();
        return m?.at === mark.at && m.text === mark.text;
    };

    const settle = (t: FailedTurn, mark: RetryMark, prev: RetryMark | null, byItself: boolean, outcome: DeliveryOutcome) => {
        if (!isCurrent(mark)) return;
        if (outcome === "accepted") {
            deps.storage.saveMark({ ...mark, state: "sent" });
            went(t, mark, prev, byItself);
            return;
        }
        // Rejected: this one didn't go; an earlier one may have.
        deps.storage.saveMark(outcome === "rejected" ? prev : { ...mark, state: "unknown" });
        putBack(t);
    };

    /** srv's answer after the send stopped waiting for it. */
    const settleLate = (t: FailedTurn, mark: RetryMark, prev: RetryMark | null, byItself: boolean, ok: boolean) => {
        if (!isCurrent(mark) || deps.storage.mark()?.state !== "unknown") return;
        if (!ok) {
            deps.storage.saveMark(prev);
            return;
        }
        deps.storage.saveMark({ ...mark, state: "sent" });
        went(t, mark, prev, byItself);
    };

    /** Send the turn's message again, once: stored as on its way before it goes. */
    const attempt = async (t: FailedTurn, auto: boolean): Promise<void> => {
        const snapshot = snapshotOf(t);
        const message = snapshot.message;
        if (!message || flying()) return;
        const prev = deps.storage.mark();
        // Retrying a failed automatic retry by hand doesn't earn it another automatic one.
        const wasAuto = prev?.text === message.text && prev.auto;
        const mark: RetryMark = { text: message.text, at: now(), auto: auto || wasAuto, state: "sending" };
        deps.storage.saveMark(mark);
        const sending = { ...t, snapshot, decided: true };
        setTurn(sending);
        const outcome = await deps.lock.run(() =>
            deps.deliver(message.text, message.attachments, (ok) => settleLate(sending, mark, prev, auto, ok))
        );
        settle(sending, mark, prev, auto, outcome);
    };

    const decide = (t: FailedTurn) => {
        const snapshot = snapshotOf(t);
        const decision = decideOnReconnect({
            turn: snapshot,
            typedSince: t.typedSince,
            live: t.live,
            autoRetryOn: deps.autoRetryOn(),
            mark: deps.storage.mark(),
        });
        if (decision.kind === "retry") {
            void attempt({ ...t, snapshot }, true);
            return;
        }
        // The row still offers Retry unless the operator's own send took it;
        // then the offer is under the composer.
        const offered = decision.reason === "typed" && !deps.failure();
        setTurn({ ...t, snapshot, decided: true, offered });
    };

    // Decide once the API answers, the pane is idle and held messages have gone.
    createEffect(() => {
        const t = turn();
        const due =
            !!t && !t.decided && answers() && !deps.retrying() && !deps.working() && deps.ready() && deps.held() === 0 && !flying();
        if (settleTimer) clearTimeout(settleTimer);
        settleTimer = null;
        if (!due) return;
        settleTimer = setTimeout(() => {
            settleTimer = null;
            if (!disposed && untrack(turn) === t) untrack(() => decide(t));
        }, (deps.settleMs ?? defaultSettleMs)());
    });

    // A retry an earlier mount had on its way went: so did this failure.
    createEffect(() => {
        const m = deps.storage.mark();
        untrack(() => {
            const t = turn();
            if (t?.priorAttempt !== undefined && m?.at === t.priorAttempt && m.state === "sent") went(t, m, null, m.auto);
        });
    });

    // One whose answer never came, found in the transcript, did go.
    createEffect(() => {
        const m = deps.storage.mark();
        if (m?.state !== "unknown" || !deps.ready()) return;
        const doc = deps.document();
        const found = doc.some((n) => n.type === "user_message" && n.message === m.text && typeof n.timestamp === "number" && n.timestamp >= m.at);
        if (!found) return;
        untrack(() => {
            deps.storage.saveMark({ ...m, state: "sent" });
            const t = turn();
            if (t && snapshotOf(t).message?.text === m.text) went(t, m, null, false);
        });
    });

    const retry = (): boolean => {
        const t = untrack(turn);
        const f = untrack(deps.failure);
        if (!t || (f && f.at !== t.failure.at)) return false;
        if (untrack(flying)) return true;
        if (!untrack(() => snapshotOf(t)).message) return false;
        void untrack(() => attempt(t, false));
        return true;
    };

    const drop = () => {
        setTurn(null);
        if (!flying()) deps.storage.saveMark(null);
    };

    const offerText = (t: FailedTurn) => {
        const s = snapshotOf(t);
        const what = which(s);
        const base = `Back online. ${what[0].toUpperCase()}${what.slice(1)} failed while the API was unreachable; it wasn't sent again, since you wrote after it.`;
        return unsure(s) === "unknown" ? `${base} A retry may have gone already.` : base;
    };

    /** Not decided yet, and would be retried by itself. */
    const willRetry = (t: FailedTurn) =>
        !t.decided &&
        decideOnReconnect({
            turn: snapshotOf(t),
            typedSince: t.typedSince,
            live: t.live,
            autoRetryOn: deps.autoRetryOn(),
            mark: deps.storage.mark(),
        }).kind === "retry";

    const reconnectHint = (): string | null => {
        const t = turn();
        if (!t || t.offered) return null;
        const state = unsure(snapshotOf(t));
        if (state === "sending") return "sending it again";
        if (state === "unknown") return "a retry may have gone already";
        return willRetry(t) ? "retries by itself when the API is back" : null;
    };

    return {
        notices: () => {
            const t = turn();
            const list = notices();
            if (!t?.offered) return list;
            return [...list, { id: offerId, text: offerText(t), retry: () => void retry(), retryDisabled: flying() }];
        },
        dismissNotice: (id) => {
            if (id === offerId) drop();
            else setNotices((list) => list.filter((n) => n.id !== id));
        },
        waitsForReconnect: (code) => code === "network" && untrack(deps.tracked),
        reconnectHint,
        retry,
        retryBusy: () => !!turn() && flying(),
        failureDismissed: drop,
        typed: () => {
            // What it did by itself is old news once the operator types.
            setNotices([]);
            const t = untrack(turn);
            if (t && !t.typedSince) setTurn({ ...t, typedSince: true, snapshot: untrack(() => snapshotOf(t)) });
            // A new message is a new turn: its failure may be retried, even
            // if it repeats the text an earlier automatic retry sent.
            if (untrack(deps.storage.mark)?.state === "sent") deps.storage.saveMark(null);
        },
        turnCompleted: () => {
            if (untrack(deps.storage.mark)?.state === "sent") deps.storage.saveMark(null);
        },
        pending: () => {
            const t = turn();
            return flying() || (!!t && willRetry(t));
        },
    };
}
