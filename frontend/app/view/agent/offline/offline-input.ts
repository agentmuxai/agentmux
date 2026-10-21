// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a pane does with the operator's input while its model API can't be
 * reached (docs/specs/SPEC_OFFLINE_RESILIENCE_2026_10_20.md §4.4).
 *
 * A message typed while the pane's turn is retrying an API request, or while
 * srv's connectivity state says the provider's API is offline or not checked
 * yet (for at most {@link UNKNOWN_HOLD_MS}), isn't sent:
 * it waits under the composer with Send now and Cancel, and goes out by
 * itself, in order and once, when neither holds, one at a time while the
 * pane is idle. It is kept with the pane's block meta until srv accepts it,
 * so a rejected send, a reload or a restart keeps it; one whose fate isn't
 * known is kept, marked as possibly gone, and left to the operator. A `/`
 * or `!` command is never held.
 *
 * {@link createOfflineInput} holds the rules over injected inputs, so tests
 * drive it with plain signals; `useOfflineInput.ts` wires the real ones.
 */

import { type Accessor, createEffect, createSignal, onCleanup, untrack } from "solid-js";
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
    /** The latest send of it; a late answer to an earlier one is ignored. */
    attempt?: string;
}

/** What the pane keeps between reloads. */
export interface OfflineStorage {
    /** The held messages, shared by every mount of the pane. Reactive. */
    held(): HeldInput[];
    saveHeld(held: HeldInput[]): void;
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

/**
 * That service's state; `unknown` for one srv tracks but the pane has no
 * word on yet (no status, or a status without it); null when srv doesn't
 * track it.
 */
export function serviceReachability(
    services: readonly ServiceStatus[] | undefined,
    service: string | null
): Reachability | null {
    if (!service) return null;
    const state = services?.find((s) => s.service === service)?.state;
    return state ?? (isTrackedService(service, services) ? "unknown" : null);
}

/** Whether the operator's input waits instead of going out: while the API
 *  is offline, or not checked yet, or the turn is retrying. */
export function holdsInput(retrying: boolean, api: Reachability | null): boolean {
    return retrying || api === "offline" || api === "unknown";
}

/**
 * How long a tracked API may stay not checked before input stops waiting on
 * it: srv probes it within seconds of starting, so a longer wait means the
 * pane isn't getting its status, which mustn't hold everything for good.
 */
export const UNKNOWN_HOLD_MS = 20_000;

/** A `/` or `!` command runs in the pane (or is the CLI's own), so it is
 *  never held: help, login and shell commands keep working offline. */
export function isLocalCommand(text: string, attachments?: readonly AttachmentRef[]): boolean {
    const t = text.trimStart();
    return !attachments?.length && (t.startsWith("/") || t.startsWith("!"));
}

/** A message with the held one's text, sent after it was held. */
export function sentInTranscript(doc: readonly DocumentNode[], item: HeldInput): boolean {
    return doc.some(
        (n) => n.type === "user_message" && n.message === item.text && typeof n.timestamp === "number" && n.timestamp >= item.heldAt
    );
}

/** "held under a minute", "held 4 min". */
export function heldForText(ms: number): string {
    const min = Math.round(ms / 60_000);
    return min < 1 ? "held under a minute" : `held ${min} min`;
}

/** A line the pane shows after it sent, or couldn't send, held input. */
export interface OfflineNotice {
    id: number;
    text: string;
}

export interface OfflineInputDeps {
    /** The pane's turn is retrying an API request. */
    retrying: Accessor<boolean>;
    /** The provider's API as {@link serviceReachability} reads it. */
    api: Accessor<Reachability | null>;
    /** The pane can send (its history has loaded). */
    ready: Accessor<boolean>;
    /** A turn is running. */
    working: Accessor<boolean>;
    /** The transcript, to find a message that may have gone. */
    document: () => readonly DocumentNode[];
    /** Send as if typed, with no hold: the pane's own send path. */
    send: (text: string, attachments?: AttachmentRef[]) => Promise<void>;
    /** The same, saying whether srv accepted it ({@link confirmedSend});
     *  after `unknown`, a late answer goes to `onLate`. */
    deliver: (
        text: string,
        attachments?: AttachmentRef[],
        onLate?: (accepted: boolean) => void
    ) => Promise<DeliveryOutcome>;
    /** Stop the running turn. */
    stop: () => void;
    storage: OfflineStorage;
    /** One chain per pane, shared by every mount of it, so two never send
     *  the same held message. */
    lock: { run<T>(task: () => Promise<T>): Promise<T> };
    now?: () => number;
    /** How long Send now waits for the stopped turn to end. */
    stopWaitMs?: number;
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
}

const DEFAULT_STOP_WAIT_MS = 5_000;
const STOP_POLL_MS = 100;

let nextHeldSeq = 0;

export function createOfflineInput(deps: OfflineInputDeps): OfflineInput {
    const now = deps.now ?? Date.now;
    const held = () => deps.storage.held();
    const [notices, setNotices] = createSignal<OfflineNotice[]>([]);
    /** The held message being sent now: it stays stored until srv takes it. */
    const [inFlight, setInFlight] = createSignal<string | null>(null);
    /** Send now: send while the hold lasts. */
    const [forced, setForced] = createSignal(false);
    /** A held message didn't go out: wait for the operator, or the next outage. */
    const [paused, setPaused] = createSignal(false);
    /** The API has been not checked for longer than {@link UNKNOWN_HOLD_MS}. */
    const [unknownTooLong, setUnknownTooLong] = createSignal(false);
    let unknownTimer: ReturnType<typeof setTimeout> | null = null;
    let nextAttempt = 0;
    let nextNoticeId = 0;
    let disposed = false;
    /** Held messages sent since the list was last empty, for one notice. */
    let burst: { noticeId: number; sent: number; oldest: number } | null = null;
    onCleanup(() => {
        disposed = true;
        if (unknownTimer) clearTimeout(unknownTimer);
    });

    createEffect(() => {
        const unknown = deps.api() === "unknown";
        if (unknownTimer) clearTimeout(unknownTimer);
        unknownTimer = null;
        setUnknownTooLong(false);
        if (unknown) unknownTimer = setTimeout(() => setUnknownTooLong(true), UNKNOWN_HOLD_MS);
    });

    const holding = () => {
        const api = deps.api();
        return holdsInput(deps.retrying(), api === "unknown" && unknownTooLong() ? null : api);
    };
    const notice = (text: string) => setNotices((list) => [...list, { id: nextNoticeId++, text }]);
    const dismissNotice = (id: number) => setNotices((list) => list.filter((n) => n.id !== id));

    const noteSent = (item: HeldInput) => {
        const b = burst ?? { noticeId: nextNoticeId++, sent: 0, oldest: Infinity };
        burst = { ...b, sent: b.sent + 1, oldest: Math.min(b.oldest, item.heldAt) };
        const what = burst.sent === 1 ? "Sent your held message" : `Sent your ${burst.sent} held messages`;
        const text = `${what} (${heldForText(now() - burst.oldest)}).`;
        const id = burst.noticeId;
        setNotices((list) => [...list.filter((n) => n.id !== id), { id, text }]);
    };

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
     *  hold it as before once rejected. Only for its latest send: a later
     *  one may be in flight, or have gone. */
    const settleLate = (id: string, attempt: string, ok: boolean) => {
        const item = untrack(held).find((m) => m.id === id);
        if (!item?.unconfirmed || item.attempt !== attempt) return;
        if (ok) {
            accepted(item);
            return;
        }
        markHeld(id, { unconfirmed: false });
        setPaused(true);
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
            const attempt = `${now()}_${nextAttempt++}`;
            markHeld(item.id, { unconfirmed: true, attempt });
            const outcome = await deps.deliver(item.text, item.attachments, (ok) => settleLate(item.id, attempt, ok));
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

    // One that may have gone and is in the transcript did go: drop it.
    createEffect(() => {
        const unsure = held().filter((m) => m.unconfirmed && m.id !== inFlight());
        if (unsure.length === 0 || !deps.ready()) return;
        const doc = deps.document();
        for (const item of unsure) if (sentInTranscript(doc, item)) untrack(() => accepted(item));
    });

    // Release once nothing holds (the API is known reachable and the turn
    // isn't retrying) and the pane is idle; the next goes when that turn ends.
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
        // What the pane said it did is old news once the operator types.
        setNotices([]);
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
        const rest = untrack(held).filter((m) => m.id !== id);
        deps.storage.saveHeld(rest);
        // Send now was for the messages just cancelled, not for the next one typed.
        if (rest.length === 0) setForced(false);
    };

    return {
        held: () => held().filter((m) => m.id !== inFlight()),
        notices,
        submit,
        sendNow,
        cancel,
        dismissNotice,
    };
}
