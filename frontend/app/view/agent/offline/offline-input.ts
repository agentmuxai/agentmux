// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a pane does with the operator's input while its model API can't be
 * reached, and with a turn that failed because of it, once it can again
 * (docs/specs/SPEC_OFFLINE_RESILIENCE_2026_10_20.md §4.3, §4.4).
 *
 * **Held input.** A message typed while the pane's turn is retrying an API
 * request, or while srv's connectivity state says the provider's API is
 * offline, isn't sent: it waits under the composer with Send now and Cancel,
 * and goes out by itself, in order and once, when neither holds, one at a
 * time while the pane is idle. It is kept with the pane's block meta until
 * srv accepts it, so a rejected send, a reload or a restart keeps it. A `/`
 * or `!` command is never held.
 *
 * **A turn that failed on the network.** When the provider's API answers
 * again, the turn is retried once by itself if nothing argues against it:
 * the operator hasn't typed in the pane since (their message goes first,
 * and the turn is only offered), no tool ran in it (a retry would run the
 * tool again), it isn't itself an automatic retry that failed, the failure
 * happened while this pane was open (one read back after a reload or a
 * restart is only offered: it may have been dismissed before), and the
 * `agent:retryonreconnect` setting is on.
 *
 * {@link createOfflineInput} holds the rules over injected inputs, so tests
 * drive it with plain signals; `useOfflineInput.ts` wires the real ones.
 */

import { type Accessor, createEffect, createSignal, onCleanup, untrack } from "solid-js";
import type { PaneFailure } from "@/app/store/agent-pane-state/types";
import type { AttachmentRef } from "@/types/rpc/AttachmentRef";
import type { Reachability } from "@/types/rpc/Reachability";
import type { ServiceStatus } from "@/types/rpc/ServiceStatus";
import type { DocumentNode } from "../types";

/** One held message. */
export interface HeldInput {
    id: string;
    text: string;
    /** When it was typed, in ms since the epoch. */
    heldAt: number;
    attachments?: AttachmentRef[];
    /** Sent, but srv's answer never came: it may have gone. Not sent again
     *  by itself; Send now says so. */
    unconfirmed?: boolean;
}

/** The turn an automatic retry re-sent, so its failure isn't retried again. */
export interface AutoRetryMarker {
    text: string;
    at: number;
}

/** What the pane keeps between reloads. */
export interface OfflineStorage {
    /** The held messages, shared by every mount of the pane. Reactive. */
    held(): HeldInput[];
    saveHeld(held: HeldInput[]): void;
    loadMarker(): AutoRetryMarker | null;
    saveMarker(marker: AutoRetryMarker | null): void;
}

/** Runs tasks for one key one after another; `chains` is shared by every
 *  caller that must not overlap. */
export function keyedLock(chains: Map<string, Promise<unknown>>, key: string): OfflineInputDeps["lock"] {
    return {
        run<T>(task: () => Promise<T>): Promise<T> {
            const next = (chains.get(key) ?? Promise.resolve()).then(task, task);
            const tail = next.catch(() => undefined);
            chains.set(key, tail);
            void tail.then(() => {
                if (chains.get(key) === tail) chains.delete(key);
            });
            return next;
        },
    };
}

/** What a pane's send reports about one message, as reducer events. */
export type PaneMessageEvent = { type: string; id?: string };

/** What became of a message sent with {@link confirmedSend}. `unknown`:
 *  no answer in time, so it may or may not have gone. */
export type DeliveryOutcome = "accepted" | "rejected" | "unknown";

/** How long a late answer is still listened for after `unknown`. */
const LATE_ANSWER_MS = 30 * 60_000;

/**
 * `send`, resolved `accepted` only once srv accepted the message (the pane's
 * `pending-accepted` for it, or srv's own `agent-message-accepted`). The
 * send path resolves either way: it turns a failed `AgentInputCommand` or a
 * login guard into `pending-rejected`, and a command it handles itself into
 * no pending entry at all; both are `rejected`. A message sent while a turn
 * runs would only join the pane's own queue, which a reload drops, so it
 * isn't sent. With no answer by `timeoutMs`, or the send path giving up on
 * one (`pending-expired`), it is `unknown`: srv may still have it, and an
 * answer that comes later still reaches `onLate`.
 */
