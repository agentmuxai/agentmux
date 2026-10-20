// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PaneFailure } from "@/app/store/agent-pane-state/types";
import type { Reachability } from "@/types/rpc/Reachability";
import type { DocumentNode } from "../types";
import {
    type AutoRetryMarker,
    createOfflineInput,
    decideOnReconnect,
    failedTurnSnapshot,
    type HeldInput,
    heldForText,
    holdsInput,
    isLocalCommand,
    keyedLock,
    modelApiService,
    type OfflineInput,
    serviceReachability,
} from "./offline-input";

const user = (message: string, timestamp?: number): DocumentNode =>
    ({ type: "user_message", id: `u-${message}`, message, ...(timestamp ? { timestamp } : {}) }) as DocumentNode;
const tool = (): DocumentNode => ({ type: "tool", id: "t" }) as unknown as DocumentNode;
const jekt = (): DocumentNode => ({ type: "jekt_message", id: "j", message: "hi" }) as unknown as DocumentNode;
const networkFailure = (at: number): PaneFailure => ({
    data: { code: "network", title: "Can't reach the API", detail: "", retryable: true },
    at,
    turnAttempted: true,
});

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

    it("reads the failed turn off the document", () => {
        expect(failedTurnSnapshot([user("a"), user("b")])).toEqual({ text: "b", toolRan: false });
        expect(failedTurnSnapshot([user("a"), tool(), user("b"), tool()])).toEqual({ text: "b", toolRan: true });
        // A tool before the message a retry re-sends doesn't count.
        expect(failedTurnSnapshot([user("a"), tool(), user("b")])).toEqual({ text: "b", toolRan: false });
        // Started by another agent: nothing to re-send.
        expect(failedTurnSnapshot([user("a"), jekt()])).toEqual({ text: null, toolRan: false });
        expect(failedTurnSnapshot([])).toEqual({ text: null, toolRan: false });
        expect(failedTurnSnapshot([user("b", 1234)])).toEqual({ text: "b", toolRan: false, sentAt: 1234 });
    });

    it("retries only a clean turn nobody has typed after, once, with the setting on", () => {
        const turn = { text: "go", toolRan: false };
        const base = { turn, typedSince: false, autoRetryOn: true, marker: null };
        expect(decideOnReconnect(base)).toEqual({ kind: "retry" });
        expect(decideOnReconnect({ ...base, typedSince: true })).toEqual({ kind: "offer", reason: "typed" });
        expect(decideOnReconnect({ ...base, turn: { text: "go", toolRan: true } })).toEqual({ kind: "offer", reason: "tool_ran" });
        expect(decideOnReconnect({ ...base, autoRetryOn: false })).toEqual({ kind: "offer", reason: "setting_off" });
        expect(decideOnReconnect({ ...base, marker: { text: "go", at: 1 } })).toEqual({ kind: "offer", reason: "retried_once" });
        expect(decideOnReconnect({ ...base, marker: { text: "other", at: 1 } })).toEqual({ kind: "retry" });
        expect(decideOnReconnect({ ...base, turn: { text: null, toolRan: false } })).toEqual({
            kind: "offer",
            reason: "nothing_to_resend",
        });
    });

    it("says how long a message was held", () => {
        expect(heldForText(20_000)).toBe("held under a minute");
        expect(heldForText(4 * 60_000)).toBe("held 4 min");
    });
});

/** A pane: plain signals for what the controller reads, a recording send. */
function makePane(opts: { api?: Reachability | null; held?: HeldInput[]; marker?: AutoRetryMarker | null } = {}) {
    const [retrying, setRetrying] = createSignal(false);
    const [api, setApi] = createSignal<Reachability | null>(opts.api === undefined ? "online" : opts.api);
    const [ready, setReady] = createSignal(true);
    const [working, setWorking] = createSignal(false);
    const [failure, setFailure] = createSignal<PaneFailure | null>(null);
    const [autoRetryOn, setAutoRetryOn] = createSignal(true);
    let doc: DocumentNode[] = [];
    const store = { held: opts.held ?? [], marker: opts.marker ?? null, savedHeld: [] as HeldInput[][] };
    const sent: string[] = [];
    const stop = vi.fn();
    // As the pane's send: the turn starts, which clears the failure row.
    const send = vi.fn(async (text: string) => {
        sent.push(text);
        setFailure(null);
    });
    let ui!: OfflineInput;
    const dispose = createRoot((d) => {
        ui = createOfflineInput({
            retrying,
            api,
            ready,
            working,
            failure,
            document: () => doc,
            autoRetryOn,
            send,
            stop,
            storage: {
                loadHeld: () => store.held,
                saveHeld: (h) => {
                    store.held = h;
                    store.savedHeld.push(h);
                },
                loadMarker: () => store.marker,
                saveMarker: (m) => {
                    store.marker = m;
                },
            },
            lock: keyedLock(new Map(), "b"),
            settleMs: () => 1_000,
            stopWaitMs: 2_000,
            clockTime: () => "14:32",
        });
        return d;
    });
    return {
        ui,
        sent,
        send,
        stop,
        store,
        dispose,
        setRetrying,
        setApi,
        setReady,
        setWorking,
        setFailure,
        setAutoRetryOn,
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
        p.send.mockImplementationOnce(async (text: string) => {
            p.sent.push(text);
            p.setRetrying(true);
        });
        p.setApi("online");
        await flush();
        expect(p.sent).toEqual(["one"]);
        expect(p.ui.held().map((m) => m.text)).toEqual(["two"]);
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

    it("never holds for a provider srv doesn't track", async () => {
        const p = makePane({ api: null });
        await p.ui.submit("hi");
        expect(p.sent).toEqual(["hi"]);
        expect(p.ui.waitsForReconnect("network")).toBe(false);
        p.dispose();
    });
});

