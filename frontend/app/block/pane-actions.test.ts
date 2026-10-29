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
const devMode = vi.hoisted(() => ({ on: true }));
vi.mock("@/app/store/dev-mode", () => ({ devModeOn: () => devMode.on }));

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
            "Split Down",
            "Split Right",
            "-",
            "Magnify Pane",
            "Close Pane",
            "-",
            "Inspect Element",
        ]);
    });

    it("dev mode off: no Inspect Element, and no separator left over for it", () => {
        devMode.on = false;
        try {
            const menu = buildPaneContextMenu(termBlock, opts({ inspectAt: { x: 1, y: 2 } }));
            expect(shape(menu)).toEqual(["Copy", "Paste", "-", "Split Down", "Split Right", "-", "Magnify Pane", "Close Pane"]);
            const only = buildPaneContextMenu(termBlock, opts({ inspectAt: { x: 1, y: 2 }, omit: new Set<PaneMenuSection>(["clipboard", "split", "magnify", "close"]) }));
            expect(shape(only)).toEqual([]);
        } finally {
            devMode.on = true;
        }
    });

    it("agent pane has no Paste; magnified pane offers Un-Magnify; no inspectAt means no Inspect", () => {
        const menu = buildPaneContextMenu(agentBlock, opts({ magnified: true }));
        expect(shape(menu)).toEqual([
            "Copy",
            "-",
            "Split Down",
            "Split Right",
            "-",
            "Un-Magnify Pane",
            "Close Pane",
        ]);
    });
});

describe("buildPaneContextMenu — icons (SPEC_CONTEXT_MENU_ICONS_2026_10_25.md §4)", () => {
    const icons = (items: ContextMenuItem[]) => Object.fromEntries(items.filter((i) => i.type !== "separator").map((i) => [i.label, i.icon]));

    it("gives every row its icon", () => {
        const menu = buildPaneContextMenu(termBlock, opts({ inspectAt: { x: 1, y: 2 } }));
        expect(icons(menu)).toEqual({
            Copy: "copy",
            Paste: "paste",
            "Split Down": "table-rows",
            "Split Right": "table-columns",
            "Magnify Pane": "regular@window-maximize",
            "Close Pane": "xmark",
            "Inspect Element": "bug",
        });
    });

    it("swaps the Magnify icon when the pane is magnified", () => {
        const menu = buildPaneContextMenu(agentBlock, opts({ magnified: true }));
        expect(icons(menu)["Un-Magnify Pane"]).toBe("regular@window-restore");
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

// SPEC_SHORTCUTS_PANE_FOR_SPLITS_2026_10_28.md: every split opens a Shortcuts pane, whatever the
// source is. It names the pane it came from and carries nothing else of it (no copied meta, no
// "Always on top" tack, no hue); the tiles take the directory and connection from that pane.
describe("split — a Shortcuts pane, whatever the pane is", () => {
    const AGENT_PICKER_META = { view: "agentsplit", controller: "cmd", "cmd:args": [] as string[] };
    registerPaneTab(
        stubPaneTab("agentsplit", {
            capabilities: { splitBlockDef: () => ({ meta: { ...AGENT_PICKER_META, "cmd:args": [] } }) },
        })
    );
    const sources = {
        tacked: {
            oid: "b9",
            meta: { view: "term", "pane:floating_ontop": true, "frame:hue": 210, "cmd:cwd": "C:/work" },
        },
        agent: {
            oid: "b7",
            meta: { view: "agentsplit", agentId: "lark", "agent:runtime": { model: "opus" }, "agent:sessionid": "sess-1" },
        },
    } as unknown as Record<string, Block>;

    it("both directions open a Shortcuts pane that names the source and copies none of its meta", async () => {
        const g = (await import("@/app/store/global")) as any;
        const expected: Record<string, any> = { "Split Down": g.createBlockSplitVertically, "Split Right": g.createBlockSplitHorizontally };
        for (const source of Object.values(sources)) {
            for (const [label, fn] of Object.entries(expected)) {
                buildPaneContextMenu(source, opts())
                    .find((i) => i.label === label)!
                    .click!();
                await Promise.resolve();
                const [blockDef, targetId, position] = fn.mock.calls.at(-1);
                expect([label, blockDef, targetId, position]).toEqual([
                    label,
                    { meta: { view: "shortcuts", "shortcuts:from": source.oid } },
                    source.oid,
                    "after",
                ]);
            }
        }
    });
});

describe("buildPaneContextMenu — Paste", () => {
    it("pastes into the pane, then gives it focus for the next keystroke", async () => {
        const { readText } = await import("@/util/clipboard");
        vi.mocked(readText).mockResolvedValue("echo hi");
        const calls: string[] = [];
        const viewModel = {
            paste: vi.fn(() => calls.push("paste")),
            giveFocus: vi.fn(() => {
                calls.push("focus");
                return true;
            }),
        } as unknown as ViewModel;
        const paste = buildPaneContextMenu(termBlock, opts(), viewModel).find((i) => i.label === "Paste")!;
        paste.click!();
        await new Promise((r) => setTimeout(r, 0));
        expect(viewModel.paste).toHaveBeenCalledWith("echo hi");
        expect(calls).toEqual(["paste", "focus"]);
    });
});
