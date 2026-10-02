// Copyright 2026-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * buildPaneContextMenu — section layout and omission.
 * docs/specs/SPEC_AGENT_SHELL_DRAWER_CONTEXT_MENU_PASTE_AND_REGIONS_2026_09_25.md §2.1, §5.
 *
 * The first describe block is the BASELINE LOCK: it was written against the
 * pre-refactor builder and asserts the exact label/separator sequence, so the
 * section refactor is provably a no-op for every caller that passes no `omit`.
 */

import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/global", () => ({
    createBlockSplitHorizontally: vi.fn(),
    createBlockSplitVertically: vi.fn(),
    getApi: () => ({ inspectElementAt: vi.fn() }),
}));
vi.mock("@/util/clipboard", () => ({ readText: vi.fn(), writeText: vi.fn() }));

import { buildPaneContextMenu, type PaneMenuSection } from "./pane-actions";
import { registerPaneTab } from "./pane-tab-registry";
import { stubPaneTab } from "./pane-tab-test-utils";

// Paste is offered for a view that declares `acceptsInput` (Pane Tab contract
// Phase 5) — as block-registry.ts declares it for the terminal.
registerPaneTab(stubPaneTab("term", { capabilities: { acceptsInput: true } }));

const termBlock = { oid: "b1", meta: { view: "term" } } as unknown as Block;
const agentBlock = { oid: "b2", meta: { view: "agent" } } as unknown as Block;

const opts = (over: Record<string, unknown> = {}) => ({
    magnified: false,
    onMagnifyToggle: () => {},
    onClose: () => {},
    ...over,
});

/** "-" for a separator, otherwise the label. */
const shape = (items: ContextMenuItem[]) => items.map((i) => (i.type === "separator" ? "-" : i.label));

const ALL_SECTIONS: PaneMenuSection[] = ["clipboard", "split", "magnify", "close", "inspect"];

describe("buildPaneContextMenu — baseline (no omit)", () => {
    it("term pane, with Inspect", () => {
        const menu = buildPaneContextMenu(termBlock, opts({ inspectAt: { x: 1, y: 2 } }));
        expect(shape(menu)).toEqual([
            "Copy",
            "Paste",
            "-",
            "Split Up",
            "Split Down",
            "Split Left",
            "Split Right",
            "-",
            "Magnify Pane",
            "Close Pane",
            "-",
            "Inspect Element",
        ]);
    });

    it("agent pane has no Paste; magnified pane offers Un-Magnify; no inspectAt means no Inspect", () => {
        const menu = buildPaneContextMenu(agentBlock, opts({ magnified: true }));
        expect(shape(menu)).toEqual([
            "Copy",
            "-",
            "Split Up",
            "Split Down",
            "Split Left",
            "Split Right",
            "-",
            "Un-Magnify Pane",
            "Close Pane",
        ]);
    });
});

describe("buildPaneContextMenu — omit", () => {
    it("drops Clipboard and Split, keeping Magnify/Close/Inspect grouped correctly", () => {
        const menu = buildPaneContextMenu(
            agentBlock,
            opts({ inspectAt: { x: 1, y: 2 }, omit: new Set<PaneMenuSection>(["clipboard", "split"]) })
        );
        expect(shape(menu)).toEqual(["Magnify Pane", "Close Pane", "-", "Inspect Element"]);
    });

    it("omitting only Split leaves no dangling separator", () => {
        const menu = buildPaneContextMenu(agentBlock, opts({ omit: new Set<PaneMenuSection>(["split"]) }));
        expect(shape(menu)).toEqual(["Copy", "-", "Magnify Pane", "Close Pane"]);
    });

    it("omitting everything yields an empty menu", () => {
        const menu = buildPaneContextMenu(termBlock, opts({ inspectAt: { x: 1, y: 2 }, omit: new Set(ALL_SECTIONS) }));
        expect(menu).toEqual([]);
    });

    it("never produces a leading, trailing, or doubled separator for ANY omitted subset", () => {
        for (let mask = 0; mask < 1 << ALL_SECTIONS.length; mask++) {
            const omit = new Set(ALL_SECTIONS.filter((_, i) => mask & (1 << i)));
            const s = shape(buildPaneContextMenu(termBlock, opts({ inspectAt: { x: 1, y: 2 }, omit })));
            const label = `omit=${[...omit].join(",") || "none"}`;
            if (s.length === 0) continue;
            expect(s[0], label).not.toBe("-");
            expect(s[s.length - 1], label).not.toBe("-");
            for (let i = 1; i < s.length; i++) {
                expect(s[i] === "-" && s[i - 1] === "-", label).toBe(false);
            }
        }
    });
});

