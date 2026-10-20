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
 * and goes out by itself, in order and once, when neither holds. It is kept
 * with the pane's block meta, so a reload or a restart keeps it. A `/` or
 * `!` command is never held.
 *
 * **A turn that failed on the network.** When the provider's API answers
 * again, the turn is retried once by itself if nothing argues against it:
 * the operator hasn't typed in the pane since (their message goes first,
 * and the turn is only offered), no tool ran in it (a retry would run the
 * tool again), it isn't itself an automatic retry that failed, and the
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
}

/** The turn an automatic retry re-sent, so its failure isn't retried again. */
export interface AutoRetryMarker {
    text: string;
    at: number;
}

/** What the pane keeps between reloads. */
export interface OfflineStorage {
    loadHeld(): HeldInput[];
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

export type ReconnectDecision =
    | { kind: "retry" }
    | { kind: "offer"; reason: "typed" | "tool_ran" | "setting_off" | "retried_once" | "nothing_to_resend" };

/** What to do with a network-failed turn once its API answers again. */
export function decideOnReconnect(input: {
    turn: FailedTurnSnapshot;
    typedSince: boolean;
    autoRetryOn: boolean;
    marker: AutoRetryMarker | null;
}): ReconnectDecision {
    if (!input.turn.text) return { kind: "offer", reason: "nothing_to_resend" };
    if (input.typedSince) return { kind: "offer", reason: "typed" };
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
    /** The provider's API as srv sees it; null when srv doesn't track it. */
    api: Accessor<Reachability | null>;
    /** The pane can send (its history has loaded). */
    ready: Accessor<boolean>;
    /** A turn is running. */
    working: Accessor<boolean>;
    failure: Accessor<PaneFailure | null>;
    document: () => readonly DocumentNode[];
    autoRetryOn: Accessor<boolean>;
    /** Send as if typed, with no hold: the pane's own send path. */
    send: (text: string, attachments?: AttachmentRef[]) => Promise<void>;
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
    at: number;
    typedSince: boolean;
    snapshot: FailedTurnSnapshot | null;
}

export interface OfflineInput {
    held: Accessor<HeldInput[]>;
    notices: Accessor<OfflineNotice[]>;
    /** The composer's send: holds while {@link holdsInput} says so, or
     *  while earlier held messages haven't gone yet. */
    submit: (text: string, attachments?: AttachmentRef[]) => Promise<void>;
    /** Stop a running turn, then send everything held, in order. */
    sendNow: () => Promise<void>;
    cancel: (id: string) => void;
    dismissNotice: (id: number) => void;
    /** A network failure this waits on: the failure row arms no timed
     *  retry for it, since retrying into an unreachable API only waits out
     *  another round of the CLI's retries. */
    waitsForReconnect: (code: string) => boolean;
    /** For the failure row, while the failed turn waits for the API. */
    reconnectHint: Accessor<string | null>;
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
    const [held, setHeld] = createSignal<HeldInput[]>(deps.storage.loadHeld());
    const [notices, setNotices] = createSignal<OfflineNotice[]>([]);
    const [failed, setFailed] = createSignal<FailedTurn | null>(null);
    let nextNoticeId = 0;
    let disposed = false;
    let settleTimer: ReturnType<typeof setTimeout> | null = null;
    onCleanup(() => {
        disposed = true;
        if (settleTimer) clearTimeout(settleTimer);
    });

    const holding = () => holdsInput(deps.retrying(), deps.api());
    const writeHeld = (next: HeldInput[]) => {
        setHeld(next);
        deps.storage.saveHeld(next);
    };
    const notice = (text: string, retry?: () => void) =>
        setNotices((list) => [...list, { id: nextNoticeId++, text, ...(retry ? { retry } : {}) }]);
    const dismissNotice = (id: number) => setNotices((list) => list.filter((n) => n.id !== id));

