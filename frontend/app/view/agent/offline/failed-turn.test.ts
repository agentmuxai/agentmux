// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PaneFailure } from "@/app/store/agent-pane-state/types";
import type { AgentFailure } from "@/types/rpc/AgentFailure";
import type { AttachmentRef } from "@/types/rpc/AttachmentRef";
import type { Reachability } from "@/types/rpc/Reachability";
import type { DocumentNode } from "../types";
import {
    createFailedTurnRecovery,
    decideOnReconnect,
    failedTurnSnapshot,
    type FailedTurnRecovery,
    lastUserMessage,
    type RetryMark,
    type TurnStarted,
} from "./failed-turn";
import { type DeliveryOutcome, keyedLock } from "./offline-input";

type Attachment = { id?: string; name: string };
const user = (message: string, opts: { timestamp?: number; attachments?: Attachment[]; supervisor?: boolean } = {}): DocumentNode =>
    ({
        type: "user_message",
        id: `u-${message}`,
        message,
        ...(opts.timestamp ? { timestamp: opts.timestamp } : {}),
        ...(opts.attachments ? { attachments: opts.attachments } : {}),
        ...(opts.supervisor ? { supervisor: { mode: "steady", n: 1, of: 3 } } : {}),
    }) as DocumentNode;
const tool = (): DocumentNode => ({ type: "tool", id: "t" }) as unknown as DocumentNode;
const jekt = (direction: "incoming" | "outgoing" = "incoming"): DocumentNode =>
    ({ type: "jekt_message", id: "j", message: "hi", direction }) as unknown as DocumentNode;
const network = (): AgentFailure => ({ code: "network", title: "Can't reach the API", detail: "ECONNREFUSED", retryable: true });
const PNG: AttachmentRef = { id: "a".repeat(64), name: "shot.png" };

describe("pure rules", () => {
    it("re-sends a message with the attachments that can still be sent", () => {
        expect(lastUserMessage([user("a"), user("look", { attachments: [PNG, { name: "gone.png" }] })])).toEqual({
            text: "look",
            attachments: [PNG],
        });
        expect(lastUserMessage([user("only text")])).toEqual({ text: "only text" });
        expect(lastUserMessage([user("", { attachments: [PNG] })])).toEqual({ text: "", attachments: [PNG] });
        expect(lastUserMessage([user("")])).toBeNull();
        expect(lastUserMessage([])).toBeNull();
    });

    it("reads the failed turn off the document", () => {
        expect(failedTurnSnapshot([user("a"), user("b")])).toEqual({ message: { text: "b" }, toolRan: false });
        expect(failedTurnSnapshot([user("a"), tool(), user("b"), tool()])).toEqual({ message: { text: "b" }, toolRan: true });
        // A tool before the message a retry re-sends doesn't count.
        expect(failedTurnSnapshot([user("a"), tool(), user("b")])).toEqual({ message: { text: "b" }, toolRan: false });
        // The agent messaging another agent is a tool call.
        expect(failedTurnSnapshot([user("b"), jekt("outgoing")]).toolRan).toBe(true);
        // Started by another agent, or by the Swarm Supervisor: not the operator's.
        expect(failedTurnSnapshot([user("a"), jekt()])).toEqual({ message: null, toolRan: false });
        expect(failedTurnSnapshot([user("keep going", { supervisor: true })]).message).toBeNull();
        expect(failedTurnSnapshot([])).toEqual({ message: null, toolRan: false });
        // Started by a background task's notice from the CLI: not the operator's earlier message.
        const notice: DocumentNode = { type: "system_note", id: "n1", text: "Monitor event", source: "cli", raw: "<task-notification/>", timestamp: 99 };
        expect(failedTurnSnapshot([user("a"), notice, tool()])).toEqual({ message: null, toolRan: true, sentAt: 99 });
        expect(failedTurnSnapshot([user("b", { timestamp: 1234, attachments: [PNG] })])).toEqual({
            message: { text: "b", attachments: [PNG] },
            toolRan: false,
            sentAt: 1234,
        });
    });

    it("retries only a clean, live turn the operator started and nobody typed after, once, with the setting on", () => {
        const turn = { message: { text: "go" }, toolRan: false };
        const base = { turn, typedSince: false, live: true, autoRetryOn: true, mark: null };
        const mark = (m: Partial<RetryMark>): RetryMark => ({ text: "go", at: 1, auto: true, state: "sent", ...m });
        expect(decideOnReconnect(base)).toEqual({ kind: "retry" });
        expect(decideOnReconnect({ ...base, live: false })).toEqual({ kind: "offer", reason: "read_back" });
        expect(decideOnReconnect({ ...base, typedSince: true })).toEqual({ kind: "offer", reason: "typed" });
        expect(decideOnReconnect({ ...base, turn: { ...turn, toolRan: true } })).toEqual({ kind: "offer", reason: "tool_ran" });
        expect(decideOnReconnect({ ...base, autoRetryOn: false })).toEqual({ kind: "offer", reason: "setting_off" });
        expect(decideOnReconnect({ ...base, turn: { message: null, toolRan: false } })).toEqual({ kind: "offer", reason: "nothing_to_resend" });
        // A failed automatic retry, or one whose fate isn't known, isn't retried again.
        expect(decideOnReconnect({ ...base, mark: mark({}) })).toEqual({ kind: "offer", reason: "retried_once" });
        expect(decideOnReconnect({ ...base, mark: mark({ auto: false, state: "unknown" }) })).toEqual({ kind: "offer", reason: "retried_once" });
        // The operator's own Retry that went, or another message's retry, doesn't count.
        expect(decideOnReconnect({ ...base, mark: mark({ auto: false }) })).toEqual({ kind: "retry" });
        expect(decideOnReconnect({ ...base, mark: mark({ text: "other" }) })).toEqual({ kind: "retry" });
    });
});

