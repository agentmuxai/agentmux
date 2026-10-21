// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Reachability } from "@/types/rpc/Reachability";
import type { DocumentNode } from "../types";
import {
    type DeliveryOutcome,
    confirmedSend,
    createOfflineInput,
    type HeldInput,
    heldForText,
    holdsInput,
    isLocalCommand,
    isTrackedService,
    keyedLock,
    modelApiService,
    type OfflineInput,
    type PaneMessageEvent,
    sentInTranscript,
    serviceReachability,
} from "./offline-input";

const user = (message: string, timestamp?: number): DocumentNode =>
    ({ type: "user_message", id: `u-${message}`, message, ...(timestamp ? { timestamp } : {}) }) as DocumentNode;


describe("pure rules", () => {
    it("names the provider's model API by its first vendor", () => {
        expect(modelApiService(["anthropic"])).toBe("model_api:anthropic");
        expect(modelApiService(undefined)).toBeNull();
        expect(modelApiService([])).toBeNull();
    });

    it("reads a tracked service's state, and null for one srv doesn't track", () => {
        const services = [{ service: "model_api:anthropic", label: "Anthropic API", state: "offline" as const, since_ms: 1, last_error: null }];
        expect(serviceReachability(services, "model_api:anthropic")).toBe("offline");
        expect(serviceReachability(services, "model_api:openai")).toBeNull();
        expect(serviceReachability(undefined, "model_api:anthropic")).toBeNull();
    });

    it("knows srv tracks Claude's API before any status arrives, and others once they report", () => {
        expect(isTrackedService("model_api:anthropic", undefined)).toBe(true);
        expect(isTrackedService("model_api:openai", undefined)).toBe(false);
        const services = [{ service: "model_api:openai", label: "OpenAI API", state: "online" as const, since_ms: 1, last_error: null }];
        expect(isTrackedService("model_api:openai", services)).toBe(true);
        expect(isTrackedService(null, services)).toBe(false);
    });

    it("finds a held message in the transcript only when sent after it was held", () => {
        const item = { id: "h", text: "build it", heldAt: 1_000 };
        expect(sentInTranscript([user("build it", 1_500)], item)).toBe(true);
        expect(sentInTranscript([user("build it", 500)], item)).toBe(false);
        expect(sentInTranscript([user("build it")], item)).toBe(false);
        expect(sentInTranscript([user("other", 1_500)], item)).toBe(false);
    });

    it("holds while the turn retries or the API is offline, never while it is online or degraded", () => {
        expect(holdsInput(true, "online")).toBe(true);
        expect(holdsInput(false, "offline")).toBe(true);
        expect(holdsInput(false, "online")).toBe(false);
        expect(holdsInput(false, "degraded")).toBe(false);
        expect(holdsInput(false, null)).toBe(false);
    });

    it("never holds a command that runs in the pane", () => {
        expect(isLocalCommand("/help")).toBe(true);
        expect(isLocalCommand("  !git status")).toBe(true);
        expect(isLocalCommand("hello")).toBe(false);
        expect(isLocalCommand("/look at this", [{ id: "i", name: "a.png" }])).toBe(false);
    });

    it("says how long a message was held", () => {
        expect(heldForText(20_000)).toBe("held under a minute");
        expect(heldForText(4 * 60_000)).toBe("held 4 min");
    });
});