export function confirmedSend(
    send: (text: string, attachments?: AttachmentRef[]) => Promise<void>,
    listen: (cb: (e: PaneMessageEvent) => void) => () => void,
    working: () => boolean,
    timeoutMs = 120_000
): OfflineInputDeps["deliver"] {
    return (text, attachments, onLate) => {
        if (working()) return Promise.resolve("rejected");
        return new Promise<DeliveryOutcome>((resolve) => {
            let id: string | null = null;
            let timedOut = false;
            let done = false;
            let timer: ReturnType<typeof setTimeout>;
            const settle = (accepted: boolean) => {
                if (done) return;
                done = true;
                unsub();
                clearTimeout(timer);
                if (timedOut) onLate?.(accepted);
                else resolve(accepted ? "accepted" : "rejected");
            };
            const giveUp = () => {
                if (timedOut || done) return;
                timedOut = true;
                clearTimeout(timer);
                resolve("unknown");
                timer = setTimeout(() => {
                    done = true;
                    unsub();
                }, LATE_ANSWER_MS);
            };
            // The send path queues its entry synchronously, before any await.
            const unsub = listen((e) => {
                if (e.type === "pending-queued" && id === null) id = e.id ?? null;
                else if (id !== null && e.id === id) {
                    if (e.type === "pending-accepted") settle(true);
                    else if (e.type === "pending-rejected") settle(false);
                    else if (e.type === "pending-expired") giveUp();
                }
            });
            timer = setTimeout(giveUp, timeoutMs);
            send(text, attachments).then(
                () => {
                    if (id === null) settle(false);
                },
                () => settle(false)
            );
        });
    };
}

/**
 * Model APIs srv reports on from the CLI's own frames before any status
 * reaches the pane (`connectivity::ANTHROPIC` in crates/srv/src/connectivity/mod.rs).
 */
const TRACKED_MODEL_APIS = new Set(["model_api:anthropic"]);

/** Whether srv tracks a service, even before the pane has its status. */
export function isTrackedService(service: string | null, services: readonly ServiceStatus[] | undefined): boolean {
    return !!service && (TRACKED_MODEL_APIS.has(service) || !!services?.some((s) => s.service === service));
}

/** srv's connectivity id for a provider's model API, from its first vendor. */
export function modelApiService(supportedVendors: readonly string[] | undefined): string | null {
    const vendor = supportedVendors?.[0];
    return vendor ? `model_api:${vendor}` : null;
}

/** That service's state, or null when srv doesn't track it. */
export function serviceReachability(
    services: readonly ServiceStatus[] | undefined,
    service: string | null
): Reachability | null {
    if (!service) return null;
    return services?.find((s) => s.service === service)?.state ?? null;
}

/** Whether the operator's input waits instead of going out. */
export function holdsInput(retrying: boolean, api: Reachability | null): boolean {
    return retrying || api === "offline";
}

/** A `/` or `!` command runs in the pane (or is the CLI's own), so it is
 *  never held: help, login and shell commands keep working offline. */
export function isLocalCommand(text: string, attachments?: readonly AttachmentRef[]): boolean {
    const t = text.trimStart();
    return !attachments?.length && (t.startsWith("/") || t.startsWith("!"));
}

/** The failed turn as a retry would re-send it. */
export interface FailedTurnSnapshot {
    /** The operator's message that started it, or null when something else
     *  did (a message from another agent), which a retry can't re-send. */
    text: string | null;
    /** A tool ran after that message. */
    toolRan: boolean;
    /** When that message was sent, when the document knows. */
    sentAt?: number;
}

/**
 * Read the turn off the document: the last message that started a turn, and
 * whether a tool ran after it.
 */
export function failedTurnSnapshot(doc: readonly DocumentNode[]): FailedTurnSnapshot {
    let toolRan = false;
    for (let i = doc.length - 1; i >= 0; i--) {
        const node = doc[i];
        if (node.type === "tool") toolRan = true;
        else if (node.type === "jekt_message") return { text: null, toolRan };
        else if (node.type === "user_message") {
            return { text: node.message || null, toolRan, ...(node.timestamp ? { sentAt: node.timestamp } : {}) };
        }
    }
    return { text: null, toolRan };
}

