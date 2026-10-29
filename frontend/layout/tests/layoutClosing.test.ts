// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The closing state of a pane and of a tab, entered before the close's first await, and the
 * close trace. SPEC_PANE_CLOSING_INDICATOR_AND_FASTER_CLOSE_2026_10_28.md §2, §3.4, §4.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createEffect, createRoot, createSignal } from "solid-js";
import { LayoutModel } from "@/layout/lib/layoutModel";
import { newLayoutNode } from "@/layout/lib/layoutNode";
import { closeNode } from "@/layout/lib/layoutMagnify";
import { closeBlockInStack, pushBlockOntoStack } from "@/layout/lib/layoutStack";
import { closingTabs, isBlockClosing } from "@/layout/lib/layoutClosing";
import { CLOSE_TRACE_MAX, createCloseTraceRing } from "@/layout/lib/closeTrace";
import { LayoutTreeActionType, LayoutTreeInsertNodeAction } from "@/layout/lib/types";
import type { SignalAtom } from "@/util/util";

const rpcCall = vi.fn();
// Layout intents (every tree edit) go to their own mock, so `rpcCall` counts
// the pane-tab commands these tests are about. Its version is one no mocked
// row reaches, so the model never swaps its tree for the (static) stored one.
const intentCall = vi.fn(async (..._args: unknown[]) => ({ applied: true, version: Number.MAX_SAFE_INTEGER }));
const deleteBlock = vi.fn();
vi.mock("@/app/store/rpc-util", () => ({
    TabRpcClient: {
        rpcCall: (...args: unknown[]) => (args[0] === "layout.applyIntent" ? intentCall(...args) : rpcCall(...args)),
    },
}));
vi.mock("@/app/store/services", () => ({
    ObjectService: {
        DeleteBlock: (...args: unknown[]) => deleteBlock(...args),
        GetObject: async () => ({ version: 1 }),
    },
}));

// Same mock harness as layoutModel.test.ts.
const layoutStateSignals = new Map<string, SignalAtom<LayoutState>>();

function makeLayoutStateSignal(oid: string): SignalAtom<LayoutState> {
    const [get, set] = createSignal<LayoutState>({
        otype: "layout",
        oid,
        version: 1,
        meta: {},
        rootnode: undefined,
        magnifiednodeid: undefined,
        focusednodeid: undefined,
        leaforder: undefined,
        pendingbackendactions: undefined,
    });
    const atom = () => get();
    (atom as any)._set = set;
    return atom as unknown as SignalAtom<LayoutState>;
}

vi.mock("@/app/store/global", () => {
    return {
        MOS: {
            makeORef: (_otype: string, oid: string) => oid,
            getMuxObjectAtom: (oid: string) => {
                if (!layoutStateSignals.has(oid)) {
                    layoutStateSignals.set(oid, makeLayoutStateSignal(oid));
                }
                return layoutStateSignals.get(oid);
            },
            getObjectValue: (oref: string) => {
                const sig = layoutStateSignals.get(oref);
                return sig ? sig() : undefined;
            },
            setObjectValue: (value: any) => {
                const oref = `${value.otype}:${value.oid}`;
                const sig = layoutStateSignals.get(value.oid) ?? layoutStateSignals.get(oref);
                if (sig) sig._set(value);
            },
        },
        getSettingsKeyAtom: () => {
            const [get] = createSignal(0.75);
            return get;
        },
        globalStore: {
            get: (accessor: any) => (typeof accessor === "function" ? accessor() : undefined),
            set: (setter: any, value: any) => {
                if (setter && typeof setter._set === "function") setter._set(value);
                else if (typeof setter === "function") setter(value);
            },
        },
    };
});