describe("a turn that failed on the network", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    /** Offline, with a turn that failed before any tool ran. */
    const failedOffline = (opts: Parameters<typeof makePane>[0] = {}) => {
        const p = makePane({ api: "offline", ...opts });
        p.setDoc([user("earlier"), user("build it", 777)]);
        p.setFailure(networkFailure(1_000));
        return p;
    };

    it("waits for the API instead of the failure row's timed retries", () => {
        const p = failedOffline();
        expect(p.ui.waitsForReconnect("network")).toBe(true);
        expect(p.ui.waitsForReconnect("rate_limited")).toBe(false);
        expect(p.ui.reconnectHint()).toBe("retries by itself when the API is back");
        p.dispose();
    });

    it("retries it once by itself after the API answers, and says so", async () => {
        const p = failedOffline();
        p.setApi("online");
        await vi.advanceTimersByTimeAsync(999);
        expect(p.sent).toEqual([]);
        await vi.advanceTimersByTimeAsync(1);
        expect(p.sent).toEqual(["build it"]);
        expect(p.ui.notices().map((n) => n.text)).toEqual([
            "Back online: sent your message from 14:32 again, since its turn failed while the API was unreachable.",
        ]);
        expect(p.store.marker?.text).toBe("build it");
        await vi.advanceTimersByTimeAsync(10_000);
        expect(p.sent).toEqual(["build it"]);
        p.dispose();
    });

    it("doesn't retry automatically a second time when the retried turn fails too", async () => {
        const p = failedOffline({ marker: { text: "build it", at: 900 } });
        p.setApi("online");
        await vi.advanceTimersByTimeAsync(5_000);
        expect(p.sent).toEqual([]);
        // The failure row, still showing, offers Retry.
        expect(p.ui.notices()).toEqual([]);
        p.dispose();
    });

    it("waits for the API to be online, not just reachable again in part", async () => {
        const p = failedOffline();
        p.setApi("degraded");
        await vi.advanceTimersByTimeAsync(5_000);
        expect(p.sent).toEqual([]);
        p.setApi("online");
        await vi.advanceTimersByTimeAsync(1_000);
        expect(p.sent).toEqual(["build it"]);
        p.dispose();
    });

    it("cancels the pending retry when the API drops again within the settle", async () => {
        const p = failedOffline();
        p.setApi("online");
        await vi.advanceTimersByTimeAsync(500);
        p.setApi("offline");
        await vi.advanceTimersByTimeAsync(5_000);
        expect(p.sent).toEqual([]);
        p.setApi("online");
        await vi.advanceTimersByTimeAsync(1_000);
        expect(p.sent).toEqual(["build it"]);
        p.dispose();
    });

    it("only offers it with the setting off", async () => {
        const p = failedOffline();
        p.setAutoRetryOn(false);
        expect(p.ui.reconnectHint()).toBeNull();
        p.setApi("online");
        await vi.advanceTimersByTimeAsync(5_000);
        expect(p.sent).toEqual([]);
        p.dispose();
    });

    it("only offers it when a tool ran in it", async () => {
        const p = makePane({ api: "offline" });
        p.setDoc([user("build it"), tool()]);
        p.setFailure(networkFailure(1_000));
        expect(p.ui.reconnectHint()).toBeNull();
        p.setApi("online");
        await vi.advanceTimersByTimeAsync(5_000);
        expect(p.sent).toEqual([]);
        p.dispose();
    });

    it("sends what the operator typed first, then offers the turn instead of retrying it", async () => {
        const p = failedOffline();
        await p.ui.submit("actually, wait");
        expect(p.sent).toEqual([]);
        p.setApi("online");
        await vi.advanceTimersByTimeAsync(5_000);
        // Their message went, and its turn start cleared the failure row.
        expect(p.sent).toEqual(["actually, wait"]);
        const offer = p.ui.notices().find((n) => n.retry);
        expect(offer?.text).toBe(
            "Back online. Your message from 14:32 failed while the API was unreachable; it wasn't sent again, since you wrote after it.",
        );
        offer!.retry!();
        expect(p.sent).toEqual(["actually, wait", "build it"]);
        expect(p.ui.notices().some((n) => n.retry)).toBe(false);
        p.dispose();
    });

    it("waits for a running turn to end before deciding", async () => {
        const p = failedOffline();
        await p.ui.submit("next");
        p.setApi("online");
        p.setWorking(true);
        await vi.advanceTimersByTimeAsync(5_000);
        expect(p.sent).toEqual(["next"]);
        expect(p.ui.notices().some((n) => n.retry)).toBe(false);
        p.setWorking(false);
        await vi.advanceTimersByTimeAsync(1_000);
        expect(p.ui.notices().some((n) => n.retry)).toBe(true);
        p.dispose();
    });

    it("forgets the turn when the operator dismisses the failure", async () => {
        const p = failedOffline();
        p.setFailure(null);
        p.setApi("online");
        await vi.advanceTimersByTimeAsync(5_000);
        expect(p.sent).toEqual([]);
        expect(p.ui.notices()).toEqual([]);
        p.dispose();
    });

    it("leaves a provider srv doesn't track to the failure row", async () => {
        const p = makePane({ api: null });
        p.setDoc([user("build it")]);
        p.setFailure(networkFailure(1_000));
        expect(p.ui.waitsForReconnect("network")).toBe(false);
        await vi.advanceTimersByTimeAsync(5_000);
        expect(p.sent).toEqual([]);
        p.dispose();
    });

    it("a new message clears the marker, so its own failure can be retried", async () => {
        const p = makePane({ marker: { text: "build it", at: 1 } });
        await p.ui.submit("build it");
        expect(p.store.marker).toBeNull();
        p.dispose();
    });
});
