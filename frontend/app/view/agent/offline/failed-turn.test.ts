// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PaneFailure } from "@/app/store/agent-pane-state/types";
import type { AgentFailure } from "@/types/rpc/AgentFailure";
import type { AttachmentRef } from "@/types/rpc/AttachmentRef";
import type { Reachability } from "@/types/rpc/Reachability";
import type { DocumentNode } from "../../../agent-model/types";
import {
    createFailedTurnRecovery,
    decideOnReconnect,
    failedTurnSnapshot,
    type FailedTurnRecovery,
    lastUserMessage,
    type RetryMark,
    type TurnStarted,
} from "./failed-turn";
import { type DeliveryOutcome, keyedLock, UNKNOWN_HOLD_MS } from "./offline-input";

type Attachment = { id?: string; name: string };
const user = (message: string, opts: { timestamp?: number; attachments?: Attachment[]; supervisor?: boolean; system?: boolean } = {}): DocumentNode =>
    ({
        type: "user_message",
        id: `u-${message}`,
        message,
        ...(opts.timestamp ? { timestamp: opts.timestamp } : {}),
        ...(opts.attachments ? { attachments: opts.attachments } : {}),
        ...(opts.supervisor ? { supervisor: { mode: "steady", n: 1, of: 3 } } : {}),
        ...(opts.system ? { system: true } : {}),
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

    it("re-sends AgentMux's own message as AgentMux's, and says nothing for the person's", () => {
        expect(lastUserMessage([user("# Session Context", { system: true })])).toEqual({ text: "# Session Context", system: true });
        expect(lastUserMessage([user("fix the build")])).toEqual({ text: "fix the build" });
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
        // A schedule's or the Supervisor's turn (a signed origin) started it too, and an older record.
        const fired: DocumentNode = { type: "system_note", id: "n2", text: "nightly", origin: { kind: "schedule" }, timestamp: 77 };
        expect(failedTurnSnapshot([user("a"), fired, tool()])).toEqual({ message: null, toolRan: true, sentAt: 77 });
        const earlier: DocumentNode = { type: "system_note", id: "n3", text: "old", source: "earlier", timestamp: 55 };
        expect(failedTurnSnapshot([user("a"), earlier, tool()])).toEqual({ message: null, toolRan: true, sentAt: 55 });
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
        /** The document is loaded (default). */
        ready?: boolean;
        /** The CLI is retrying an API request. */
        retrying?: boolean;
        /** The pause before an automatic retry; "default" leaves the recovery's own. */
        settleMs?: number | "default";
        /** Leave the clock format to the recovery's own. */
        defaultClock?: boolean;
    } = {}
) {
    const [failure, setFailure] = createSignal<PaneFailure | null>(null);
    const [api, setApi] = createSignal<Reachability | null>(opts.api === undefined ? "offline" : opts.api);
    const [tracked, setTracked] = createSignal(true);
    const [retrying, setRetrying] = createSignal(opts.retrying ?? false);
    const [ready, setReady] = createSignal(opts.ready ?? true);
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
    const settleMs = opts.settleMs ?? 0;
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
            ...(settleMs === "default" ? {} : { settleMs: () => settleMs }),
            ...(opts.defaultClock ? {} : { clockTime: (ms: number) => `t${ms}` }),
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
        setReady,
        setRetrying,
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

const BASE = user("build it", { timestamp: 1_000, attachments: [PNG] });
const otherFailure = (): AgentFailure => ({ code: "rate_limited", title: "Slow down", detail: "429", retryable: true });
const retryMark = (m: Partial<RetryMark> = {}): RetryMark => ({
    text: "build it",
    at: 5_000,
    auto: true,
    state: "unknown",
    ...m,
});
const BACK_ONLINE =
    "Back online: sent your message from t1000 again, since its turn failed while the API was unreachable.";
const offerOf = (p: ReturnType<typeof makePane>) => p.ui.notices().find((n) => n.retry)!;

describe("when it retries and how it counts time", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    it("waits two to five seconds after the API answers when nothing says otherwise", async () => {
        const random = vi.spyOn(Math, "random").mockReturnValue(0.5);
        const p = makePane({ api: "online", settleMs: "default" });
        p.failLive();
        await vi.advanceTimersByTimeAsync(3_499);
        expect(p.sent).toEqual([]);
        await vi.advanceTimersByTimeAsync(1);
        expect(p.sent).toHaveLength(1);
        random.mockRestore();
        p.dispose();
    });

    it("names the time of the message in the clock format of the machine", async () => {
        const sentAt = new Date(2026, 10, 5, 14, 30).getTime();
        const p = makePane({ api: "online", doc: [user("build it", { timestamp: sentAt })], defaultClock: true });
        p.failLive();
        await flush();
        expect(p.ui.notices()[0].text).toMatch(/your message from 14\D30 again/);
        p.dispose();
    });

    it("says 'your last message' when the document doesn't know when it was sent", async () => {
        const p = makePane({ api: "online", doc: [user("build it")] });
        p.failLive();
        await flush();
        expect(p.ui.notices().map((n) => n.text)).toEqual([
            "Back online: sent your last message again, since its turn failed while the API was unreachable.",
        ]);
        p.dispose();
    });

    it("keeps every notice, in the order they came", async () => {
        const p = makePane({ api: "online" });
        p.failLive();
        await flush();
        p.setDoc([BASE, user("then this", { timestamp: 2_000 })]);
        p.failLive();
        await flush();
        expect(p.ui.notices().map((n) => n.text)).toEqual([
            BACK_ONLINE,
            "Back online: sent your message from t2000 again, since its turn failed while the API was unreachable.",
        ]);
        p.dispose();
    });

    it("leaves no timer behind once disposed", () => {
        const waiting = makePane({ api: "online", settleMs: 10_000 });
        waiting.failLive();
        expect(vi.getTimerCount()).toBe(1);
        waiting.dispose();
        expect(vi.getTimerCount()).toBe(0);

        const unchecked = makePane({ api: "unknown" });
        expect(vi.getTimerCount()).toBe(1);
        unchecked.dispose();
        expect(vi.getTimerCount()).toBe(0);
    });

    it("starts the unchecked wait over each time the API goes unchecked again", async () => {
        const p = makePane({ api: "unknown" });
        p.failLive();
        await vi.advanceTimersByTimeAsync(UNKNOWN_HOLD_MS - 1_000);
        p.setApi("offline");
        p.setApi("unknown");
        // The first wait would have ended here (and a retry follows a pause later).
        await vi.advanceTimersByTimeAsync(1_001);
        expect(p.sent).toEqual([]);
        await vi.advanceTimersByTimeAsync(UNKNOWN_HOLD_MS);
        expect(p.sent).toHaveLength(1);
        p.dispose();
    });

    it("forgets that the API stayed unchecked once it is known again", async () => {
        const p = makePane({ api: "unknown" });
        await vi.advanceTimersByTimeAsync(UNKNOWN_HOLD_MS);
        p.setApi("offline");
        p.failLive();
        p.setApi("unknown");
        await flush();
        expect(p.sent).toEqual([]);
        await vi.advanceTimersByTimeAsync(UNKNOWN_HOLD_MS + 1);
        expect(p.sent).toHaveLength(1);
        p.dispose();
    });

    it("waits while the CLI is still retrying the API request", async () => {
        const p = makePane({ api: "online", retrying: true });
        p.failLive();
        await flush();
        expect(p.sent).toEqual([]);
        p.setRetrying(false);
        await flush();
        expect(p.sent).toHaveLength(1);
        p.dispose();
    });

    it("doesn't act on a settle timer the conditions have since withdrawn", async () => {
        const p = makePane({ api: "online", settleMs: 1_000 });
        p.failLive();
        await vi.advanceTimersByTimeAsync(500);
        p.setWorking(true);
        await vi.advanceTimersByTimeAsync(1_000);
        expect(p.sent).toEqual([]);
        p.setWorking(false);
        await vi.advanceTimersByTimeAsync(1_000);
        expect(p.sent).toHaveLength(1);
        p.dispose();
    });

    it("doesn't decide before the document is ready, and a failure seen before it is only offered", async () => {
        const p = makePane({ api: "online", ready: false });
        p.failLive();
        p.typeAndSend();
        await flush();
        expect(p.ui.notices()).toEqual([]);
        p.setReady(true);
        await flush();
        expect(offerOf(p)).toBeDefined();
        expect(p.sent).toEqual([]);
        p.dispose();

        // Nor retried by itself: the turn was not seen live.
        const q = makePane({ api: "online", ready: false });
        q.failLive();
        q.setReady(true);
        await flush();
        expect(q.sent).toEqual([]);
        q.dispose();
    });

    it("stops waiting when the API stops being tracked", async () => {
        const p = makePane();
        p.failLive();
        expect(p.ui.pending()).toBe(true);
        p.setTracked(false);
        expect(p.ui.pending()).toBe(false);
        p.setApi("online");
        await flush();
        expect(p.sent).toEqual([]);
        p.dispose();
    });

    it("doesn't wait for a failure that isn't a network one, and drops the wait when one follows", async () => {
        const p = makePane({ api: "online" });
        p.failLive(otherFailure());
        expect(p.ui.pending()).toBe(false);
        await flush();
        expect(p.sent).toEqual([]);
        p.dispose();

        const q = makePane();
        q.failLive();
        expect(q.ui.pending()).toBe(true);
        q.failAfter(otherFailure());
        expect(q.ui.pending()).toBe(false);
        q.setApi("online");
        await flush();
        expect(q.sent).toEqual([]);
        q.dispose();
    });
});

describe("whether a failure is the operator's live turn", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    it("needs a turn that started after the previous failure and no later than this one", async () => {
        // Started when the previous failure landed: that one's turn, not this one's.
        const same = makePane();
        same.failLive();
        same.failLive(network(), { at: 100 });
        same.setApi("online");
        await flush();
        expect(same.sent).toEqual([]);
        same.dispose();

        // Started the moment this one landed: live.
        const exact = makePane({ api: "online" });
        exact.failLive(network(), { at: 100 });
        await flush();
        expect(exact.sent).toHaveLength(1);
        exact.dispose();

        // Started after it landed: some later turn's.
        const later = makePane({ api: "online" });
        later.failLive(network(), { at: 101 });
        await flush();
        expect(later.sent).toEqual([]);
        later.dispose();
    });

    it("isn't retried when the operator had a message held, even once it has gone", async () => {
        const p = makePane({ api: "online" });
        p.setHeld(1);
        p.failLive();
        p.setHeld(0);
        await flush();
        expect(p.sent).toEqual([]);
        p.dispose();
    });

    it("re-sends the turn as it was when it failed, not as the document reads later", async () => {
        const p = makePane({ doc: [BASE] });
        p.failLive();
        p.setDoc([BASE, tool()]);
        p.setApi("online");
        await flush();
        expect(p.sent).toEqual([{ text: "build it", attachments: [PNG] }]);
        p.dispose();
    });

    it("only says it retries by itself for a turn that would", async () => {
        const typed = makePane();
        typed.failLive();
        typed.ui.typed();
        expect(typed.ui.reconnectHint()).toBeNull();
        expect(typed.ui.pending()).toBe(false);
        typed.dispose();

        const readBack = makePane();
        readBack.failReadBack();
        expect(readBack.ui.reconnectHint()).toBeNull();
        expect(readBack.ui.pending()).toBe(false);
        readBack.dispose();

        const retried = makePane({ mark: createSignal<RetryMark | null>(retryMark({ state: "sent" })) });
        retried.failLive();
        expect(retried.ui.reconnectHint()).toBeNull();
        expect(retried.ui.pending()).toBe(false);
        retried.dispose();

        const other = makePane({ mark: createSignal<RetryMark | null>(retryMark({ text: "something else" })) });
        other.failLive();
        expect(other.ui.reconnectHint()).toBe("retries by itself when the API is back");
        other.dispose();
    });
});

describe("a turn the operator wrote after", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    it("is offered only once the row has gone, and decided once", async () => {
        const p = makePane({ api: "online" });
        p.failLive();
        // Typed but not sent: the row still shows its Retry.
        p.ui.typed();
        await flush();
        expect(p.ui.notices()).toEqual([]);
        expect(vi.getTimerCount()).toBe(0);
        // Sent: the row goes, and the offer takes its place.
        p.typeAndSend();
        await flush();
        expect(p.ui.notices().filter((n) => n.retry)).toHaveLength(1);
        expect(vi.getTimerCount()).toBe(0);
        p.dispose();
    });

    it("is not offered when nothing started by the operator is left to re-send", async () => {
        const p = makePane({ api: "online", doc: [user("hi", { timestamp: 500 }), jekt()] });
        p.failAfter();
        p.typeAndSend();
        await flush();
        expect(p.ui.notices()).toEqual([]);
        p.dispose();
    });

    it("is offered as it was when the operator typed, whatever the document gains", async () => {
        const p = makePane({ doc: [BASE] });
        p.failLive();
        p.typeAndSend();
        p.setDoc([BASE, user("u there?", { timestamp: 2_000 })]);
        // Typing again changes nothing about the turn that failed.
        p.ui.typed();
        p.setApi("online");
        await flush();
        expect(offerOf(p).text).toContain("Your message from t1000 failed");
        offerOf(p).retry!();
        await flush();
        expect(p.sent).toEqual([{ text: "build it", attachments: [PNG] }]);
        p.dispose();
    });

    it("is read off the document when the turn was not ready at the failure", async () => {
        const p = makePane({ api: "online", ready: false, doc: [BASE] });
        p.failLive();
        p.typeAndSend();
        p.setDoc([BASE, user("u there?", { timestamp: 2_000 })]);
        p.setReady(true);
        await flush();
        expect(offerOf(p).text).toContain("Your message from t1000 failed");
        offerOf(p).retry!();
        await flush();
        expect(p.sent).toEqual([{ text: "build it", attachments: [PNG] }]);
        p.dispose();
    });

    it("keeps the turn it decided on when the document gains messages later", async () => {
        const p = makePane({ api: "online", ready: false, doc: [BASE] });
        p.setHeld(1);
        p.failLive();
        p.typeAndSend();
        p.setHeld(0);
        p.setReady(true);
        await flush();
        p.setDoc([BASE, user("u there?", { timestamp: 2_000 })]);
        expect(offerOf(p).text).toContain("Your message from t1000 failed");
        p.dispose();
    });

    it("says a retry may have gone already, but only once it is known not to have been answered", async () => {
        const p = makePane();
        p.failLive();
        p.typeAndSend();
        p.setApi("online");
        await flush();
        expect(offerOf(p).text).not.toContain("may have gone");

        p.outcomes.push("pending");
        offerOf(p).retry!();
        await flush();
        expect(offerOf(p).text).not.toContain("may have gone");
        p.answer("unknown");
        await flush();
        expect(offerOf(p).text).toBe(
            "Back online. Your message from t1000 failed while the API was unreachable; it wasn't sent again, since you wrote after it. A retry may have gone already."
        );
        // The offer is under the composer: the row has nothing to say about it.
        expect(p.ui.reconnectHint()).toBeNull();
        p.dispose();
    });

    it("doesn't put the row back for an offered retry that didn't go", async () => {
        const p = makePane();
        p.failLive();
        p.typeAndSend();
        p.setApi("online");
        await flush();
        p.outcomes.push("rejected");
        offerOf(p).retry!();
        await flush();
        expect(p.restoreFailure).not.toHaveBeenCalled();
        expect(offerOf(p)).toBeDefined();
        p.dispose();
    });

    it("keeps an earlier notice ahead of the offer", async () => {
        const p = makePane({ api: "online" });
        p.outcomes.push("unknown");
        p.failLive();
        await flush();
        // The next turn fails and the operator writes before the first retry is confirmed.
        p.setDoc([BASE, user("then this", { timestamp: 2_000 })]);
        p.failLive();
        p.typeAndSend();
        await flush();
        p.late[0](true);
        const list = p.ui.notices();
        expect(list.map((n) => n.text)).toEqual([
            BACK_ONLINE,
            expect.stringContaining("Your message from t2000 failed"),
        ]);
        expect(list[1].retry).toBeDefined();
        p.dispose();
    });

    it("dismisses one notice by itself, and the offer by dropping the turn", async () => {
        const p = makePane({ api: "online" });
        p.failLive();
        await flush();
        p.setDoc([BASE, user("then this", { timestamp: 2_000 })]);
        p.failLive();
        await flush();
        const [first, second] = p.ui.notices();
        p.ui.dismissNotice(first.id);
        expect(p.ui.notices().map((n) => n.id)).toEqual([second.id]);
        p.dispose();

        const q = makePane({ api: "online" });
        q.failLive();
        q.typeAndSend();
        await flush();
        q.ui.dismissNotice(offerOf(q).id);
        expect(q.ui.notices()).toEqual([]);
        expect(q.ui.pending()).toBe(false);
        q.dispose();
    });

    it("typing clears what it did by itself, and a retry that went, and only that", async () => {
        const p = makePane({ api: "online" });
        p.failLive();
        await flush();
        expect(p.ui.notices()).toHaveLength(1);
        expect(p.mark()?.state).toBe("sent");
        p.ui.typed();
        expect(p.ui.notices()).toEqual([]);
        expect(p.mark()).toBeNull();
        p.dispose();

        const q = makePane({ mark: createSignal<RetryMark | null>(retryMark()) });
        q.ui.typed();
        expect(q.mark()?.state).toBe("unknown");
        q.dispose();
    });
});