// SPEC_PANE_SELECT_AUTOFOCUS_2026_09_22.md: `requestNodeFocus()` used to be
// a no-op, so InsertNode/FocusNode/MagnifyNodeToggle calling it here was
// inert. Now real, it reaches into `@/app/store/global`'s `atoms` (which
// this file's own minimal mock above deliberately doesn't provide — these
// tests exercise pure tree-mutation logic, not app-wide focus plumbing) via
// `getLayoutModelForStaticTab()`. Mocked out entirely rather than backfilled
// into the mock above, same reasoning as that mock's own narrow surface.
const { ensureSelectionFocused } = vi.hoisted(() => ({ ensureSelectionFocused: vi.fn() }));
vi.mock("@/app/store/focusManager", () => ({
    focusManager: {
        requestNodeFocus: () => {},
        refocusNode: () => {},
        claimFocusOnMount: () => {},
        ensureSelectionFocused,
    },
}));

/** Let the layout outbox send what is queued (pane-tab commands wait behind
 *  the intents ahead of them). */
const settle = async () => {
    for (let i = 0; i < 20; i++) await Promise.resolve();
};

function createLayoutModel(): LayoutModel {
    const [getTab] = createSignal<Tab>({
        otype: "tab",
        oid: "tab-1",
        version: 1,
        meta: {},
        name: "Test Tab",
        layoutstate: "layout-1",
        blockids: [],
    });
    const model = new LayoutModel(getTab);
    model.getBoundingRect = () => ({ top: 0, left: 0, width: 800, height: 600 });
    model.displayContainerRef.current = {
        getBoundingClientRect: () => ({ top: 0, left: 0, width: 800, height: 600 }),
    } as any;
    return model;
}

/** Insert a single leaf as the tab's root and return its node id. */
function insertRootBlock(model: LayoutModel, blockId: string): string {
    const node = newLayoutNode(undefined, undefined, undefined, { blockId });
    model.treeReducer({
        type: LayoutTreeActionType.InsertNode,
        node,
        magnified: false,
        focused: true,
    } as LayoutTreeInsertNodeAction);
    return model.treeState.rootNode!.id;
}