/** A message with the held one's text, sent after it was held. */
export function sentInTranscript(doc: readonly DocumentNode[], item: HeldInput): boolean {
    return doc.some(
        (n) => n.type === "user_message" && n.message === item.text && typeof n.timestamp === "number" && n.timestamp >= item.heldAt
    );
}

export type ReconnectDecision =
    | { kind: "retry" }
    | { kind: "offer"; reason: "typed" | "read_back" | "tool_ran" | "setting_off" | "retried_once" | "nothing_to_resend" };

/** What to do with a network-failed turn once its API answers again. */
export function decideOnReconnect(input: {
    turn: FailedTurnSnapshot;
    typedSince: boolean;
    /** The failure happened while this pane was open, not read back on mount. */
    live: boolean;
    autoRetryOn: boolean;
    marker: AutoRetryMarker | null;
}): ReconnectDecision {
    if (!input.turn.text) return { kind: "offer", reason: "nothing_to_resend" };
    if (input.typedSince) return { kind: "offer", reason: "typed" };
    if (!input.live) return { kind: "offer", reason: "read_back" };
    if (input.turn.toolRan) return { kind: "offer", reason: "tool_ran" };
    if (!input.autoRetryOn) return { kind: "offer", reason: "setting_off" };
    if (input.marker?.text === input.turn.text) return { kind: "offer", reason: "retried_once" };
    return { kind: "retry" };
}

/** "held under a minute", "held 4 min". */
export function heldForText(ms: number): string {
    const min = Math.round(ms / 60_000);
    return min < 1 ? "held under a minute" : `held ${min} min`;
}

/** A line the pane shows after acting by itself, or offering to. */
export interface OfflineNotice {
    id: number;
    text: string;
    /** The offered retry, when there is one. */
    retry?: () => void;
}

export interface OfflineInputDeps {
    /** The pane's turn is retrying an API request. */
    retrying: Accessor<boolean>;
    /** The provider's API as srv sees it; null when srv doesn't track it,
     *  or the pane doesn't have its status yet. */
    api: Accessor<Reachability | null>;
    /** srv tracks the provider's API ({@link isTrackedService}): its
     *  network failures wait for it, even before its status arrives. */
    tracked: Accessor<boolean>;
    /** The pane can send (its history has loaded). */
    ready: Accessor<boolean>;
    /** A turn is running. */
    working: Accessor<boolean>;
    failure: Accessor<PaneFailure | null>;
    document: () => readonly DocumentNode[];
    autoRetryOn: Accessor<boolean>;
    /** Send as if typed, with no hold: the pane's own send path. */
    send: (text: string, attachments?: AttachmentRef[]) => Promise<void>;
    /** The same, saying whether srv accepted it ({@link confirmedSend});
     *  after `unknown`, a late answer goes to `onLate`. */
    deliver: (
        text: string,
        attachments?: AttachmentRef[],
        onLate?: (accepted: boolean) => void
    ) => Promise<DeliveryOutcome>;
    /** Put a failure back on the row, after a retry that didn't go out. */
    restoreFailure: (failure: PaneFailure) => void;
    /** Stop the running turn. */
    stop: () => void;
    storage: OfflineStorage;
    /** One chain per pane, shared by every mount of it, so two never send
     *  the same held message. */
    lock: { run<T>(task: () => Promise<T>): Promise<T> };
    now?: () => number;
    /** The pause between the API answering and an automatic retry: a
     *  single answer can still be a flap, and a fleet shouldn't all retry
     *  on the same second. */
    settleMs?: () => number;
    /** How long Send now waits for the stopped turn to end. */
    stopWaitMs?: number;
    clockTime?: (ms: number) => string;
}

interface FailedTurn {
    failure: PaneFailure;
    at: number;
    /** Seen while this pane was open, after a turn ran in it. */
    live: boolean;
    typedSince: boolean;
    snapshot: FailedTurnSnapshot | null;
}