describe("a retry whose answer is late, wrong or superseded", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    it("ignores an answer for a retry that is no longer the stored one", async () => {
        const sameTime = makePane({ api: "online" });
        sameTime.outcomes.push("pending");
        sameTime.failLive();
        await flush();
        const stored = sameTime.mark()!;
        sameTime.setMark({ ...stored, text: "x" });
        sameTime.answer("accepted");
        await flush();
        expect(sameTime.mark()).toEqual({ ...stored, text: "x" });
        expect(sameTime.clearFailure).not.toHaveBeenCalled();
        sameTime.dispose();

        const sameText = makePane({ api: "online" });
        sameText.outcomes.push("pending");
        sameText.failLive();
        await flush();
        const kept = sameText.mark()!;
        sameText.setMark({ ...kept, at: kept.at + 1 });
        sameText.answer("accepted");
        await flush();
        expect(sameText.mark()).toEqual({ ...kept, at: kept.at + 1 });
        sameText.dispose();
    });

    it("ignores an answer for a retry another mount already settled", async () => {
        const p = makePane({ api: "online" });
        p.outcomes.push("pending");
        p.failLive();
        await flush();
        p.setMark({ ...p.mark()!, state: "sent" });
        p.answer("accepted");
        await flush();
        expect(p.ui.notices()).toEqual([]);
        p.dispose();
    });

    it("takes a late answer only for the stored retry, and only while it is unknown", async () => {
        const replaced = makePane({ api: "online" });
        replaced.outcomes.push("unknown");
        replaced.failLive();
        await flush();
        const stored = replaced.mark()!;
        replaced.setMark({ ...stored, text: "x" });
        replaced.late[0](true);
        expect(replaced.mark()).toEqual({ ...stored, text: "x" });
        expect(replaced.clearFailure).not.toHaveBeenCalled();
        replaced.dispose();

        const settled = makePane({ api: "online" });
        settled.outcomes.push("unknown");
        settled.failLive();
        await flush();
        settled.setMark({ ...settled.mark()!, state: "sent" });
        settled.late[0](false);
        expect(settled.mark()?.state).toBe("sent");
        settled.dispose();
    });

    it("goes back to the earlier mark when the late answer says it didn't go", async () => {
        const p = makePane({ api: "online" });
        p.outcomes.push("unknown");
        p.failLive();
        await flush();
        p.late[0](false);
        expect(p.mark()).toBeNull();
        expect(p.clearFailure).not.toHaveBeenCalled();
        expect(p.ui.notices()).toEqual([]);
        p.dispose();
    });

    it("says a late-confirmed automatic retry went", async () => {
        const p = makePane({ api: "online" });
        p.outcomes.push("unknown");
        p.failLive();
        await flush();
        expect(p.ui.notices()).toEqual([]);
        p.late[0](true);
        expect(p.ui.notices().map((n) => n.text)).toEqual([BACK_ONLINE]);
        p.dispose();
    });

    it("says it may have gone twice when a retry that may have gone is retried and confirmed late", async () => {
        const p = makePane({ api: "online" });
        p.outcomes.push("unknown", "unknown");
        p.failLive();
        await flush();
        expect(p.ui.retry()).toBe(true);
        await flush();
        p.late[1](true);
        expect(p.ui.notices().map((n) => n.text)).toEqual(["Sent your message again. It may have gone once already."]);
        p.dispose();
    });

    it("keeps a retry the operator made by hand a hand retry, even after one that may have gone", async () => {
        const p = makePane();
        p.outcomes.push("unknown");
        p.failLive();
        expect(p.ui.retry()).toBe(true);
        await flush();
        expect(p.mark()).toMatchObject({ auto: false, state: "unknown" });
        expect(p.ui.retry()).toBe(true);
        await flush();
        expect(p.mark()).toMatchObject({ auto: false, state: "sent" });
        p.dispose();
    });

    it("puts the row back unless the turn it started has settled", async () => {
        // Another message's retry is not this one's.
        const other = makePane({ api: "online", startsTurn: true });
        other.outcomes.push("unknown");
        other.failLive();
        await flush();
        other.setMark({ text: "other", at: 6_000, auto: false, state: "sent" });
        other.setWorking(false);
        expect(other.restoreFailure).toHaveBeenCalledTimes(1);
        other.dispose();

        // This one's own, settled by another mount while the turn ran.
        const same = makePane({ api: "online", startsTurn: true });
        same.outcomes.push("unknown");
        same.failLive();
        await flush();
        same.setMark({ text: "build it", at: 5_000, auto: true, state: "sent" });
        same.setWorking(false);
        expect(same.restoreFailure).not.toHaveBeenCalled();
        same.dispose();
    });

    it("forgets the retry's mark when the row is dismissed, unless a retry is on its way", async () => {
        const p = makePane({ api: "online" });
        p.outcomes.push("unknown");
        p.failLive();
        await flush();
        p.dismissRow();
        expect(p.mark()).toBeNull();
        p.dispose();

        const q = makePane({ api: "online" });
        q.outcomes.push("pending");
        q.failLive();
        await flush();
        q.dismissRow();
        expect(q.mark()?.state).toBe("sending");
        q.dispose();
    });

    it("is busy only while a retry of the turn it tracks is on its way", () => {
        const flying = makePane({ mark: createSignal<RetryMark | null>(retryMark({ state: "sending", at: 4_000 })) });
        expect(flying.ui.retryBusy()).toBe(false);
        flying.failLive();
        expect(flying.ui.retryBusy()).toBe(true);
        flying.dispose();

        const idle = makePane();
        idle.failLive();
        expect(idle.ui.retryBusy()).toBe(false);
        idle.dispose();
    });
});