describe("confirmedSend", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    /** A send path that queues an entry, then reports `outcome` for it. */
    const harness = (outcome: "accepted" | "rejected" | "expired" | "none", busy = false) => {
        const sinks = new Set<(e: PaneMessageEvent) => void>();
        const emit = (e: PaneMessageEvent) => sinks.forEach((cb) => cb(e));
        const send = vi.fn(async () => {
            if (outcome === "none") return; // handled in the pane: no entry
            emit({ type: "pending-queued", id: "m1" });
            // Another message's events must not count for this one.
            emit({ type: "pending-accepted", id: "other" });
            await Promise.resolve();
            emit({ type: `pending-${outcome}`, id: "m1" });
        });
        const deliver = confirmedSend(
            send,
            (cb) => {
                sinks.add(cb);
                return () => sinks.delete(cb);
            },
            () => busy,
            5_000
        );
        return { send, deliver, sinks };
    };

    it("is true once srv accepted the message", async () => {
        const h = harness("accepted");
        await expect(h.deliver("hi")).resolves.toBe("accepted");
        expect(h.sinks.size).toBe(0);
    });

    it("is false when the send path rejected it or lost it", async () => {
        await expect(harness("rejected").deliver("hi")).resolves.toBe("rejected");
    });

    it("is unknown when the send path gives up waiting, and still hears srv's late acceptance", async () => {
        const sinks = new Set<(e: PaneMessageEvent) => void>();
        const emit = (e: PaneMessageEvent) => sinks.forEach((cb) => cb(e));
        const deliver = confirmedSend(
            async () => {
                emit({ type: "pending-queued", id: "m1" });
                emit({ type: "pending-expired", id: "m1" });
            },
            (cb) => {
                sinks.add(cb);
                return () => sinks.delete(cb);
            },
            () => false,
            120_000
        );
        const late = vi.fn();
        await expect(deliver("hi", undefined, late)).resolves.toBe("unknown");
        emit({ type: "pending-accepted", id: "m1" });
        expect(late).toHaveBeenCalledWith(true);
    });

    it("is false when nothing was queued for srv", async () => {
        await expect(harness("none").deliver("hi")).resolves.toBe("rejected");
    });

    it("doesn't send while a turn runs: the message would only join the pane's own queue", async () => {
        const h = harness("accepted", true);
        await expect(h.deliver("hi")).resolves.toBe("rejected");
        expect(h.send).not.toHaveBeenCalled();
    });

    it("is unknown when no answer comes in time, and hands a late answer on", async () => {
        const sinks = new Set<(e: PaneMessageEvent) => void>();
        const emit = (e: PaneMessageEvent) => sinks.forEach((cb) => cb(e));
        const deliver = confirmedSend(
            async () => emit({ type: "pending-queued", id: "m1" }),
            (cb) => {
                sinks.add(cb);
                return () => sinks.delete(cb);
            },
            () => false,
            5_000
        );
        const late = vi.fn();
        const result = deliver("hi", undefined, late);
        await vi.advanceTimersByTimeAsync(5_000);
        await expect(result).resolves.toBe("unknown");
        expect(late).not.toHaveBeenCalled();
        emit({ type: "pending-accepted", id: "m1" });
        expect(late).toHaveBeenCalledWith(true);
        expect(sinks.size).toBe(0);
    });
});

/** A pane: plain signals for what the controller reads, a recording send. */
function makePane(
    opts: { api?: Reachability | null; tracked?: boolean; statusKnown?: boolean; held?: HeldInput[] } = {}
) {
    const [retrying, setRetrying] = createSignal(false);
    const [api, setApi] = createSignal<Reachability | null>(opts.api === undefined ? "online" : opts.api);
    const [tracked, setTracked] = createSignal(opts.tracked ?? opts.api !== null);
    const [statusKnown, setStatusKnown] = createSignal(opts.statusKnown ?? true);
    const [ready, setReady] = createSignal(true);
    const [working, setWorking] = createSignal(false);
    let doc: DocumentNode[] = [];
    const [held, setHeld] = createSignal<HeldInput[]>(opts.held ?? []);
    const store = {
        get held() {
            return held();
        },
    };
    const sent: string[] = [];
    const stop = vi.fn();
    const send = vi.fn(async (text: string) => {
        sent.push(text);
    });
    // As confirmedSend over that send: whether srv took it.
    const accepts: DeliveryOutcome[] = [];
    /** The late-answer callback of each delivery that came back `unknown`. */
    const late: ((accepted: boolean) => void)[] = [];
    const deliver = vi.fn(async (text: string, _attachments?: unknown, onLate?: (accepted: boolean) => void): Promise<DeliveryOutcome> => {
        const outcome = accepts.length > 0 ? accepts.shift()! : "accepted";
        if (outcome !== "rejected") sent.push(text);
        if (outcome === "unknown" && onLate) late.push(onLate);
        return outcome;
    });
    let ui!: OfflineInput;
    const dispose = createRoot((d) => {
        ui = createOfflineInput({
            retrying,
            api,
            tracked,
            statusKnown,
            ready,
            working,
            document: () => doc,
            send,
            deliver,
            stop,
            storage: { held, saveHeld: setHeld },
            lock: keyedLock(new Map(), "b"),
            stopWaitMs: 2_000,
        });
        return d;
    });
    return {
        ui,
        sent,
        send,
        deliver,
        accepts,
        late,
        stop,
        store,
        dispose,
        setRetrying,
        setApi,
        setTracked,
        setStatusKnown,
        setReady,
        setWorking,
        setDoc: (d: DocumentNode[]) => {
            doc = d;
        },
    };
}