describe("closing state", () => {
    beforeEach(() => {
        layoutStateSignals.clear();
        vi.useFakeTimers();
    });
    afterEach(() => {
        vi.useRealTimers();
    });

    it("a pane is closing before the close's first await, and not any more once it is gone", async () => {
        const model = createLayoutModel();
        const nodeId = insertRootBlock(model, "b1");
        model.onNodeDelete = vi.fn().mockResolvedValue(undefined);
        model.beforeNodeDelete = vi.fn(async () => true);

        const closing = closeNode(model, nodeId);
        expect(closingTabs(model, nodeId)).toBe(1); // synchronously, with the first await still pending
        await closing;
        expect(closingTabs(model, nodeId)).toBe(0);
        expect(model.treeState.rootNode).toBeUndefined();
    });

    it("counts the tabs of a stacked pane", async () => {
        const model = createLayoutModel();
        const nodeId = insertRootBlock(model, "b1");
        pushBlockOntoStack(model, nodeId, "b2");
        pushBlockOntoStack(model, nodeId, "b3");
        model.onNodeDelete = vi.fn().mockResolvedValue(undefined);
        let answer!: (go: boolean) => void;
        model.beforeNodeDelete = vi.fn(() => new Promise<boolean>((resolve) => (answer = resolve)));

        const closing = closeNode(model, nodeId);
        expect(closingTabs(model, nodeId)).toBe(3);
        answer(true);
        await closing;
    });

    it("a second close of a closing pane does nothing", async () => {
        const model = createLayoutModel();
        const nodeId = insertRootBlock(model, "b1");
        model.onNodeDelete = vi.fn().mockResolvedValue(undefined);
        let answer!: (go: boolean) => void;
        const before = vi.fn(() => new Promise<boolean>((resolve) => (answer = resolve)));
        model.beforeNodeDelete = before;

        const first = closeNode(model, nodeId);
        await closeNode(model, nodeId);
        expect(before).toHaveBeenCalledTimes(1);
        answer(true);
        await first;
    });

    it("a kept close puts the pane back to normal", async () => {
        const model = createLayoutModel();
        const nodeId = insertRootBlock(model, "b1");
        model.onNodeDelete = vi.fn().mockResolvedValue(undefined);
        model.beforeNodeDelete = vi.fn(async () => false);

        await closeNode(model, nodeId);
        expect(closingTabs(model, nodeId)).toBe(0);
        expect(model.treeState.rootNode).toBeDefined();
    });

    it("the trace records the asks done before the wait on a person", async () => {
        const model = createLayoutModel();
        const nodeId = insertRootBlock(model, "b1");
        model.onNodeDelete = vi.fn().mockResolvedValue(undefined);
        let answer!: (go: boolean) => void;
        model.beforeNodeDelete = vi.fn(async (_data, hooks) => hooks!.asking(new Promise<boolean>((resolve) => (answer = resolve))));
        const closing = closeNode(model, nodeId);
        await Promise.resolve();
        await Promise.resolve();
        answer(true);
        await closing;
        await vi.advanceTimersByTimeAsync(50);
        const line = (globalThis as { __closeTrace?: () => string[] }).__closeTrace!().at(-1)!;
        expect(line.indexOf("probes+")).toBeGreaterThan(-1);
        expect(line.indexOf("probes+")).toBeLessThan(line.indexOf("prompt+"));
    });

    it("a close whose asks throw leaves a failed entry in the trace", async () => {
        const model = createLayoutModel();
        const nodeId = insertRootBlock(model, "b1");
        model.beforeNodeDelete = vi.fn(async () => {
            throw new Error("probe failed");
        });
        await expect(closeNode(model, nodeId)).rejects.toThrow();
        expect((globalThis as { __closeTrace?: () => string[] }).__closeTrace!().some((l) => l.includes("failed+"))).toBe(true);
    });

    it("a close that throws puts the pane back to normal", async () => {
        const model = createLayoutModel();
        const nodeId = insertRootBlock(model, "b1");
        model.onNodeDelete = vi.fn().mockResolvedValue(undefined);
        model.beforeNodeDelete = vi.fn(async () => {
            throw new Error("probe failed");
        });

        await expect(closeNode(model, nodeId)).rejects.toThrow("probe failed");
        expect(closingTabs(model, nodeId)).toBe(0);
    });

    it("while a prompt is open the pane shows its normal look, and goes back to closing on yes", async () => {
        const model = createLayoutModel();
        const nodeId = insertRootBlock(model, "b1");
        model.onNodeDelete = vi.fn().mockResolvedValue(undefined);
        let answer!: (go: boolean) => void;
        const seen: number[] = [];
        model.beforeNodeDelete = vi.fn(async (_data, hooks) => {
            await Promise.resolve(); // the probes come first, the pane is closing meanwhile
            const asked = hooks!.asking(new Promise<boolean>((resolve) => (answer = resolve)));
            seen.push(closingTabs(model, nodeId)); // the prompt is open
            const yes = await asked;
            seen.push(closingTabs(model, nodeId)); // answered
            return yes;
        });

        const closing = closeNode(model, nodeId);
        expect(closingTabs(model, nodeId)).toBe(1);
        await Promise.resolve();
        await Promise.resolve();
        answer(true);
        await closing;
        expect(seen).toEqual([0, 1]);
    });

    it("a no to the prompt leaves the pane, not closing", async () => {
        const model = createLayoutModel();
        const nodeId = insertRootBlock(model, "b1");
        model.onNodeDelete = vi.fn().mockResolvedValue(undefined);
        let answer!: (go: boolean) => void;
        model.beforeNodeDelete = vi.fn(async (_data, hooks) => hooks!.asking(new Promise<boolean>((resolve) => (answer = resolve))));

        const closing = closeNode(model, nodeId);
        await Promise.resolve();
        answer(false);
        await closing;
        expect(closingTabs(model, nodeId)).toBe(0);
        expect(model.treeState.rootNode).toBeDefined();
    });

    it("one tab of several closes on its own: its pill is closing, the pane is not", async () => {
        const model = createLayoutModel();
        const nodeId = insertRootBlock(model, "b1");
        pushBlockOntoStack(model, nodeId, "b2");
        model.onNodeDelete = vi.fn().mockResolvedValue(undefined);
        let answer!: (go: boolean) => void;
        model.beforeNodeDelete = vi.fn(() => new Promise<boolean>((resolve) => (answer = resolve)));

        const closing = closeBlockInStack(model, nodeId, "b2");
        expect(isBlockClosing(model, "b2")).toBe(true);
        expect(isBlockClosing(model, "b1")).toBe(false);
        expect(closingTabs(model, nodeId)).toBe(0);
        answer(false);
        await closing;
        expect(isBlockClosing(model, "b2")).toBe(false);
    });

    it("a reader of the state follows it", async () => {
        const model = createLayoutModel();
        const nodeId = insertRootBlock(model, "b1");
        model.onNodeDelete = vi.fn().mockResolvedValue(undefined);
        let answer!: (go: boolean) => void;
        model.beforeNodeDelete = vi.fn(() => new Promise<boolean>((resolve) => (answer = resolve)));
        const reads: number[] = [];
        const dispose = createRoot((d) => {
            const nodeModel = model.getNodeModel(model.treeState.rootNode!);
            // The node model's own memos, as the header reads them.
            createEffect(() => reads.push(nodeModel.closingTabs!()));
            return d;
        });
        await Promise.resolve();
        const closing = closeNode(model, nodeId);
        await Promise.resolve();
        answer(false);
        await closing;
        dispose();
        expect(reads).toEqual([0, 1, 0]);
    });
});