describe("a retry an earlier mount had on its way, and the transcript", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    it("settles this mount's failure only when that same retry went", async () => {
        const shared = createSignal<RetryMark | null>(null);
        const first = makePane({ api: "online", mark: shared });
        first.outcomes.push("pending");
        first.failLive();
        await flush();
        first.dispose();

        const second = makePane({ api: "online", mark: shared });
        second.failReadBack();
        await flush();
        // Another retry, at another time, went.
        shared[1]({ text: "build it", at: 9_999, auto: true, state: "sent" });
        await flush();
        expect(second.clearFailure).not.toHaveBeenCalled();
        // The same one, still not answered.
        shared[1]({ text: "build it", at: 5_000, auto: true, state: "unknown" });
        await flush();
        expect(second.clearFailure).not.toHaveBeenCalled();
        // The same one went: so did this failure, and it says the retry was automatic.
        shared[1]({ text: "build it", at: 5_000, auto: true, state: "sent" });
        await flush();
        expect(second.clearFailure).toHaveBeenCalledTimes(1);
        expect(second.ui.notices().map((n) => n.text)).toEqual([BACK_ONLINE]);
        second.dispose();
    });

    it("finds an unanswered retry in the transcript only when ready, and only the same message after it", async () => {
        const p = makePane({ api: "online" });
        p.outcomes.push("unknown");
        p.failLive();
        await flush();
        const notFound = async (doc: DocumentNode[]) => {
            p.setDoc(doc);
            await flush();
            expect(p.mark()?.state).toBe("unknown");
        };
        const agent = {
            type: "agent_message",
            id: "a",
            message: "build it",
            timestamp: 5_100,
            direction: "incoming",
        } as unknown as DocumentNode;
        await notFound([BASE, agent]);
        await notFound([BASE, user("something else", { timestamp: 5_100 })]);
        await notFound([BASE, user("build it")]);
        await notFound([BASE, user("build it", { timestamp: 4_000 })]);
        // Not before the document is ready.
        p.setReady(false);
        p.setDoc([BASE, user("build it", { timestamp: 5_000 })]);
        await flush();
        expect(p.mark()?.state).toBe("unknown");
        // At the very time of the retry counts.
        p.setReady(true);
        await flush();
        expect(p.mark()?.state).toBe("sent");
        // A transcript is not a notice: nothing says AgentMux did it.
        expect(p.ui.notices()).toEqual([]);
        p.dispose();
    });

    it("leaves the turn alone when the retry found is another message's", async () => {
        const mark = createSignal<RetryMark | null>(retryMark({ text: "other", auto: false }));
        const p = makePane({ mark, ready: false, doc: [user("other", { timestamp: 6_000 }), BASE] });
        p.failLive();
        p.setReady(true);
        await flush();
        expect(p.mark()?.state).toBe("sent");
        expect(p.clearFailure).not.toHaveBeenCalled();
        p.dispose();
    });
});