export interface OfflineInput {
    held: Accessor<HeldInput[]>;
    notices: Accessor<OfflineNotice[]>;
    /** The composer's send: holds while {@link holdsInput} says so, or
     *  while earlier held messages haven't gone yet. */
    submit: (text: string, attachments?: AttachmentRef[]) => Promise<void>;
    /** Stop a running turn, then send everything held, in order, even
     *  while the API looks unreachable. */
    sendNow: () => Promise<void>;
    cancel: (id: string) => void;
    dismissNotice: (id: number) => void;
    /** A network failure this waits on: the failure row arms no timed
     *  retry for it, since retrying into an unreachable API only waits out
     *  another round of the CLI's retries. */
    waitsForReconnect: (code: string) => boolean;
    /** For the failure row, while the failed turn waits for the API. */
    reconnectHint: Accessor<string | null>;
    /** The operator dismissed the failure row: nothing is retried or
     *  offered for that turn, typed since or not. */
    failureDismissed: () => void;
}

const DEFAULT_STOP_WAIT_MS = 5_000;
const STOP_POLL_MS = 100;

const defaultSettleMs = () => 2_000 + Math.floor(Math.random() * 3_000);

const defaultClock = (ms: number) =>
    new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false });

let nextHeldSeq = 0;

export function createOfflineInput(deps: OfflineInputDeps): OfflineInput {
    const now = deps.now ?? Date.now;
    const clock = deps.clockTime ?? defaultClock;
    const held = () => deps.storage.held();
    const [notices, setNotices] = createSignal<OfflineNotice[]>([]);
    const [failed, setFailed] = createSignal<FailedTurn | null>(null);
    /** The held message being sent now: it stays stored until srv takes it. */
    const [inFlight, setInFlight] = createSignal<string | null>(null);
    /** Send now: send while the hold lasts. */
    const [forced, setForced] = createSignal(false);
    /** A held message didn't go out: wait for the operator, or the next outage. */
    const [paused, setPaused] = createSignal(false);
    let nextNoticeId = 0;
    let disposed = false;
    let settleTimer: ReturnType<typeof setTimeout> | null = null;
    /** The failure an automatic retry was decided for: one put back after
     *  that retry didn't go out must not start another. */
    let decidedAt: number | null = null;
    /** Held messages sent since the list was last empty, for one notice. */
    let burst: { noticeId: number; sent: number; oldest: number } | null = null;
    /** A turn ran in this pane since it mounted or since the last failure:
     *  a failure that follows it was seen live, not read back on mount. */
    let workedSinceFailure = false;
    createEffect(() => {
        if (deps.working()) workedSinceFailure = true;
    });
    onCleanup(() => {
        disposed = true;
        if (settleTimer) clearTimeout(settleTimer);
    });

    const holding = () => holdsInput(deps.retrying(), deps.api());
    const notice = (text: string, retry?: () => void) =>
        setNotices((list) => [...list, { id: nextNoticeId++, text, ...(retry ? { retry } : {}) }]);
    const dismissNotice = (id: number) => setNotices((list) => list.filter((n) => n.id !== id));

    const noteSent = (item: HeldInput) => {
        const b = burst ?? { noticeId: nextNoticeId++, sent: 0, oldest: Infinity };
        burst = { ...b, sent: b.sent + 1, oldest: Math.min(b.oldest, item.heldAt) };
        const what = burst.sent === 1 ? "Sent your held message" : `Sent your ${burst.sent} held messages`;
        const text = `${what} (${heldForText(now() - burst.oldest)}).`;
        const id = burst.noticeId;
        setNotices((list) => [...list.filter((n) => n.id !== id), { id, text }]);
    };

    /**
     * Send the oldest held message, and take it off the list only once srv
     * accepted it, so a rejected send, or a reload mid-send, keeps it. One
     * at a time, each when the pane is idle: a message sent during a turn
     * would only join the pane's own queue, which a reload loses.
     */
    const releaseOne = (): Promise<void> =>
        deps.lock.run(async () => {
            const item = untrack(held)[0];
            const isForced = untrack(forced);
            // One that may have gone already waits for the operator, and so
            // does everything after it.
            const blocked = !isForced && (untrack(holding) || !!item?.unconfirmed);
            if (disposed || !item || blocked || untrack(paused) || untrack(deps.working) || !untrack(deps.ready)) return;
            const wasUnconfirmed = !!item.unconfirmed;
            setInFlight(item.id);
            // Stored as possibly gone before it goes: a reload before srv's
            // answer must not send it again.
            markHeld(item.id, { unconfirmed: true });
            const outcome = await deps.deliver(item.text, item.attachments, (accepted) => settleLate(item.id, accepted));
            setInFlight(null);
            if (outcome === "unknown") {
                setForced(false);
                notice("No answer yet for your held message, so it may have gone. It stays here and isn't sent again by itself.");
                return;
            }
            if (outcome === "rejected") {
                // This send didn't go; an earlier one may have.
                markHeld(item.id, { unconfirmed: wasUnconfirmed });
                setPaused(true);
                setForced(false);
                notice("Couldn't send your held message, so it is still held. Send now tries again.");
                return;
            }
            if (wasUnconfirmed) notice("Sent your held message again. It may have gone once already.");
            accepted(item);
        });

    const markHeld = (id: string, patch: Partial<HeldInput>) =>
        deps.storage.saveHeld(untrack(held).map((m) => (m.id === id ? { ...m, ...patch } : m)));

    const accepted = (item: HeldInput) => {
        if (!untrack(held).some((m) => m.id === item.id)) return;
        const rest = untrack(held).filter((m) => m.id !== item.id);
        deps.storage.saveHeld(rest);
        noteSent(item);
        if (rest.length === 0) {
            burst = null;
            setForced(false);
        }
    };

    /** srv's answer for a message that timed out: drop it once accepted, or
     *  hold it as before once rejected. */
    const settleLate = (id: string, ok: boolean) => {
        const item = untrack(held).find((m) => m.id === id);
        if (!item?.unconfirmed) return;
        if (ok) {
            accepted(item);
            return;
        }
        markHeld(id, { unconfirmed: false });
        setPaused(true);
    };

    // One that may have gone and is in the transcript did go: drop it.
    createEffect(() => {
        const unsure = held().filter((m) => m.unconfirmed && m.id !== inFlight());
        if (unsure.length === 0 || !deps.ready()) return;
        const doc = deps.document();
        for (const item of unsure) if (sentInTranscript(doc, item)) untrack(() => accepted(item));
    });

    // Release once nothing holds (the API is reachable and the turn isn't
    // retrying) and the pane is idle; the next goes when that turn ends.
    createEffect(() => {
        const due = held().length > 0 && (forced() || !holding()) && !paused() && deps.ready() && !deps.working();
        if (due && inFlight() === null) void releaseOne();
    });
    // A new outage starts over: what didn't go out last time is tried again.
    createEffect(() => {
        if (holding()) setPaused(false);
    });

    const submit = async (text: string, attachments?: AttachmentRef[]): Promise<void> => {
        if (isLocalCommand(text, attachments)) return deps.send(text, attachments);
        // What the pane did by itself is old news once the operator types;
        // an offered retry stays until they take it or dismiss it.
        setNotices((list) => list.filter((n) => n.retry));
        const turn = untrack(failed);
        if (turn && !turn.typedSince) {
            setFailed({ ...turn, typedSince: true, snapshot: turn.snapshot ?? failedTurnSnapshot(deps.document()) });
        }
        // A new message is a new turn: its failure may be retried, even if
        // it repeats the text an earlier automatic retry sent.
        if (deps.storage.loadMarker()) deps.storage.saveMarker(null);
        if (untrack(holding) || untrack(held).length > 0) {
            const item: HeldInput = {
                id: `held_${now()}_${nextHeldSeq++}`,
                text,
                heldAt: now(),
                ...(attachments?.length ? { attachments } : {}),
            };
            deps.storage.saveHeld([...untrack(held), item]);
            setPaused(false);
            return;
        }
        await deps.send(text, attachments);
    };

    const waitUntilIdle = async () => {
        const deadline = now() + (deps.stopWaitMs ?? DEFAULT_STOP_WAIT_MS);
        while (untrack(deps.working) && now() < deadline && !disposed) {
            await new Promise((resolve) => setTimeout(resolve, STOP_POLL_MS));
        }
    };

    /** Lifts the hold until everything held has gone; a running turn is
     *  stopped first, and each message goes once the pane is idle. */
    const sendNow = async (): Promise<void> => {
        setPaused(false);
        setForced(true);
        if (untrack(deps.working)) {
            deps.stop();
            await waitUntilIdle();
        }
    };

    const cancel = (id: string) => {
        if (id === untrack(inFlight)) return;
        deps.storage.saveHeld(untrack(held).filter((m) => m.id !== id));
    };

    // A network failure starts waiting for the API; any other clears the wait,
    // and so does the failure going away unless the operator typed since (then
    // their own send cleared it, and the turn is still to be offered).
    createEffect(() => {
        const f = deps.failure();
        const tracked = deps.tracked();
        untrack(() => {
            const turn = failed();
            if (!f) {
                if (turn && !turn.typedSince) setFailed(null);
                return;
            }
            if (f.data.code !== "network" || !tracked || f.at === decidedAt) {
                setFailed(null);
                return;
            }
            if (turn?.at === f.at) return;
            const live = workedSinceFailure;
            workedSinceFailure = false;
            setFailed({
                failure: f,
                at: f.at,
                live,
                typedSince: held().length > 0,
                snapshot: deps.ready() ? failedTurnSnapshot(deps.document()) : null,
            });
        });
    });

    const decide = async (turn: FailedTurn) => {
        setFailed(null);
        decidedAt = turn.at;
        const snapshot = turn.snapshot ?? failedTurnSnapshot(deps.document());
        const marker = deps.storage.loadMarker();
        const decision = decideOnReconnect({
            turn: snapshot,
            typedSince: turn.typedSince,
            live: turn.live,
            autoRetryOn: deps.autoRetryOn(),
            marker,
        });
        const text = snapshot.text;
        // The message's own time: a failure read back after a reload carries
        // the time it was read, not the time it happened.
        const which = snapshot.sentAt ? `your message from ${clock(snapshot.sentAt)}` : "your last message";
        if (decision.kind === "retry" && text) {
            // Marked and announced only once srv took it. Otherwise the
            // failure row, which the send cleared, comes back with its Retry.
            const settled = (ok: boolean) => {
                if (ok) {
                    deps.storage.saveMarker({ text, at: now() });
                    notice(`Back online: sent ${which} again, since its turn failed while the API was unreachable.`);
                } else if (!disposed && !untrack(deps.failure)) {
                    deps.restoreFailure(turn.failure);
                }
            };
            const outcome = await deps.deliver(text, undefined, settled);
            if (outcome !== "unknown") settled(outcome === "accepted");
            return;
        }
        // The failure row still offers Retry unless the operator's own send
        // cleared it; then the offer is here, until a retry goes out.
        if (decision.kind === "offer" && decision.reason === "typed" && text && !deps.failure()) {
            const retry = () => {
                const drop = (ok: boolean) => ok && setNotices((list) => list.filter((n) => n.retry !== retry));
                void deps.deliver(text, undefined, drop).then((outcome) => drop(outcome === "accepted"));
            };
            const what = which[0].toUpperCase() + which.slice(1);
            notice(`Back online. ${what} failed while the API was unreachable; it wasn't sent again, since you wrote after it.`, retry);
        }
    };

    // Decide once the API answers, the pane is idle and held messages have gone.
    createEffect(() => {
        const turn = failed();
        const due = turn && deps.api() === "online" && !deps.retrying() && !deps.working() && deps.ready() && held().length === 0;
        if (settleTimer) clearTimeout(settleTimer);
        settleTimer = null;
        if (!due) return;
        settleTimer = setTimeout(() => {
            settleTimer = null;
            if (!disposed && untrack(failed) === turn) void untrack(() => decide(turn));
        }, (deps.settleMs ?? defaultSettleMs)());
    });

    const reconnectHint = (): string | null => {
        const turn = failed();
        if (!turn || deps.api() === "online") return null;
        const decision = decideOnReconnect({
            turn: turn.snapshot ?? failedTurnSnapshot(deps.document()),
            typedSince: turn.typedSince,
            live: turn.live,
            autoRetryOn: deps.autoRetryOn(),
            marker: deps.storage.loadMarker(),
        });
        return decision.kind === "retry" ? "retries by itself when the API is back" : null;
    };

    return {
        held: () => held().filter((m) => m.id !== inFlight()),
        notices,
        submit,
        sendNow,
        cancel,
        dismissNotice,
        waitsForReconnect: (code) => code === "network" && untrack(deps.tracked),
        failureDismissed: () => setFailed(null),
        reconnectHint,
    };
}
