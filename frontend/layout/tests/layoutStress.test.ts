// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Random sequences of the operations a person can do to a tab (add a pane, split one,
// close one, minimize or restore, maximize, minimize a maximized pane, swap two) run
// through the real layout functions, with the layout doctor and a few rules the UI
// promises checked after every step. A failing sequence prints its seed and the steps,
// so it replays: LAYOUT_STRESS_SEED=<seed> npx vitest run frontend/layout/tests/layoutStress.test.ts

import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import { applyLayoutAction } from "../lib/layoutIntentActions";
import { validateLayoutInvariants } from "../lib/layoutInvariants";
import { collapseMaximizedNode, countExpandedLeaves, minimizeNodeToggle, restoreMaximizedNode } from "../lib/layoutMinimize";
import { balanceNode, newLayoutNode } from "../lib/layoutNode";
import { FlexDirection, LayoutTreeActionType, type LayoutNode, type LayoutTreeAction, type LayoutTreeState } from "../lib/types";

function mulberry32(seed: number) {
    return () => {
        seed = (seed + 0x6d2b79f5) | 0;
        let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
        t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
        return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
}

function leaves(n: LayoutNode | undefined, out: LayoutNode[] = []): LayoutNode[] {
    if (!n) return out;
    if (n.children?.length) n.children.forEach((c) => leaves(c, out));
    else if (n.data !== undefined) out.push(n);
    return out;
}

/** The tree state and just enough of a LayoutModel for the minimize functions. */
function makeModel() {
    const model: any = {
        treeState: { rootNode: undefined, pendingBackendActions: [] } as LayoutTreeState,
        minimizedNodeIds: { _set: () => {} },
        localTreeStateAtom: { _set: () => {} },
        // The real treeReducer applies the action, then updateTree balances the tree (layoutGeometry.ts).
        // updateTree also ends the maximize once one pane is left (validateMagnifiedNode, layoutMagnify.ts).
        updateTree: () => {
            if (model.treeState.rootNode) model.treeState.rootNode = balanceNode(model.treeState.rootNode);
            if (leaves(model.treeState.rootNode).length === 1) model.treeState.magnifiedNodeId = undefined;
        },
        treeReducer: (action: LayoutTreeAction) => {
            applyLayoutAction(model.treeState, action);
            model.updateTree();
        },
        magnifyNodeToggle: (nodeId: string) => model.treeReducer({ type: LayoutTreeActionType.MagnifyNodeToggle, nodeId } as LayoutTreeAction),
    };
    return model;
}

let serial = 0;
const freshLeaf = () => newLayoutNode(FlexDirection.Row, 100, undefined, { blockId: `b${serial++}` });

type Step = { op: string; target?: string };

function run(seed: number, steps: number) {
    const rand = mulberry32(seed);
    const pick = <T,>(xs: T[]): T => xs[Math.floor(rand() * xs.length)];
    const model = makeModel();
    const log: Step[] = [];
    const problems: string[] = [];
    const check = (when: string) => {
        const root = model.treeState.rootNode as LayoutNode | undefined;
        const ls = leaves(root);
        const ids = ls.map((l) => l.id);
        const here = `after ${when} (seed ${seed}, step ${log.length})`;
        if (new Set(ids).size !== ids.length) problems.push(`${here}: duplicate leaf ids`);
        const v = validateLayoutInvariants(root);
        if (v.length) problems.push(`${here}: doctor: ${v.map((x) => `${x.code} ${x.detail}`).join("; ")}`);
        const mag = model.treeState.magnifiedNodeId;
        if (mag !== undefined && !ids.includes(mag)) problems.push(`${here}: maximized pane ${mag} is not in the tree`);
        if (ls.length > 0 && countExpandedLeaves(root) < 1) problems.push(`${here}: ${ls.length} pane(s) and every one is minimized`);
    };

    // What each operation promises, from the state before it. Merging a single child into its parent (balanceNode) can
    // rename a pane, so a pane that vanished by name is looked up by leaf count rather than by id where that matters.
    const post = (op: string, target: string | undefined, before: { ids: string[]; minimized: Set<string>; expanded: number; mag?: string }, when: string) => {
        const root = model.treeState.rootNode as LayoutNode | undefined;
        const now = leaves(root);
        const byId = new Map(now.map((l) => [l.id, l]));
        const here = `after ${when} (seed ${seed}, step ${log.length})`;
        const fail = (m: string) => problems.push(`${here}: ${m}`);
        switch (op) {
            case "insert":
                if (now.length !== before.ids.length + 1) fail(`insert changed the pane count ${before.ids.length} -> ${now.length}`);
                break;
            case "splitH":
            case "splitV": {
                // A minimized pane cannot be split; anything else adds exactly one.
                const expected = before.minimized.has(target!) ? before.ids.length : before.ids.length + 1;
                if (now.length !== expected) fail(`split changed the pane count ${before.ids.length} -> ${now.length}, expected ${expected}`);
                break;
            }
            case "delete":
                if (now.length !== before.ids.length - 1) fail(`delete changed the pane count ${before.ids.length} -> ${now.length}`);
                if (before.mag === target && model.treeState.magnifiedNodeId !== undefined) fail("closing the maximized pane left a maximized pane");
                break;
            case "swap":
                if (now.length !== before.ids.length) fail("swap changed the pane count");
                break;
            case "minimize": {
                const t = byId.get(target!);
                if (!t) break;
                const wasMin = before.minimized.has(target!);
                if (wasMin && t.minimized) fail("the toggle on a minimized pane did not restore it");
                if (!wasMin && before.expanded > 1 && !t.minimized) fail("a pane with another expanded pane beside it did not minimize");
                if (!wasMin && before.expanded <= 1 && t.minimized) fail("the last expanded pane was minimized");
                break;
            }
            case "restore": {
                const t = byId.get(target!);
                if (t?.minimized) fail("restore left the pane minimized");
                break;
            }
            case "collapseMax": {
                const t = byId.get(target!);
                const others = before.expanded - (before.minimized.has(target!) ? 0 : 1);
                if (others >= 1) {
                    if (model.treeState.magnifiedNodeId !== undefined) fail("minimizing the maximized pane left it maximized");
                    if (t && !t.minimized) fail("minimizing the maximized pane did not minimize it");
                } else if (model.treeState.magnifiedNodeId !== target) fail("with no other expanded pane the maximized pane lost its maximize");
                break;
            }
            case "restoreMax": {
                if (model.treeState.magnifiedNodeId !== undefined) fail("Restore left a pane maximized");
                const t = byId.get(target!);
                if (t?.minimized) fail("Restore on a maximized pane left it minimized");
                break;
            }
            case "maximize": {
                const want = before.mag === target ? undefined : target;
                // The root pane of a one-pane tab cannot be maximized.
                if (now.length > 1 && model.treeState.magnifiedNodeId !== want) fail(`maximize toggle: maximized is ${model.treeState.magnifiedNodeId}, expected ${want}`);
                break;
            }
        }
    };

    // Start with one pane.
    const first = freshLeaf();
    model.treeState.rootNode = first;
    applyLayoutAction(model.treeState, { type: LayoutTreeActionType.ClearTree } as LayoutTreeAction);
    model.treeReducer({ type: LayoutTreeActionType.InsertNode, node: first, magnified: false, focused: true } as LayoutTreeAction);
    check("the first pane");

    for (let i = 0; i < steps && problems.length === 0; i++) {
        const ls = leaves(model.treeState.rootNode);
        const some = () => (ls.length ? pick(ls) : undefined);
        const logged = log.length;
        const before = {
            ids: ls.map((l) => l.id),
            minimized: new Set(ls.filter((l) => l.minimized).map((l) => l.id)),
            expanded: countExpandedLeaves(model.treeState.rootNode),
            mag: model.treeState.magnifiedNodeId as string | undefined,
        };
        const op = pick(["insert", "insert", "splitH", "splitV", "delete", "minimize", "minimize", "restoreAll", "maximize", "collapseMax", "restoreMax", "swap", "focus"]);
        const t = some();
        switch (op) {
            case "insert":
                log.push({ op });
                model.treeReducer({ type: LayoutTreeActionType.InsertNode, node: freshLeaf(), magnified: false, focused: rand() < 0.5 } as LayoutTreeAction);
                break;
            case "splitH":
            case "splitV":
                if (!t) break;
                log.push({ op, target: t.id });
                model.treeReducer({
                    type: op === "splitH" ? LayoutTreeActionType.SplitHorizontal : LayoutTreeActionType.SplitVertical,
                    targetNodeId: t.id,
                    newNode: freshLeaf(),
                    position: rand() < 0.5 ? "before" : "after",
                    focused: rand() < 0.5,
                    sizeFraction: 0.3 + rand() * 0.4,
                } as LayoutTreeAction);
                break;
            case "delete":
                // closeNode (layoutMagnify.ts): a maximized pane is un-maximized first; closing the last pane closes the tab.
                if (!t || ls.length < 2) break;
                log.push({ op, target: t.id });
                if (model.treeState.magnifiedNodeId === t.id) {
                    model.treeReducer({ type: LayoutTreeActionType.MagnifyNodeToggle, nodeId: t.id, focused: false } as LayoutTreeAction);
                }
                model.treeReducer({ type: LayoutTreeActionType.DeleteNode, nodeId: t.id } as LayoutTreeAction);
                break;
            case "minimize":
                if (!t) break;
                log.push({ op, target: t.id });
                minimizeNodeToggle(model, t.id);
                break;
            case "restoreAll":
                for (const l of ls.filter((x) => x.minimized)) {
                    log.push({ op: "restore", target: l.id });
                    minimizeNodeToggle(model, l.id);
                }
                break;
            case "maximize":
                if (!t) break;
                log.push({ op, target: t.id });
                model.treeReducer({ type: LayoutTreeActionType.MagnifyNodeToggle, nodeId: t.id } as LayoutTreeAction);
                break;
            case "collapseMax": {
                const mag = model.treeState.magnifiedNodeId;
                if (!mag) break;
                log.push({ op, target: mag });
                collapseMaximizedNode(model, mag);
                break;
            }
            case "restoreMax": {
                const mag = model.treeState.magnifiedNodeId;
                if (!mag) break;
                log.push({ op, target: mag });
                restoreMaximizedNode(model, mag);
                break;
            }
            case "swap": {
                if (ls.length < 2) break;
                const a = pick(ls);
                const b = pick(ls.filter((x) => x.id !== a.id));
                log.push({ op, target: `${a.id}<->${b.id}` });
                model.treeReducer({ type: LayoutTreeActionType.Swap, node1Id: a.id, node2Id: b.id } as LayoutTreeAction);
                break;
            }
            case "focus":
                if (!t) break;
                log.push({ op, target: t.id });
                model.treeReducer({ type: LayoutTreeActionType.FocusNode, nodeId: t.id } as LayoutTreeAction);
                break;
        }
        const what = `${op}${log.at(-1)?.target ? " " + log.at(-1)!.target : ""}`;
        check(what);
        // Only an operation that really ran (it logs a step) has postconditions.
        if (problems.length === 0 && log.length === logged + 1 && op !== "restoreAll") post(op, log.at(-1)!.target, before, what);
        if (problems.length === 0 && op === "restoreAll" && leaves(model.treeState.rootNode).some((l) => l.minimized)) {
            problems.push(`after restoreAll (seed ${seed}, step ${log.length}): a pane is still minimized`);
        }
    }
    return { problems, log, tree: model.treeState.rootNode };
}

describe("layout stress: random sequences keep the tree sound", () => {
    // Refused operations (split a chip, swap with one, maximize the only pane) warn by design; thousands of them would bury the result.
    beforeAll(() => {
        vi.spyOn(console, "warn").mockImplementation(() => {});
    });
    afterAll(() => vi.restoreAllMocks());
    const only = process.env.LAYOUT_STRESS_SEED ? Number(process.env.LAYOUT_STRESS_SEED) : undefined;
    const seeds = only !== undefined ? [only] : Array.from({ length: Number(process.env.LAYOUT_STRESS_COUNT ?? 300) }, (_, i) => i + 1);
    it(`${seeds.length} sequence(s) of up to ${process.env.LAYOUT_STRESS_STEPS ?? 60} operations`, () => {
        const failing: { steps: number; text: string }[] = [];
        for (const seed of seeds) {
            const r = run(seed, Number(process.env.LAYOUT_STRESS_STEPS ?? 60));
            if (r.problems.length) failing.push({ steps: r.log.length, text: `${r.problems[0]}\n  steps: ${r.log.map((s) => s.op + (s.target ? `(${s.target.slice(0, 4)})` : "")).join(", ")}` });
        }
        // The shortest sequence first: it is the easiest to read.
        const failures = failing.sort((a, b) => a.steps - b.steps).map((f) => f.text);
        const kinds = new Map<string, number>();
        for (const f of failures) {
            const k = f.split(": ").slice(1, 3).join(": ").replace(/[\w-]{36}|b\d+|\d+/g, "#").slice(0, 80);
            kinds.set(k, (kinds.get(k) ?? 0) + 1);
        }
        expect(failures.length, `${failures.length} of ${seeds.length} failed; kinds:\n${[...kinds].map(([k, n]) => `  ${n}x ${k}`).join("\n")}\nfirst:\n${failures[0]}`).toBe(0);
    }, 600_000);
});