describe("a turn completing", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    const pane = (m: Partial<RetryMark> = {}, started: Partial<TurnStarted> | null = {}) => {
        const p = makePane({ mark: createSignal<RetryMark | null>(retryMark(m)) });
        p.failLive();
        p.setStarted(started === null ? null : { at: 6_000, content: "build it", hidden: false, ...started });
        return p;
    };

    it("settles the retry it started, and the turn with it", () => {
        const p = pane();
        p.ui.turnCompleted();
        expect(p.mark()).toBeNull();
        expect(p.ui.pending()).toBe(false);
        expect(p.ui.notices().map((n) => n.text)).toEqual([BACK_ONLINE]);
        p.dispose();

        // At the very time the retry was sent counts too.
        const exact = pane({}, { at: 5_000 });
        exact.ui.turnCompleted();
        expect(exact.mark()).toBeNull();
        exact.dispose();
    });

    it("settles nothing for a turn that isn't that retry's", () => {
        for (const started of [null, { hidden: true }, { content: "other" }, { at: 4_000 }]) {
            const p = pane({}, started);
            p.ui.turnCompleted();
            expect(p.mark()?.state).toBe("unknown");
            expect(p.ui.pending()).toBe(false);
            expect(p.ui.notices()).toEqual([]);
            p.dispose();
        }
    });

    it("clears a retry that already went, without settling the turn again", () => {
        const p = pane({ state: "sent" });
        p.ui.turnCompleted();
        expect(p.mark()).toBeNull();
        expect(p.ui.notices()).toEqual([]);
        p.dispose();
    });

    it("settles another message's retry without touching this turn", () => {
        const p = pane({ text: "other" }, { content: "other" });
        p.ui.turnCompleted();
        expect(p.mark()).toBeNull();
        expect(p.ui.notices()).toEqual([]);
        expect(p.clearFailure).not.toHaveBeenCalled();
        p.dispose();
    });
});