/** A pane: plain signals for what the recovery reads, a recording deliver. */
function makePane(
    opts: {
        api?: Reachability | null;
        doc?: DocumentNode[];
        mark?: ReturnType<typeof createSignal<RetryMark | null>>;
        /** A send starts a turn that keeps running (Submitting, then Streaming). */
        startsTurn?: boolean;
    } = {}
) {
    const [failure, setFailure] = createSignal<PaneFailure | null>(null);
    const [api, setApi] = createSignal<Reachability | null>(opts.api === undefined ? "offline" : opts.api);
    const [tracked, setTracked] = createSignal(true);
    const [retrying] = createSignal(false);
    const [ready] = createSignal(true);
    const [working, setWorking] = createSignal(false);
    const [held, setHeld] = createSignal(0);
    const [autoRetryOn, setAutoRetryOn] = createSignal(true);
    const [doc, setDoc] = createSignal<DocumentNode[]>(opts.doc ?? [user("build it", { timestamp: 1_000, attachments: [PNG] })]);
    const [mark, setMark] = opts.mark ?? createSignal<RetryMark | null>(null);
    /** The last turn this mount started. */
    const [started, setStarted] = createSignal<TurnStarted | null>(null);
    const sent: { text: string; attachments?: AttachmentRef[] }[] = [];
    /** What each delivery answers; a pending entry waits for `answer`. */
    const outcomes: (DeliveryOutcome | "pending")[] = [];
    const late: ((ok: boolean) => void)[] = [];
    let answer: ((o: DeliveryOutcome) => void) | null = null;
    const deliver = vi.fn((text: string, attachments?: AttachmentRef[], onLate?: (ok: boolean) => void): Promise<DeliveryOutcome> => {
        sent.push(attachments ? { text, attachments } : { text });
        // The pane's send starts a turn, and TurnStart clears the row.
        setFailure(null);
        if (opts.startsTurn) {
            setStarted({ at: 5_000, content: text, hidden: false });
            setWorking(true);
        }
        const outcome = outcomes.length > 0 ? outcomes.shift()! : "accepted";
        if (outcome === "unknown" && onLate) late.push(onLate);
        if (outcome === "pending") return new Promise((resolve) => (answer = resolve));
        return Promise.resolve(outcome);
    });
    const restoreFailure = vi.fn((f: PaneFailure) => setFailure(f));
    const clearFailure = vi.fn(() => setFailure(null));
    let ui!: FailedTurnRecovery;
    const dispose = createRoot((d) => {
        ui = createFailedTurnRecovery({
            failure,
            started,
            tracked,
            api,
            retrying,
            ready,
            working,
            held,
            document: doc,
            autoRetryOn,
            deliver,
            restoreFailure,
            clearFailure,
            storage: { mark, saveMark: setMark },
            lock: keyedLock(new Map(), "b"),
            now: () => 5_000,
            settleMs: () => 0,
            clockTime: (ms) => `t${ms}`,
        });
        return d;
    });
    let nextAt = 100;
    return {
        ui,
        sent,
        deliver,
        outcomes,
        late,
        answer: (o: DeliveryOutcome) => answer?.(o),
        restoreFailure,
        clearFailure,
        failure,
        mark,
        setMark,
        setApi,
        setTracked,
        setWorking,
        setHeld,
        setAutoRetryOn,
        setDoc,
        dispose,
        setStarted,
        /** A network failure of the turn this pane started with the last message. */
        failLive: (data: AgentFailure = network(), start: Partial<TurnStarted> = {}) => {
            const at = nextAt++;
            const last = [...doc()].reverse().find((n) => n.type === "user_message");
            setStarted({ at: at - 0.5, content: last?.type === "user_message" ? last.message : "", hidden: false, ...start });
            setFailure({ data, at, turnAttempted: true });
            return at;
        },
        /** One that follows no turn this mount started. */
        failAfter: (data: AgentFailure = network()) => {
            const at = nextAt++;
            setFailure({ data, at, turnAttempted: true });
            return at;
        },
        /** One read back on mount, or replayed by the broker to a pane that ran nothing. */
        failReadBack: (data: AgentFailure = network()) => setFailure({ data, at: nextAt++, turnAttempted: true }),
        /** The failure row's Dismiss, as useAgentFailure calls it. */
        dismissRow: () => {
            ui.failureDismissed();
            setFailure(null);
        },
        /** The operator's own send while the row shows: TurnStart clears it. */
        typeAndSend: () => {
            ui.typed();
            setFailure(null);
        },
    };
}