const flush = () => vi.advanceTimersByTimeAsync(0);

describe("held input", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    it("sends at once while the API is online", async () => {
        const p = makePane();
        await p.ui.submit("hello");
        expect(p.sent).toEqual(["hello"]);
        expect(p.ui.held()).toEqual([]);
        p.dispose();
    });

    it("holds while the API is offline and sends in order, once, when it is back", async () => {
        const p = makePane({ api: "offline" });
        await p.ui.submit("one");
        await p.ui.submit("two");
        expect(p.sent).toEqual([]);
        expect(p.ui.held().map((m) => m.text)).toEqual(["one", "two"]);
        expect(p.store.held.map((m) => m.text)).toEqual(["one", "two"]);

        p.setApi("online");
        await flush();
        expect(p.sent).toEqual(["one", "two"]);
        expect(p.ui.held()).toEqual([]);
        expect(p.store.held).toEqual([]);
        expect(p.ui.notices().map((n) => n.text)).toEqual(["Sent your 2 held messages (held under a minute)."]);

        // Flapping back and forth sends nothing again.
        p.setApi("offline");
        p.setApi("online");
        await flush();
        expect(p.sent).toEqual(["one", "two"]);
        p.dispose();
    });

    it("holds while the turn retries, and releases when the retry clears", async () => {
        const p = makePane();
        p.setRetrying(true);
        await p.ui.submit("u there?");
        expect(p.sent).toEqual([]);
        p.setRetrying(false);
        await flush();
        expect(p.sent).toEqual(["u there?"]);
        p.dispose();
    });

    it("keeps a message typed during a release behind the ones before it", async () => {
        const p = makePane({ api: "offline" });
        await p.ui.submit("one");
        p.setApi("online");
        // Typed before the release ran: joins the queue rather than jumping it.
        await p.ui.submit("two");
        await flush();
        expect(p.sent).toEqual(["one", "two"]);
        p.dispose();
    });

    it("stops releasing when the API goes away again mid-release", async () => {
        const p = makePane({ api: "offline" });
        await p.ui.submit("one");
        await p.ui.submit("two");
        p.deliver.mockImplementationOnce(async (text: string) => {
            p.sent.push(text);
            p.setRetrying(true);
            return "accepted" as const;
        });
        p.setApi("online");
        await flush();
        expect(p.sent).toEqual(["one"]);
        expect(p.ui.held().map((m) => m.text)).toEqual(["two"]);
        p.dispose();
    });

    it("keeps a held message srv didn't accept, says so, and doesn't count it as sent", async () => {
        const p = makePane({ api: "offline" });
        await p.ui.submit("one");
        await p.ui.submit("two");
        p.accepts.push("rejected");
        p.setApi("online");
        await flush();
        expect(p.sent).toEqual([]);
        expect(p.store.held.map((m) => m.text)).toEqual(["one", "two"]);
        expect(p.ui.held().map((m) => m.text)).toEqual(["one", "two"]);
        expect(p.ui.notices().map((n) => n.text)).toEqual([
            "Couldn't send your held message, so it is still held. Send now tries again.",
        ]);
        // No retry loop: a turn starting and ending around the rejection
        // doesn't send it again by itself.
        p.setWorking(true);
        p.setWorking(false);
        await flush();
        expect(p.deliver).toHaveBeenCalledTimes(1);
        // Send now tries again, in order.
        await p.ui.sendNow();
        await flush();
        expect(p.sent).toEqual(["one", "two"]);
        expect(p.store.held).toEqual([]);
        p.dispose();
    });

    it("keeps the held message stored, but out of the list, while it is being sent", async () => {
        const p = makePane({ api: "offline" });
        await p.ui.submit("one");
        let accept!: (outcome: DeliveryOutcome) => void;
        p.deliver.mockImplementationOnce(() => new Promise<DeliveryOutcome>((r) => (accept = r)));
        p.setApi("online");
        await flush();
        expect(p.store.held.map((m) => m.text)).toEqual(["one"]);
        expect(p.ui.held()).toEqual([]);
        // A new message waits behind it.
        await p.ui.submit("two");
        expect(p.ui.held().map((m) => m.text)).toEqual(["two"]);
        accept("accepted");
        await flush();
        expect(p.store.held.map((m) => m.text)).toEqual([]);
        expect(p.deliver.mock.calls.map((c) => c[0])).toEqual(["one", "two"]);
        p.dispose();
    });

    it("sends the next held message only once the turn the last one started has ended", async () => {
        const p = makePane({ api: "offline" });
        await p.ui.submit("one");
        await p.ui.submit("two");
        p.deliver.mockImplementationOnce(async (text: string) => {
            p.sent.push(text);
            p.setWorking(true);
            return "accepted" as const;
        });
        p.setApi("online");
        await flush();
        expect(p.sent).toEqual(["one"]);
        expect(p.store.held.map((m) => m.text)).toEqual(["two"]);
        p.setWorking(false);
        await flush();
        expect(p.sent).toEqual(["one", "two"]);
        expect(p.ui.notices().map((n) => n.text)).toEqual(["Sent your 2 held messages (held under a minute)."]);
        p.dispose();
    });

    it("doesn't send again a held message whose answer never came; a late acceptance drops it", async () => {
        const p = makePane({ api: "offline" });
        await p.ui.submit("one");
        await p.ui.submit("two");
        p.accepts.push("unknown");
        p.setApi("online");
        await flush();
        expect(p.deliver).toHaveBeenCalledTimes(1);
        const [one, two] = p.store.held;
        expect(one).toMatchObject({ text: "one", unconfirmed: true });
        expect(two.text).toBe("two");
        expect(p.ui.notices().map((n) => n.text)).toEqual([
            "No answer yet for your held message, so it may have gone. It stays here and isn't sent again by itself.",
        ]);
        // Turns come and go: neither it nor what follows it goes by itself.
        p.setWorking(true);
        p.setWorking(false);
        await flush();
        expect(p.deliver).toHaveBeenCalledTimes(1);
        // srv's answer comes after all: it went, so it leaves the list, and
        // the next one follows.
        p.late[0](true);
        await flush();
        expect(p.deliver.mock.calls.map((c) => c[0])).toEqual(["one", "two"]);
        expect(p.store.held).toEqual([]);
        p.dispose();
    });

    it("holds again, as not sent, a message whose late answer is a rejection", async () => {
        const p = makePane({ api: "offline" });
        await p.ui.submit("one");
        p.accepts.push("unknown");
        p.setApi("online");
        await flush();
        p.late[0](false);
        await flush();
        expect(p.store.held).toEqual([expect.objectContaining({ text: "one", unconfirmed: false })]);
        expect(p.deliver).toHaveBeenCalledTimes(1);
        p.dispose();
    });

    it("Send now sends one whose answer never came, and says it may have gone once already", async () => {
        const p = makePane({ api: "offline" });
        await p.ui.submit("one");
        p.accepts.push("unknown");
        p.setApi("online");
        await flush();
        await p.ui.sendNow();
        await flush();
        expect(p.deliver).toHaveBeenCalledTimes(2);
        expect(p.store.held).toEqual([]);
        expect(p.ui.notices().map((n) => n.text)).toContain("Sent your held message again. It may have gone once already.");
        // A late answer for the first send changes nothing now.
        p.late[0](true);
        expect(p.store.held).toEqual([]);
        p.dispose();
    });

    it("stores a held message as possibly gone before it goes, so a reload doesn't send it again", async () => {
        const p = makePane({ api: "offline" });
        await p.ui.submit("one");
        let answer!: (outcome: DeliveryOutcome) => void;
        p.deliver.mockImplementationOnce(() => new Promise<DeliveryOutcome>((r) => (answer = r)));
        p.setApi("online");
        await flush();
        expect(p.store.held).toEqual([expect.objectContaining({ text: "one", unconfirmed: true })]);
        answer("rejected");
        await flush();
        // A definite rejection: it didn't go, so it is held as unsent.
        expect(p.store.held).toEqual([expect.objectContaining({ text: "one", unconfirmed: false })]);
        p.dispose();
    });

    it("after a reload, shows one stored mid-send as possibly gone and doesn't send it, nor what follows", async () => {
        const p = makePane({
            held: [
                { id: "a", text: "one", heldAt: 1_000, unconfirmed: true },
                { id: "b", text: "two", heldAt: 1_000 },
            ],
        });
        await vi.advanceTimersByTimeAsync(10_000);
        expect(p.deliver).not.toHaveBeenCalled();
        expect(p.ui.held().map((m) => [m.text, !!m.unconfirmed])).toEqual([
            ["one", true],
            ["two", false],
        ]);
        p.dispose();
    });

    it("drops one stored mid-send once it shows up in the transcript, then sends the rest", async () => {
        const p = makePane({
            held: [
                { id: "a", text: "one", heldAt: 1_000, unconfirmed: true },
                { id: "b", text: "two", heldAt: 1_000 },
            ],
        });
        p.setReady(false);
        p.setDoc([user("one", 2_000)]);
        p.setReady(true);
        await flush();
        expect(p.deliver.mock.calls.map((c) => c[0])).toEqual(["two"]);
        expect(p.store.held).toEqual([]);
        p.dispose();
    });

    it("Cancel drops a held message and saves the list", async () => {
        const p = makePane({ api: "offline" });
        await p.ui.submit("one");
        await p.ui.submit("two");
        p.ui.cancel(p.ui.held()[0].id);
        expect(p.store.held.map((m) => m.text)).toEqual(["two"]);
        p.setApi("online");
        await flush();
        expect(p.sent).toEqual(["two"]);
        p.dispose();
    });

    it("Send now stops the retrying turn, waits for it to end, then sends in order while still offline", async () => {
        const p = makePane({ api: "offline" });
        p.setWorking(true);
        p.setRetrying(true);
        await p.ui.submit("one");
        await p.ui.submit("two");
        const done = p.ui.sendNow();
        expect(p.stop).toHaveBeenCalledTimes(1);
        await vi.advanceTimersByTimeAsync(300);
        expect(p.sent).toEqual([]);
        p.setWorking(false);
        p.setRetrying(false);
        await vi.advanceTimersByTimeAsync(200);
        await done;
        expect(p.sent).toEqual(["one", "two"]);
        expect(p.ui.held()).toEqual([]);
        p.dispose();
    });

    it("Send now with no turn running sends without stopping anything", async () => {
        const p = makePane({ api: "offline" });
        await p.ui.submit("one");
        await p.ui.sendNow();
        expect(p.stop).not.toHaveBeenCalled();
        expect(p.sent).toEqual(["one"]);
        p.dispose();
    });

    it("sends what an earlier session held once the pane can send (restart)", async () => {
        const held: HeldInput[] = [
            { id: "a", text: "before the restart", heldAt: 0 },
            { id: "b", text: "and this", heldAt: 0 },
        ];
        const p = makePane({ held });
        p.setReady(false);
        await flush();
        // Mounted while history loads: nothing goes until the pane can send.
        expect(p.ui.held().map((m) => m.text)).toEqual(["before the restart", "and this"]);
        expect(p.sent).toEqual([]);
        p.setReady(true);
        await flush();
        expect(p.sent).toEqual(["before the restart", "and this"]);
        p.dispose();
    });

    it("never holds a pane command, which works offline", async () => {
        const p = makePane({ api: "offline" });
        await p.ui.submit("/help");
        await p.ui.submit("!git status");
        expect(p.sent).toEqual(["/help", "!git status"]);
        expect(p.ui.held()).toEqual([]);
        p.dispose();
    });

    it("drops what it said it did once the operator types", async () => {
        const p = makePane({ api: "offline" });
        await p.ui.submit("one");
        p.setApi("online");
        await flush();
        expect(p.ui.notices()).toHaveLength(1);
        await p.ui.submit("two");
        expect(p.ui.notices()).toEqual([]);
        p.dispose();
    });

    it("doesn't release into a tracked API whose state isn't known yet, as on reopening", async () => {
        const p = makePane({ api: null, tracked: true, statusKnown: false, held: [{ id: "a", text: "one", heldAt: 1 }] });
        await vi.advanceTimersByTimeAsync(10_000);
        expect(p.deliver).not.toHaveBeenCalled();
        // The first status says the API is still offline: still held.
        p.setApi("offline");
        p.setStatusKnown(true);
        await flush();
        expect(p.deliver).not.toHaveBeenCalled();
        p.setApi("online");
        await flush();
        expect(p.sent).toEqual(["one"]);
        p.dispose();
    });

    it("releases for an untracked provider without waiting for a status", async () => {
        const p = makePane({ api: null, tracked: false, statusKnown: false, held: [{ id: "a", text: "one", heldAt: 1 }] });
        await flush();
        expect(p.sent).toEqual(["one"]);
        p.dispose();
    });

    it("never holds for a provider srv doesn't track", async () => {
        const p = makePane({ api: null });
        await p.ui.submit("hi");
        expect(p.sent).toEqual(["hi"]);
        p.dispose();
    });
});
