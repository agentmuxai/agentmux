// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The window's trail of places: what records a step, what never does, and what Back and Forward put back
 * (SPEC_BACK_FORWARD_FOLLOW_THE_WHOLE_TRAIL_2026_11_26.md). The window is a small stand-in: two tabs of panes,
 * one active tab and one focused pane, both signals the history's effect follows, and a Kanban that registers
 * a place source for its pane.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const w = vi.hoisted(() => ({
    // The active tab and the focused pane, solid signals the history's effect follows (made when the global mock loads).
    s: null as unknown as {
        tab: () => string;
        setTab: (id: string) => void;
        focus: () => string | null;
        setFocus: (id: string | null) => void;
    },
    tabs: { tab1: ["A", "B", "K", "agent"], tab2: ["C"] } as Record<string, string[]>,
    blockMeta: { K: { view: "kanban" } } as Record<string, { view: string }>,
}));

vi.mock("@/app/store/global", async () => {
    const { createSignal } = await import("solid-js");
    const [tab, setTab] = createSignal("tab1");
    const [focus, setFocus] = createSignal<string | null>("A");
    w.s = { tab, setTab: (id) => void setTab(id), focus, setFocus: (id) => void setFocus(id) };
    return { atoms: { activeTabId: () => tab(), workspace: () => ({ tabids: Object.keys(w.tabs), pinnedtabids: [] }) } };
});
vi.mock("@/app/store/tab-actions", () => ({ setActiveTab: async (id: string) => void w.s.setTab(id) }));
vi.mock("@/app/store/mos", () => ({
    makeORef: (otype: string, oid: string) => `${otype}:${oid}`,
    getObjectValue: (oref: string) => {
        const [otype, oid] = oref.split(":");
        if (otype === "tab") return w.tabs[oid] ? { blockids: w.tabs[oid] } : undefined;
        return { meta: w.blockMeta[oid] ?? { view: "term" } };
    },
}));
vi.mock("@/app/store/focusManager", () => ({ focusManager: { blockFocusAtom: () => w.s.focus(), refocusNode: () => {} } }));
vi.mock("@/app/tab/tab-content-settled", () => ({ whenTabContentSettled: async () => {} }));
const locate = vi.hoisted(() => vi.fn());
vi.mock("@/app/util/pane-locate", () => ({ emitPaneLocate: locate }));
vi.mock("@/layout/index", () => ({
    getLayoutModelForTabById: (tabId: string) => ({
        treeState: { rootNode: { tabId } },
        focusNode: (id: string) => w.s.setFocus(id),
    }),
    setActiveBlockInStack: () => {},
}));
vi.mock("@/layout/lib/layoutNode", () => ({
    findNodeByBlockId: (root: { tabId: string }, blockId: string) =>
        w.tabs[root.tabId]?.includes(blockId) ? { id: blockId, data: { blockId } } : null,
}));

import {
    installPaneHistory,
    paneHistoryBack,
    paneHistoryForward,
    paneHistorySnapshot,
    recordLinkedPlace,
    recordPlaceMove,
    resetPaneHistory,
    runGesture,
    withoutHistory,
} from "./pane-history";
import { registerPlaceSource, resetPlaceSourcesForTests } from "./place-sources";
import { asGesture, withoutTrail } from "./trail-hooks";

/** The Kanban's own state: the card open in it, and what its restore does. */
const kanban = { task: undefined as string | undefined, restoreOk: true, gone: new Set<unknown>(), restored: [] as unknown[] };
let dispose: () => void;
let unregister: () => void;

/** Let frames and queued steps run. */
const settle = (ms = 120) => vi.advanceTimersByTimeAsync(ms);
/** The person selects pane `block` of `tab`. */
async function select(tab: string, block: string) {
    w.s.setTab(tab);
    w.s.setFocus(block);
    await settle();
}
const trail = () => paneHistorySnapshot().entries.map((e) => (e.view?.task ? `${e.blockId}:${e.view.task}` : e.blockId));