const flush = () => vi.advanceTimersByTimeAsync(0);

describe("a turn that failed on the network", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    it("is retried once, with its attachments, when the API is back, and says so", async () => {
        const p = makePane();
        p.failLive();
        expect(p.ui.waitsForReconnect("network")).toBe(true);
        expect(p.ui.reconnectHint()).toBe("retries by itself when the API is back");
        await flush();
        expect(p.sent).toEqual([]);

        p.setApi("online");
        await flush();
        expect(p.sent).toEqual([{ text: "build it", attachments: [PNG] }]);
        expect(p.mark()).toEqual({ text: "build it", at: 5_000, auto: true, state: "sent" });
        expect(p.ui.notices().map((n) => n.text)).toEqual([
            "Back online: sent your message from t1000 again, since its turn failed while the API was unreachable.",
        ]);

        // A flap afterwards sends nothing again.
        p.setApi("offline");
        p.setApi("online");
        await flush();
        expect(p.sent).toHaveLength(1);
        p.dispose();
    });

    it("waits for the pane to be idle and for a flap to settle", async () => {
        const p = makePane();
        p.failLive();
        p.setWorking(true);
        p.setApi("online");
        await flush();
        expect(p.sent).toEqual([]);
        p.setWorking(false);
        await flush();
        expect(p.sent).toHaveLength(1);
        p.dispose();
    });

    it("is only offered with the setting off, and leaves network failures to the row's timed retry", async () => {
        const p = makePane();
        p.setAutoRetryOn(false);
        // As before retry-on-reconnect: the row counts down and retries by itself.
        expect(p.ui.waitsForReconnect("network")).toBe(false);
        p.failLive();
        expect(p.ui.reconnectHint()).toBeNull();
        expect(p.ui.pending()).toBe(false);
        p.setApi("online");
        await flush();
        expect(p.sent).toEqual([]);
        // The row's Retry still re-sends it.
        expect(p.ui.retry()).toBe(true);
        await flush();
        expect(p.sent).toEqual([{ text: "build it", attachments: [PNG] }]);
        p.dispose();
    });

    it("is only offered when a tool ran in it", async () => {
        const p = makePane({ doc: [user("build it"), tool()] });
        p.failLive();
        p.setApi("online");
        await flush();
        expect(p.sent).toEqual([]);
        expect(p.failure()).not.toBeNull();
        p.dispose();
    });

    it("is only offered when another agent started it", async () => {
        const p = makePane({ doc: [user("earlier"), jekt()] });
        p.failLive();
        p.setApi("online");
        await flush();
        expect(p.sent).toEqual([]);
        // Nothing of the operator's to re-send: the row's own retry runs instead.
        expect(p.ui.retry()).toBe(false);
        p.dispose();
    });

    it("isn't retried again when the automatic retry fails too", async () => {
        const p = makePane({ api: "online" });
        p.failLive();
        await flush();
        expect(p.sent).toHaveLength(1);
        // The retried turn fails on the network again.
        p.setApi("offline");
        p.failLive();
        p.setApi("online");
        await flush();
        expect(p.sent).toHaveLength(1);
        expect(p.failure()).not.toBeNull();
        p.dispose();
    });

    it("retrying a failed automatic retry by hand doesn't earn it another automatic one", async () => {
        const p = makePane({ api: "online" });
        p.failLive();
        await flush();
        p.failLive();
        await flush();
        expect(p.sent).toHaveLength(1);
        p.ui.retry();
        await flush();
        expect(p.sent).toHaveLength(2);
        expect(p.mark()?.auto).toBe(true);
        // Only the first one said it went by itself.
        expect(p.ui.notices()).toHaveLength(1);
        p.failLive();
        await flush();
        expect(p.sent).toHaveLength(2);
        p.dispose();
    });

    it("stops waiting on an API that stays unchecked, so a missing status can't keep the turn waiting", async () => {
        const p = makePane({ api: "unknown" });
        p.failLive();
        await vi.advanceTimersByTimeAsync(19_000);
        expect(p.sent).toEqual([]);
        // The settle pause starts once the wait ends.
        await vi.advanceTimersByTimeAsync(1_001);
        expect(p.sent).toHaveLength(1);
        p.dispose();
    });

    it("leaves other failures, and an API srv doesn't track, to the row's timed retry", () => {
        const p = makePane();
        expect(p.ui.waitsForReconnect("rate_limited")).toBe(false);
        p.setTracked(false);
        expect(p.ui.waitsForReconnect("network")).toBe(false);
        p.failLive();
        expect(p.ui.retry()).toBe(false);
        p.dispose();
    });

    it("read back on mount, or replayed to a pane that ran nothing, is only offered", async () => {
        const p = makePane({ api: "online" });
        p.failReadBack();
        await flush();
        expect(p.sent).toEqual([]);
        // Its Retry re-sends it, once.
        p.ui.retry();
        await flush();
        expect(p.sent).toEqual([{ text: "build it", attachments: [PNG] }]);
        p.dispose();
    });

    it("is only offered when this mount's last turn came before an earlier failure", async () => {
        const p = makePane({ api: "online" });
        p.setTracked(false);
        p.failLive();
        p.setTracked(true);
        // The broker replays that failure later: no turn started since.
        p.failAfter();
        await flush();
        expect(p.sent).toEqual([]);
        p.dispose();
    });

    it("is only offered when the turn wasn't the operator's message (a hidden memory turn, another text)", async () => {
        const p = makePane({ api: "online" });
        p.failLive(network(), { hidden: true });
        await flush();
        expect(p.sent).toEqual([]);
        p.dispose();

        const q = makePane({ api: "online" });
        q.failLive(network(), { content: "something else" });
        await flush();
        expect(q.sent).toEqual([]);
        q.dispose();
    });

    it("typed since: the operator's message goes first and the turn is offered, with its attachments", async () => {
        const p = makePane();
        p.failLive();
        p.setHeld(1);
        p.typeAndSend();
        p.setApi("online");
        await flush();
        // Their held message hasn't gone yet.
        expect(p.ui.notices()).toEqual([]);
        p.setHeld(0);
        p.setDoc([user("build it", { timestamp: 1_000, attachments: [PNG] }), user("u there?", { timestamp: 2_000 })]);
        await flush();
        expect(p.sent).toEqual([]);
        const offer = p.ui.notices().find((n) => n.retry)!;
        expect(offer.text).toBe(
            "Back online. Your message from t1000 failed while the API was unreachable; it wasn't sent again, since you wrote after it."
        );
        offer.retry!();
        await flush();
        expect(p.sent).toEqual([{ text: "build it", attachments: [PNG] }]);
        expect(p.ui.notices().some((n) => n.retry)).toBe(false);
        p.dispose();
    });

    it("sends at most once: a second click, or a click during the automatic retry, sends nothing", async () => {
        const p = makePane({ api: "online" });
        p.outcomes.push("pending");
        p.failLive();
        await flush();
        expect(p.sent).toHaveLength(1);
        // Stored as on its way before srv answers.
        expect(p.mark()?.state).toBe("sending");
        expect(p.ui.retryBusy()).toBe(true);
        expect(p.ui.retry()).toBe(true);
        expect(p.ui.retry()).toBe(true);
        await flush();
        expect(p.sent).toHaveLength(1);
        p.answer("accepted");
        await flush();
        expect(p.ui.retryBusy()).toBe(false);
        p.dispose();
    });

    it("an offered Retry is disabled while it is on its way", async () => {
        const p = makePane();
        p.failLive();
        p.typeAndSend();
        p.setApi("online");
        await flush();
        p.outcomes.push("pending");
        const offer = () => p.ui.notices().find((n) => n.retry)!;
        offer().retry!();
        offer().retry!();
        await flush();
        expect(p.sent).toHaveLength(1);
        expect(offer().retryDisabled).toBe(true);
        p.answer("rejected");
        await flush();
        // It didn't go: the offer stays, and can be taken again.
        expect(offer().retryDisabled).toBe(false);
        p.dispose();
    });

    it("an unknown outcome keeps the row, its Retry and a warning, and isn't marked retried", async () => {
        const p = makePane({ api: "online" });
        p.outcomes.push("unknown");
        const at = p.failLive();
        await flush();
        expect(p.sent).toHaveLength(1);
        expect(p.mark()?.state).toBe("unknown");
        // The row is back, saying the retry may have gone.
        expect(p.restoreFailure).toHaveBeenCalledTimes(1);
        expect(p.failure()?.at).toBe(at);
        expect(p.ui.reconnectHint()).toBe("a retry may have gone already");
        // Nothing goes again by itself.
        p.setApi("offline");
        p.setApi("online");
        await flush();
        expect(p.sent).toHaveLength(1);
        // The operator can still retry it, and is told it may have gone twice.
        expect(p.ui.retry()).toBe(true);
        await flush();
        expect(p.sent).toHaveLength(2);
        expect(p.ui.notices().map((n) => n.text)).toEqual(["Sent your message again. It may have gone once already."]);
        p.dispose();
    });

    it("an unknown outcome doesn't end the retried turn: the row comes back once the pane is idle, if still unknown", async () => {
        const p = makePane({ api: "online", startsTurn: true });
        p.outcomes.push("unknown");
        const at = p.failLive();
        await flush();
        expect(p.mark()?.state).toBe("unknown");
        // The retried turn still runs: putting the failure back would end it.
        expect(p.restoreFailure).not.toHaveBeenCalled();
        expect(p.failure()).toBeNull();
        p.setWorking(false);
        expect(p.restoreFailure).toHaveBeenCalledTimes(1);
        expect(p.failure()?.at).toBe(at);
        p.dispose();

        // Settled while it ran: nothing to put back.
        const q = makePane({ api: "online", startsTurn: true });
        q.outcomes.push("unknown");
        q.failLive();
        await flush();
        q.late[0](true);
        q.setWorking(false);
        expect(q.restoreFailure).not.toHaveBeenCalled();
        expect(q.failure()).toBeNull();
        q.dispose();
    });

    it("the retried turn completing settles an unanswered retry as sent", async () => {
        const p = makePane({ api: "online", startsTurn: true });
        p.outcomes.push("unknown");
        p.failLive();
        await flush();
        p.ui.turnCompleted();
        p.setWorking(false);
        expect(p.restoreFailure).not.toHaveBeenCalled();
        expect(p.failure()).toBeNull();
        expect(p.ui.retry()).toBe(false);
        expect(p.ui.notices().map((n) => n.text)).toEqual([
            "Back online: sent your message from t1000 again, since its turn failed while the API was unreachable.",
        ]);
        // Done with: a late answer changes nothing.
        p.late[0](false);
        expect(p.mark()).toBeNull();
        p.dispose();
    });

    it("a rejected retry waits for the pane to be idle before putting the row back", async () => {
        // The send path left the pane looking busy.
        const p = makePane({ api: "online", startsTurn: true });
        p.outcomes.push("rejected");
        p.failLive();
        await flush();
        expect(p.sent).toHaveLength(1);
        expect(p.restoreFailure).not.toHaveBeenCalled();
        p.setWorking(false);
        expect(p.restoreFailure).toHaveBeenCalledTimes(1);
        p.dispose();
    });

    it("an unknown outcome's late answer that it went takes the row away", async () => {
        const p = makePane({ api: "online" });
        p.outcomes.push("unknown");
        p.failLive();
        await flush();
        expect(p.failure()).not.toBeNull();
        p.late[0](true);
        expect(p.clearFailure).toHaveBeenCalledTimes(1);
        expect(p.failure()).toBeNull();
        expect(p.mark()?.state).toBe("sent");
        expect(p.ui.retry()).toBe(false);
        p.dispose();
    });

    it("a rejected retry puts the row back with its Retry", async () => {
        const p = makePane({ api: "online" });
        p.outcomes.push("rejected");
        p.failLive();
        await flush();
        expect(p.failure()).not.toBeNull();
        expect(p.mark()).toBeNull();
        expect(p.ui.reconnectHint()).toBeNull();
        p.dispose();
    });

    it("dismissing the row drops the turn, even one the operator typed after", async () => {
        const p = makePane();
        p.failLive();
        p.dismissRow();
        p.setApi("online");
        await flush();
        expect(p.sent).toEqual([]);
        p.dispose();

        const q = makePane();
        q.failLive();
        q.ui.typed();
        q.dismissRow();
        q.setApi("online");
        await flush();
        expect(q.sent).toEqual([]);
        expect(q.ui.notices()).toEqual([]);
        q.dispose();
    });

    it("a clear by the operator's send isn't a dismiss: the turn is still offered", async () => {
        const p = makePane({ api: "online" });
        p.failLive();
        p.typeAndSend();
        await flush();
        expect(p.ui.notices().some((n) => n.retry)).toBe(true);
        p.dispose();
    });

    it("a remount mid-retry doesn't repeat it", async () => {
        const shared = createSignal<RetryMark | null>(null);
        const first = makePane({ api: "online", mark: shared });
        first.outcomes.push("pending");
        first.failLive();
        await flush();
        expect(first.sent).toHaveLength(1);
        first.dispose();

        // The reopened pane reads the failure back while that retry is on its way.
        const second = makePane({ api: "online", mark: shared });
        second.failReadBack();
        await flush();
        expect(second.sent).toEqual([]);
        expect(second.ui.retryBusy()).toBe(true);
        expect(second.ui.reconnectHint()).toBe("sending it again");
        expect(second.ui.retry()).toBe(true);
        await flush();
        expect(second.sent).toEqual([]);
        // The first mount's retry went: the read-back row goes with it.
        first.answer("accepted");
        await flush();
        expect(shared[0]()?.state).toBe("sent");
        expect(second.clearFailure).toHaveBeenCalledTimes(1);
        expect(second.ui.retry()).toBe(false);
        second.dispose();
    });

    it("a retry found in the transcript after its answer never came did go", async () => {
        const p = makePane({ api: "online" });
        p.outcomes.push("unknown");
        p.failLive();
        await flush();
        expect(p.failure()).not.toBeNull();
        p.setDoc([user("build it", { timestamp: 1_000 }), user("build it", { timestamp: 5_100 })]);
        await flush();
        expect(p.mark()?.state).toBe("sent");
        expect(p.failure()).toBeNull();
        p.dispose();
    });

    it("keeps a reload from losing a live failure that waits, or a retry on its way", async () => {
        const p = makePane();
        expect(p.ui.pending()).toBe(false);
        p.failLive();
        expect(p.ui.pending()).toBe(true);
        p.outcomes.push("pending");
        p.setApi("online");
        await flush();
        expect(p.ui.pending()).toBe(true);
        p.answer("accepted");
        await flush();
        expect(p.ui.pending()).toBe(false);
        p.dispose();
    });
});
