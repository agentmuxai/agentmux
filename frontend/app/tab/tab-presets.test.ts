// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Regression test for the sibling-ordering bug Codex flagged (P2) on PR
 * #2796: applyNode's split-target for the 2nd+ child of a multi-child
 * preset split always pointed at the FIRST child instead of the PREVIOUS
 * one, so a split with 3+ children didn't land in declared order. This
 * only surfaced once DEFAULT_TAB_PRESET grew a 3-child vertical split
 * (swarm/memory/sysinfo, SPEC_DEFAULT_WIDGETS_REORDER_2026_08_25.md) — a
 * 2-child split can't distinguish "first child" from "previous child".
 *
 * DEFAULT_TAB_PRESET is back down to a 2-child right column (sysinfo above
 * swarm; memory dropped from the starter set), so the shipped default no
 * longer exercises this path. The fixtures below are deliberately local
 * and stay 3-child: the applier still supports N children, and this is the
 * only thing guarding that ordering — don't retire it just because the
 * current default happens not to hit it.
 */

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { applyTabPreset, DEFAULT_TAB_PRESET, type PresetNode } from "./tab-presets";

let nextBlockId = 0;
const createBlock = vi.fn(async (blockDef: any) => {
    const view = blockDef?.meta?.view ?? "unknown";
    nextBlockId += 1;
    return `block-${view}-${nextBlockId}`;
});

vi.mock("@/app/store/services", () => ({
    ObjectService: { CreateBlock: (...args: unknown[]) => createBlock(...(args as [any])) },
}));

const widgets: Record<string, any> = {
    "defwidget@agent": { blockdef: { meta: { view: "agent" } } },
    "defwidget@swarm": { blockdef: { meta: { view: "swarm" } } },
    "defwidget@memory": { blockdef: { meta: { view: "memory" } } },
    "defwidget@sysinfo": { blockdef: { meta: { view: "sysinfo" } } },
};

vi.mock("@/app/store/global", () => ({
    fullConfigAtom: () => ({ widgets }),
    MOS: {
        getObjectValue: () => ({ oid: "tab-1" }),
        makeORef: (kind: string, id: string) => `${kind}:${id}`,
    },
}));

const dispatched: Array<{ type: string; targetNodeId?: string; position?: string }> = [];
const markBlockRecentlyCreated = vi.fn();

vi.mock("@/layout/index", () => ({
    LayoutTreeActionType: { InsertNode: "insert", SplitHorizontal: "splithorizontal", SplitVertical: "splitvertical" },
    markBlockRecentlyCreated: (...args: unknown[]) => markBlockRecentlyCreated(...args),
    getLayoutModelForTabById: () => ({
        treeReducer: (action: any) => dispatched.push(action),
        // Every block id resolves to a node id of the same shape — good
        // enough to distinguish "which block was targeted" without
        // reimplementing the real tree.
        getNodeByBlockId: (blockId: string) => ({ id: `node-${blockId}` }),
    }),
}));

describe("applyTabPreset sibling ordering (Codex P2 on PR #2796)", () => {
    beforeEach(() => {
        vi.clearAllMocks();
        dispatched.length = 0;
        nextBlockId = 0;
    });

    it("a 3-child split inserts each sibling after the PREVIOUS one, preserving declared order", async () => {
        const preset: PresetNode = {
            split: "horizontal",
            children: [
                { widget: "defwidget@agent" },
                {
                    split: "vertical",
                    children: [{ widget: "defwidget@swarm" }, { widget: "defwidget@memory" }, { widget: "defwidget@sysinfo" }],
                },
            ],
        };

        await applyTabPreset("tab-1", preset);

        // agent: root insert, no split target.
        expect(dispatched[0]).toMatchObject({ type: "insert" });
        // swarm: first child of the vertical split — splits off agent.
        expect(dispatched[1]).toMatchObject({ type: "splithorizontal", targetNodeId: "node-block-agent-1" });
        // memory: splits off swarm (the previous sibling).
        expect(dispatched[2]).toMatchObject({ type: "splitvertical", targetNodeId: "node-block-swarm-2" });
        // sysinfo: splits off memory (the previous sibling) — NOT off
        // swarm again, which is the bug this test guards against.
        expect(dispatched[3]).toMatchObject({ type: "splitvertical", targetNodeId: "node-block-memory-3" });
    });

    it("a 2-child split still works (previous === first for exactly 2 children)", async () => {
        const preset: PresetNode = {
            split: "horizontal",
            children: [{ widget: "defwidget@agent" }, { widget: "defwidget@swarm" }],
        };

        await applyTabPreset("tab-1", preset);

        expect(dispatched[0]).toMatchObject({ type: "insert" });
        expect(dispatched[1]).toMatchObject({ type: "splithorizontal", targetNodeId: "node-block-agent-1" });
    });
});