beforeEach(async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "performance"] });
    vi.stubGlobal("requestAnimationFrame", (cb: (t: number) => void) => setTimeout(() => cb(performance.now()), 16));
    locate.mockClear();
    w.s.setTab("tab1");
    w.s.setFocus("A");
    kanban.task = undefined;
    kanban.restoreOk = true;
    kanban.gone = new Set();
    kanban.restored = [];
    resetPaneHistory();
    resetPlaceSourcesForTests();
    unregister = registerPlaceSource("K", {
        read: () => ({ board: "work", task: kanban.task }),
        restore: (place) => {
            kanban.restored.push(place);
            const ok = kanban.restoreOk && !kanban.gone.has(place.task);
            if (ok) kanban.task = place.task as string | undefined;
            return ok;
        },
    });
    dispose = installPaneHistory();
    await settle(); // the first place stands apart from the next in time
});

afterEach(() => {
    dispose();
    unregister();
    vi.useRealTimers();
    vi.unstubAllGlobals();
});

describe("the trail of places", () => {
    it("records the panes the person selects, and Back and Forward walk them", async () => {
        await select("tab1", "B");
        await select("tab2", "C");
        expect(trail()).toEqual(["A", "B", "C"]);
        paneHistoryBack();
        await settle();
        expect([w.s.tab(), w.s.focus()]).toEqual(["tab1", "B"]);
        paneHistoryForward();
        await settle();
        expect([w.s.tab(), w.s.focus()]).toEqual(["tab2", "C"]);
        // A step is not recorded as a place of its own.
        expect(trail()).toEqual(["A", "B", "C"]);
        expect(paneHistorySnapshot().index).toBe(2);
    });

    it("a link followed in a gesture is one step of the pane and the card it opened, and Back puts the card back", async () => {
        // Pane A, then a link to task x: the Kanban is selected and shows the card, then an agent's pane, then pane B.
        const link = runGesture(async () => {
            w.s.setFocus("K");
            kanban.task = "x";
        });
        await settle();
        await link;
        expect(trail()).toEqual(["A", "K:x"]);
        await select("tab1", "agent");
        await select("tab1", "B");
        expect(trail()).toEqual(["A", "K:x", "agent", "B"]);

        kanban.task = "elsewhere"; // the card moved on by itself since: Back still puts the one the trail holds
        paneHistoryBack();
        await settle();
        expect(w.s.focus()).toBe("agent");
        paneHistoryBack();
        await settle();
        expect(w.s.focus()).toBe("K");
        expect(kanban.restored).toEqual([{ board: "work", task: "x" }]);
        expect(kanban.task).toBe("x");
        paneHistoryBack();
        await settle();
        expect(w.s.focus()).toBe("A");
        // Forward replays: the Kanban again, with its card.
        kanban.task = undefined;
        paneHistoryForward();
        await settle();
        expect([w.s.focus(), kanban.task]).toEqual(["K", "x"]);
        expect(trail()).toEqual(["A", "K:x", "agent", "B"]);
    });

    it("the link funnels and the reveals reach the trail through the window's hooks", async () => {
        const link = asGesture(async () => {
            w.s.setFocus("K");
            kanban.task = "x";
        });
        await settle();
        await link;
        expect(trail()).toEqual(["A", "K:x"]);
        const reveal = withoutTrail(async () => void w.s.setFocus("B"));
        await settle();
        await reveal;
        expect(trail()).toEqual(["A", "K:x"]);
    });

    it("what a gesture changes in two places is one step, not two", async () => {
        const link = runGesture(async () => {
            w.s.setFocus("K");
            await Promise.resolve();
            kanban.task = "x";
            recordPlaceMove("K"); // the view's own click handler, while the gesture runs: the gesture covers it
        });
        await settle();
        await link;
        expect(trail()).toEqual(["A", "K:x"]);
    });

    it("a click inside a view that moves it is a step, the same place again is not, and clicks in one frame are one", async () => {
        await select("tab1", "K");
        kanban.task = "x";
        recordPlaceMove("K");
        await settle();
        expect(trail()).toEqual(["A", "K", "K:x"]);
        recordPlaceMove("K");
        await settle();
        expect(trail()).toEqual(["A", "K", "K:x"]);
        kanban.task = "y";
        recordPlaceMove("K");
        kanban.task = "z";
        recordPlaceMove("K"); // within a frame of the last: it stands for it
        await settle();
        expect(trail()).toEqual(["A", "K", "K:x", "K:z"]);
    });

    it("a card a link opens after the gesture recorded its place is part of that step, not another", async () => {
        const link = runGesture(async () => void w.s.setFocus("K"));
        await settle();
        await link;
        expect(trail()).toEqual(["A", "K"]);
        kanban.task = "x"; // the request reaches the Kanban a moment after the gesture ended
        recordLinkedPlace("K");
        await settle();
        expect(trail()).toEqual(["A", "K:x"]);
        // Back from another pane puts the card back.
        await select("tab1", "B");
        kanban.task = undefined;
        paneHistoryBack();
        await settle();
        expect([w.s.focus(), kanban.task]).toEqual(["K", "x"]);
    });

    it("a link followed where the person already is shows its card as one step of its own, and nothing else is moved by a request", async () => {
        await select("tab1", "K");
        kanban.task = "y";
        recordPlaceMove("K");
        await settle();
        const link = runGesture(async () => {}); // a link inside the Kanban: nothing to select
        await settle();
        await link;
        kanban.task = "z";
        recordLinkedPlace("K");
        await settle();
        expect(trail()).toEqual(["A", "K", "K:y", "K:z"]);
        // A request with no gesture of the person just before it (an agent's) is not a step.
        kanban.task = "q";
        recordLinkedPlace("K");
        await settle();
        expect(trail()).toEqual(["A", "K", "K:y", "K:z"]);
    });

    it("a view that has not joined records nothing of its own: a move inside it is not a step", async () => {
        await select("tab1", "B");
        recordPlaceMove("B");
        await settle();
        expect(trail()).toEqual(["A", "B"]);
    });

    it("what an agent or the host moves is not a step, and Back then returns to where the trail left the person first", async () => {
        await select("tab1", "B");
        // An agent's reveal moves the person to pane C in another tab, without a step.
        const moved = withoutHistory(async () => {
            w.s.setTab("tab2");
            w.s.setFocus("C");
        });
        await settle();
        await moved;
        expect(trail()).toEqual(["A", "B"]);
        paneHistoryBack();
        await settle();
        expect([w.s.tab(), w.s.focus()]).toEqual(["tab1", "B"]);
        paneHistoryBack();
        await settle();
        expect(w.s.focus()).toBe("A");
    });

    it("a place that is gone from its view still lands on the pane, with the place dropped from the entry", async () => {
        const link = runGesture(async () => {
            w.s.setFocus("K");
            kanban.task = "x";
        });
        await settle();
        await link;
        await select("tab1", "B");
        kanban.task = undefined; // the task was deleted: its card is gone from the Kanban
        kanban.restoreOk = false;
        paneHistoryBack();
        await settle();
        expect(w.s.focus()).toBe("K");
        expect(trail()).toEqual(["A", "K", "B"]);
    });

    it("when the pane is the same and its place is gone, the press goes on to the entry before: it never looks like nothing happened", async () => {
        await select("tab1", "K");
        kanban.task = "x";
        recordPlaceMove("K");
        await settle();
        kanban.task = "y";
        recordPlaceMove("K");
        await settle();
        expect(trail()).toEqual(["A", "K", "K:x", "K:y"]);
        kanban.gone = new Set(["x"]);
        paneHistoryBack(); // K:x is gone: the pane is the same and shows y still, so on to the plain Kanban entry
        await settle();
        expect(w.s.focus()).toBe("K");
        expect(kanban.task).toBeUndefined();
        expect(paneHistorySnapshot().index).toBe(1);
        // The dead place was dropped from its entry, which is now a plain Kanban entry too.
        expect(trail()).toEqual(["A", "K", "K", "K:y"]);
    });

    it("a pane that was closed is skipped and removed, never an error", async () => {
        await select("tab1", "B");
        await select("tab2", "C");
        w.tabs.tab1 = w.tabs.tab1.filter((b) => b !== "B");
        paneHistoryBack();
        await settle();
        expect(w.s.focus()).toBe("A");
        expect(trail()).toEqual(["A", "C"]);
        w.tabs.tab1 = ["A", "B", "K", "agent"];
    });

    it("the person's own selection while a press waits drops the press", async () => {
        await select("tab1", "B");
        paneHistoryBack();
        w.s.setFocus("agent"); // before the step ran
        await settle();
        expect(w.s.focus()).toBe("agent");
        expect(trail()).toEqual(["A", "B", "agent"]);
    });
});
