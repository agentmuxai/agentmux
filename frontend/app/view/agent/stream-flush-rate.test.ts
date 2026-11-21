// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRoot, createSignal } from "solid-js";

const state = vi.hoisted(() => ({
    focused: "" as string | undefined,
    throwOnFocus: false,
    /** A reactive focus, as the layout's is; when set it is read instead of `focused`. */
    focusRead: undefined as (() => string) | undefined,
    focusReads: 0,
}));
const dormant = vi.hoisted(() => new Map<string, () => boolean>());

vi.mock("@/app/store/block-component-registry", () => ({
    isBlockDormant: (id: string) => dormant.get(id) ?? (() => false),
    getFocusedBlockId: () => {
        state.focusReads++;
        if (state.throwOnFocus) throw new Error("no layout");
        return state.focusRead ? state.focusRead() : state.focused;
    },
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { createPaneFlushQueue, paneCursorHold, paneFlushRate } from "./stream-flush-rate";
import { createStreamFlushQueue } from "./stream-flush-queue";
import { HIDDEN_DELAY_MS, __resetStreamSchedulerForTests } from "./stream-scheduler";
import type { DocumentNode } from "../../agent-model/types";

describe("paneFlushRate", () => {
    beforeEach(() => {
        dormant.clear();
        state.focused = "";
        state.throwOnFocus = false;
    });

    it("is full for the focused pane", () => {
        state.focused = "a";
        expect(paneFlushRate("a")).toBe("full");
    });

    it("is background for a shown pane that is not focused (a split's other pane)", () => {
        state.focused = "a";
        expect(paneFlushRate("b")).toBe("background");
    });

    it("is hidden for a pane in a tab that is not shown, even if it was the last focused", () => {
        state.focused = "a";
        dormant.set("a", () => true);
        expect(paneFlushRate("a")).toBe("hidden");
    });

    it("follows the pane as its tab is shown and hidden again", () => {
        const [isDormant, setDormant] = createSignal(true);
        dormant.set("a", isDormant);
        state.focused = "a";
        expect(paneFlushRate("a")).toBe("hidden");
        setDormant(false);
        expect(paneFlushRate("a")).toBe("full");
        setDormant(true);
        expect(paneFlushRate("a")).toBe("hidden");
    });

    it("is full, never slowed on a guess, when the focus cannot be looked up", () => {
        state.throwOnFocus = true;
        expect(paneFlushRate("a")).toBe("full");
    });
});

describe("a pane's flush queue asks the scheduler for the pane's rate", () => {
    let frames: FrameRequestCallback[];
    let clock: number;
    const runFrame = () => {
        const due = frames;
        frames = [];
        for (const cb of due) cb(clock);
    };
    const model = () => {
        const docs: unknown[] = [];
        return {
            docs,
            model: { blockId: "b", dispatchDoc: (c: unknown) => docs.push(c), dispatchPane: () => {} } as any,
        };
    };
    const node = (id: string) => ({ type: "markdown", id, content: "x" }) as DocumentNode;

    beforeEach(() => {
        frames = [];
        clock = 1000;
        dormant.clear();
        state.focused = "";
        state.throwOnFocus = false;
        state.focusRead = undefined;
        state.focusReads = 0;
        vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
            frames.push(cb);
            return frames.length;
        });
        vi.stubGlobal("cancelAnimationFrame", () => {
            frames = [];
        });
        vi.spyOn(performance, "now").mockImplementation(() => clock);
        __resetStreamSchedulerForTests();
    });
    afterEach(() => {
        __resetStreamSchedulerForTests();
        vi.restoreAllMocks();
        vi.unstubAllGlobals();
    });

    it("a hidden pane waits out HIDDEN_DELAY_MS, then flushes: its data is late, not lost", () => {
        const [isDormant] = createSignal(true);
        dormant.set("b", isDormant);
        const { model: m, docs } = model();
        const q = createStreamFlushQueue(m, () => paneFlushRate("b"));
        q.pushNewNode(node("n1"));
        q.scheduleFlush();
        runFrame();
        expect(docs).toHaveLength(0);
        clock += HIDDEN_DELAY_MS;
        // The scheduler's own timer arms the wake-up; here the frame it would request is run by hand.
        q.scheduleFlush();
        runFrame();
        expect(docs.length).toBeGreaterThan(0);
    });

    it("a pane whose tab is shown again flushes in the next frame, not after the hidden delay", () => {
        const [isDormant, setDormant] = createSignal(true);
        dormant.set("b", isDormant);
        state.focused = "b";
        const { model: m, docs } = model();
        const dispose = createRoot((d) => {
            const q = createPaneFlushQueue(m, "b");
            q.pushNewNode(node("n1"));
            q.scheduleFlush();
            return d;
        });
        runFrame();
        expect(docs).toHaveLength(0);
        setDormant(false); // the helper's effect asks again, which re-arms the scheduler
        runFrame();
        expect(docs.length).toBeGreaterThan(0);
        dispose();
    });

    it("a focus change does not re-run the helper's effect: it reads only its own pane's tab", () => {
        let dormantReads = 0;
        const [focus, setFocus] = createSignal("b");
        dormant.set("b", () => (dormantReads++, false));
        state.focusRead = focus;
        const { model: m } = model();
        // Nothing is pending, so the effect's own first request is the one that arms the scheduler and reads the rates.
        const dispose = createRoot((d) => {
            createPaneFlushQueue(m, "b");
            return d;
        });
        const readsBefore = dormantReads;
        expect(state.focusReads).toBeGreaterThan(0);
        setFocus("other");
        expect(dormantReads).toBe(readsBefore);
        dispose();
    });

    it("a pane whose window tab is not shown is hidden too, though its pane stack shows it", () => {
        const [off, setOff] = createSignal(true);
        state.focused = "b"; // not dormant in its stack: only the wider signal says it is off screen
        const { model: m, docs } = model();
        const dispose = createRoot((d) => {
            const q = createPaneFlushQueue(m, "b", off);
            q.pushNewNode(node("n1"));
            q.scheduleFlush();
            return d;
        });
        runFrame();
        expect(docs).toHaveLength(0);
        setOff(false);
        runFrame();
        expect(docs.length).toBeGreaterThan(0);
        dispose();
    });

    it("a held cursor asks for the hidden slot once, and giving it back withdraws the request", () => {
        let off = true;
        const hold = paneCursorHold("b", () => off);
        const ran: string[] = [];
        expect(hold.hold()).toBe(true);
        hold.wake(() => ran.push("drain"));
        runFrame();
        expect(ran).toEqual([]); // not due before HIDDEN_DELAY_MS
        clock += HIDDEN_DELAY_MS;
        hold.wake(() => ran.push("drain"));
        runFrame();
        expect(ran).toEqual(["drain"]);
        hold.wake(() => ran.push("later"));
        off = false;
        hold.cancel();
        clock += HIDDEN_DELAY_MS;
        runFrame();
        expect(ran).toEqual(["drain"]);
    });

    it("the focused pane flushes in the next frame", () => {
        state.focused = "b";
        const { model: m, docs } = model();
        const q = createStreamFlushQueue(m, () => paneFlushRate("b"));
        q.pushNewNode(node("n1"));
        q.scheduleFlush();
        runFrame();
        expect(docs.length).toBeGreaterThan(0);
    });
});