// ANALYSIS_NEW_WINDOW_TAB_LATENCY_2026_09_30.md §3.3: the preset's blocks are
// created concurrently, then placed in declared order whatever order their
// round trips finish in.
describe("applyTabPreset concurrent creation", () => {
    const preset: PresetNode = {
        split: "horizontal",
        children: [
            { widget: "defwidget@agent" },
            {
                split: "vertical",
                children: [{ widget: "defwidget@swarm" }, { widget: "defwidget@memory" }, { widget: "defwidget@sysinfo" }],
            },
        ],
    };

    beforeEach(() => {
        vi.clearAllMocks();
        dispatched.length = 0;
        nextBlockId = 0;
    });

    it("issues every CreateBlock before any returns, and still places them in declared order", async () => {
        const pending = new Map<string, (id: string) => void>();
        createBlock.mockImplementation(
            (blockDef: { meta: { view: string } }) => new Promise<string>((resolve) => pending.set(blockDef.meta.view, resolve))
        );
        const done = applyTabPreset("tab-1", preset);
        await vi.waitFor(() => expect(createBlock).toHaveBeenCalledTimes(4));
        expect(dispatched).toHaveLength(0);
        for (const view of ["sysinfo", "swarm", "agent", "memory"]) pending.get(view)!(`block-${view}`);
        await done;

        expect(dispatched[0]).toMatchObject({ type: "insert" });
        expect(dispatched[1]).toMatchObject({ type: "splithorizontal", targetNodeId: "node-block-agent" });
        expect(dispatched[2]).toMatchObject({ type: "splitvertical", targetNodeId: "node-block-swarm" });
        expect(dispatched[3]).toMatchObject({ type: "splitvertical", targetNodeId: "node-block-memory" });
        expect(markBlockRecentlyCreated).toHaveBeenCalledTimes(4);
    });

    it("skips a block that failed and lets the next sibling take its place", async () => {
        createBlock.mockImplementation(async (blockDef: { meta: { view: string } }) => {
            if (blockDef.meta.view === "swarm") throw new Error("boom");
            return `block-${blockDef.meta.view}`;
        });
        await applyTabPreset("tab-1", preset);

        expect(dispatched).toHaveLength(3);
        expect(dispatched[0]).toMatchObject({ type: "insert" });
        // memory is now the column's first child: it splits off agent, as swarm would have.
        expect(dispatched[1]).toMatchObject({ type: "splithorizontal", targetNodeId: "node-block-agent" });
        expect(dispatched[2]).toMatchObject({ type: "splitvertical", targetNodeId: "node-block-memory" });
    });
});

// DEFAULT_TAB_PRESET mirrors srv's first-window layout, `default_three_pane_tree`
// in crates/srv/src/backend/wcore/mod.rs, by hand (no shared code). This reads
// that builder, so a change to one side without the other fails here. Only the
// shape is compared: the preset can't express the Rust tree's 20/80 sizes.
describe("DEFAULT_TAB_PRESET mirrors default_three_pane_tree", () => {
    const repoRoot = resolve(__dirname, "../../..");
    const wcore = readFileSync(resolve(repoRoot, "crates/srv/src/backend/wcore/mod.rs"), "utf8").replace(/\/\/.*$/gm, "");

    /** `horizontal(agent,vertical(sysinfo,swarm))`: splits and leaf views, in order. */
    function presetShape(node: PresetNode): string {
        if ("widget" in node) return node.widget.replace(/^defwidget@/, "");
        return `${node.split}(${node.children.map(presetShape).join(",")})`;
    }

    type RustNode = { dir?: string; leaf?: string; children: RustNode[] };

    /** The same shape read from the Rust builder. A `LayoutNode` with children
     *  is a split (Row lays them side by side, the preset's "horizontal");
     *  a leaf is named by its `<view>_block_id` parameter. */
    function rustShape(): string {
        const fnStart = wcore.indexOf("pub(crate) fn default_three_pane_tree(");
        expect(fnStart, "default_three_pane_tree in wcore/mod.rs").toBeGreaterThanOrEqual(0);
        const fn = wcore.slice(fnStart);
        const body = fn.slice(fn.indexOf("let rootnode = LayoutNode {"), fn.indexOf("let leaforder"));
        const open: (RustNode | null)[] = [];
        const innermost = (): RustNode | undefined => {
            for (let i = open.length - 1; i >= 0; i--) if (open[i]) return open[i];
            return undefined;
        };
        let root: RustNode | undefined;
        for (const m of body.matchAll(/LayoutNode\s*\{|\{|\}|FlexDirection::(\w+)|block_id: (\w+)_block_id\b/g)) {
            if (m[0].startsWith("LayoutNode")) {
                const node: RustNode = { children: [] };
                const parent = innermost();
                if (parent) parent.children.push(node);
                else root = node;
                open.push(node);
            } else if (m[0] === "{") open.push(null);
            else if (m[0] === "}") open.pop();
            else if (m[1]) innermost().dir ??= m[1];
            else if (m[2]) innermost().leaf = m[2];
        }
        const shape = (n: RustNode): string =>
            n.children.length === 0
                ? (n.leaf ?? "?")
                : `${n.dir === "Row" ? "horizontal" : "vertical"}(${n.children.map(shape).join(",")})`;
        return root ? shape(root) : "";
    }

    it("has the same splits and panes, in the same order", () => {
        expect(presetShape(DEFAULT_TAB_PRESET)).toBe(rustShape());
    });

    it("names panes by the view each one opens on both sides", () => {
        // srv seeds `<name>_block` with view "<name>"; the preset's
        // `defwidget@<name>` opens view "<name>" (crates/srv/src/config/widgets.json).
        const seeded = new Map(
            [...wcore.matchAll(/(\w+)_meta\.insert\("view"\.to_string\(\), serde_json::json!\("(\w+)"\)\)/g)].map((m) => [m[1], m[2]])
        );
        const widgetsJson = JSON.parse(readFileSync(resolve(repoRoot, "crates/srv/src/config/widgets.json"), "utf8"));
        const leaves = presetShape(DEFAULT_TAB_PRESET).match(/\w+(?=[,)]|$)/g) ?? [];
        expect(leaves.length).toBeGreaterThan(0);
        for (const leaf of leaves) {
            expect(seeded.get(leaf), `srv seeds ${leaf}`).toBe(leaf);
            expect(widgetsJson[`defwidget@${leaf}`]?.blockdef?.meta?.view, `defwidget@${leaf}`).toBe(leaf);
        }
    });
});