describe("close trace", () => {
    it("keeps the last 50 closes", () => {
        const ring = createCloseTraceRing(() => 0, () => {});
        for (let i = 0; i < CLOSE_TRACE_MAX + 5; i++) {
            const t = ring.start(`pane ${i}`);
            t.mark("reducer");
            t.mark("gone");
            t.mark("closePane");
        }
        const lines = ring.lines();
        expect(lines).toHaveLength(CLOSE_TRACE_MAX);
        expect(lines[0]).toContain("pane 5 ");
    });

    it("a close slower than 500 ms to leave logs one line naming its slowest phase", () => {
        let now = 0;
        const log = vi.fn();
        const ring = createCloseTraceRing(() => now, log);
        const t = ring.start("pane ab12 2 tabs");
        now = 40;
        t.mark("probes");
        now = 1540; // the prompt took a second and a half
        t.mark("prompt");
        now = 1545;
        t.mark("reducer");
        now = 1560;
        t.mark("gone");
        now = 1700;
        t.mark("closePane");
        expect(log).toHaveBeenCalledTimes(1);
        expect(log.mock.calls[0][0]).toContain("slowest phase: prompt (1500ms)");
        expect(ring.lines()[0]).toContain("gone+1560ms");
    });

    it("a fast close logs nothing", () => {
        let now = 0;
        const log = vi.fn();
        const ring = createCloseTraceRing(() => now, log);
        const t = ring.start("pane ab12 1 tab");
        now = 30;
        t.mark("reducer");
        now = 40;
        t.mark("gone");
        t.mark("closePane");
        expect(log).not.toHaveBeenCalled();
    });

    it("a kept close is recorded and ends the trace", () => {
        const ring = createCloseTraceRing(() => 0, () => {});
        const t = ring.start("pane ab12 1 tab");
        t.mark("kept");
        expect(ring.lines()).toHaveLength(1);
        expect(ring.lines()[0]).toContain("kept+0ms");
    });

    it("end() records kept or failed for a close that never removed the pane, and nothing once it did", () => {
        const ring = createCloseTraceRing(() => 0, () => {});
        const a = ring.start("pane a");
        a.mark("probes");
        a.end("failed");
        a.end("kept"); // already ended: no second line
        const b = ring.start("pane b");
        b.mark("reducer");
        b.end("kept"); // removed the pane: closePane and gone still to come
        expect(ring.lines()).toHaveLength(1);
        expect(ring.lines()[0]).toContain("pane a probes+0ms failed+0ms");
    });
});