    /**
     * Send held messages, oldest first, each taken off the list before it
     * goes so it can't go twice. `force` (Send now) sends while held; else
     * it stops when the hold comes back, which a sent message can cause.
     */
    const release = (force: boolean): Promise<void> =>
        deps.lock.run(async () => {
            let sent = 0;
            let oldest = Infinity;
            while (!disposed && untrack(() => deps.ready())) {
                const list = untrack(held);
                if (list.length === 0 || (!force && untrack(holding))) break;
                const [item, ...rest] = list;
                writeHeld(rest);
                sent += 1;
                oldest = Math.min(oldest, item.heldAt);
                await deps.send(item.text, item.attachments);
            }
            if (sent > 0) {
                const what = sent === 1 ? "Sent your held message" : `Sent your ${sent} held messages`;
                notice(`${what} (${heldForText(now() - oldest)}).`);
            }
        });

    // Release once nothing holds: the API is reachable and the turn isn't retrying.
    createEffect(() => {
        if (held().length > 0 && !holding() && deps.ready()) void release(false);
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
            writeHeld([...untrack(held), item]);
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

    const sendNow = async (): Promise<void> => {
        if (untrack(deps.working)) {
            deps.stop();
            await waitUntilIdle();
        }
        await release(true);
    };

    const cancel = (id: string) => writeHeld(untrack(held).filter((m) => m.id !== id));

    // A network failure starts waiting for the API; any other clears the wait,
    // and so does the failure going away unless the operator typed since (then
    // their own send cleared it, and the turn is still to be offered).
    createEffect(() => {
        const f = deps.failure();
        const tracked = deps.api() !== null;
        untrack(() => {
            const turn = failed();
            if (!f) {
                if (turn && !turn.typedSince) setFailed(null);
                return;
            }
            if (f.data.code !== "network" || !tracked) {
                setFailed(null);
                return;
            }
            if (turn?.at === f.at) return;
            setFailed({
                at: f.at,
                typedSince: held().length > 0,
                snapshot: deps.ready() ? failedTurnSnapshot(deps.document()) : null,
            });
        });
    });

    const decide = (turn: FailedTurn) => {
        setFailed(null);
        const snapshot = turn.snapshot ?? failedTurnSnapshot(deps.document());
        const marker = deps.storage.loadMarker();
        const decision = decideOnReconnect({ turn: snapshot, typedSince: turn.typedSince, autoRetryOn: deps.autoRetryOn(), marker });
        const text = snapshot.text;
        // The message's own time: a failure read back after a reload carries
        // the time it was read, not the time it happened.
        const which = snapshot.sentAt ? `your message from ${clock(snapshot.sentAt)}` : "your last message";
        if (decision.kind === "retry" && text) {
            deps.storage.saveMarker({ text, at: now() });
            notice(`Back online: sent ${which} again, since its turn failed while the API was unreachable.`);
            void deps.send(text);
            return;
        }
        // The failure row still offers Retry unless the operator's own send
        // cleared it; then the offer is here.
        if (decision.kind === "offer" && decision.reason === "typed" && text && !deps.failure()) {
            const offer = () => {
                setNotices((list) => list.filter((n) => n.retry !== offer));
                void deps.send(text);
            };
            notice(`Back online. ${which[0].toUpperCase()}${which.slice(1)} failed while the API was unreachable; it wasn't sent again, since you wrote after it.`, offer);
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
            if (!disposed && untrack(failed) === turn) untrack(() => decide(turn));
        }, (deps.settleMs ?? defaultSettleMs)());
    });

    const reconnectHint = (): string | null => {
        const turn = failed();
        if (!turn || deps.api() === "online") return null;
        const decision = decideOnReconnect({
            turn: turn.snapshot ?? failedTurnSnapshot(deps.document()),
            typedSince: turn.typedSince,
            autoRetryOn: deps.autoRetryOn(),
            marker: deps.storage.loadMarker(),
        });
        return decision.kind === "retry" ? "retries by itself when the API is back" : null;
    };

    return {
        held,
        notices,
        submit,
        sendNow,
        cancel,
        dismissNotice,
        waitsForReconnect: (code) => code === "network" && untrack(deps.api) !== null,
        reconnectHint,
    };
}