describe("split — the floating 'Always on top' tack is not inherited", () => {
    it("drops pane:floating_ontop and keeps the rest of the source meta", async () => {
        const { createBlockSplitVertically } = await import("@/app/store/global");
        const tacked = {
            oid: "b9",
            meta: { view: "term", "pane:floating_ontop": true, "frame:hue": 210 },
        } as unknown as Block;
        const splitDown = buildPaneContextMenu(tacked, opts()).find((i) => i.label === "Split Down");
        splitDown!.click!();
        await Promise.resolve();

        const [blockDef, targetId] = (createBlockSplitVertically as any).mock.calls.at(-1);
        expect(targetId).toBe("b9");
        expect(blockDef.meta).not.toHaveProperty("pane:floating_ontop");
        expect(blockDef.meta["frame:hue"]).toBe(210);
        expect(blockDef.meta.view).toBe("term");
    });
});

// SPEC_AGENT_PANE_SPLIT_OPENS_PICKER_2026_09_30.md: a view with `splitBlockDef`
// splits into that block; nothing comes from the source pane.
describe("split — a view's splitBlockDef replaces copying its meta", () => {
    const PICKER_META = { view: "agentsplit", controller: "cmd", "cmd:args": [] as string[] };
    registerPaneTab(
        stubPaneTab("agentsplit", {
            capabilities: { splitBlockDef: () => ({ meta: { ...PICKER_META, "cmd:args": [] } }) },
        })
    );
    const historyTab = {
        oid: "b7",
        meta: {
            view: "agentsplit",
            agentId: "lark",
            "agent:historyTabFor": "lark",
            "agent:runtime": { model: "opus" },
            "agent:sessionid": "sess-1",
            "pane:floating_ontop": true,
        },
    } as unknown as Block;

    it("Split Down creates exactly the declared block below the source", async () => {
        const { createBlockSplitVertically } = await import("@/app/store/global");
        buildPaneContextMenu(historyTab, opts())
            .find((i) => i.label === "Split Down")!
            .click!();
        await Promise.resolve();
        const [blockDef, targetId, position] = (createBlockSplitVertically as any).mock.calls.at(-1);
        expect(targetId).toBe("b7");
        expect(position).toBe("after");
        expect(blockDef).toEqual({ meta: PICKER_META });
    });

    it("every direction splits into the declared block", async () => {
        const g = (await import("@/app/store/global")) as any;
        const expected: Record<string, [any, string]> = {
            "Split Up": [g.createBlockSplitVertically, "before"],
            "Split Down": [g.createBlockSplitVertically, "after"],
            "Split Left": [g.createBlockSplitHorizontally, "before"],
            "Split Right": [g.createBlockSplitHorizontally, "after"],
        };
        for (const [label, [fn, position]] of Object.entries(expected)) {
            buildPaneContextMenu(historyTab, opts())
                .find((i) => i.label === label)!
                .click!();
            await Promise.resolve();
            const [blockDef, targetId, pos] = fn.mock.calls.at(-1);
            expect([label, blockDef, targetId, pos]).toEqual([label, { meta: PICKER_META }, "b7", position]);
        }
    });
});
