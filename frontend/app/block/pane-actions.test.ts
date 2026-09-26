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

const { replaceItems } = vi.hoisted(() => ({ replaceItems: [] as any[] }));

vi.mock("@/app/store/global", () => ({
    atoms: { fullConfigAtom: () => ({ widgets: {}, settings: {} }) },
    createBlockSplitHorizontally: vi.fn(),
    createBlockSplitVertically: vi.fn(),
    getApi: () => ({ inspectElementAt: vi.fn() }),
    replaceBlock: vi.fn(),
}));
vi.mock("@/app/window/action-widgets-config", () => ({
    buildPaneWidgetMenuItems: () => replaceItems,
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

const ALL_SECTIONS: PaneMenuSection[] = ["clipboard", "split", "replace", "magnify", "close", "inspect"];

describe("buildPaneContextMenu — baseline (no omit)", () => {
    it("term pane, with a Replace submenu and Inspect", () => {
        replaceItems.splice(0, replaceItems.length, { label: "Browser" });
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
            "Replace With...",
            "-",
            "Magnify Pane",
            "Close Pane",
            "-",
            "Inspect Element",
        ]);
    });

    it("agent pane has no Paste; magnified pane offers Un-Magnify; no inspectAt means no Inspect", () => {
        replaceItems.splice(0, replaceItems.length, { label: "Browser" });
        const menu = buildPaneContextMenu(agentBlock, opts({ magnified: true }));
        expect(shape(menu)).toEqual([
            "Copy",
            "-",
            "Split Up",
            "Split Down",
            "Split Left",
            "Split Right",
            "-",
            "Replace With...",
            "-",
            "Un-Magnify Pane",
            "Close Pane",
        ]);
    });

    it("with no replacement widgets the Replace entry and its separator both disappear", () => {
        replaceItems.splice(0, replaceItems.length);
        const menu = buildPaneContextMenu(agentBlock, opts());
        expect(shape(menu)).toEqual([
            "Copy",
            "-",
            "Split Up",
            "Split Down",
            "Split Left",
            "Split Right",
            "-",
            "Magnify Pane",
            "Close Pane",
        ]);
    });
});

describe("buildPaneContextMenu — omit", () => {
    it("drops Split and Replace, keeping Magnify/Close/Inspect grouped correctly", () => {
        replaceItems.splice(0, replaceItems.length, { label: "Browser" });
        const menu = buildPaneContextMenu(
            agentBlock,
            opts({ inspectAt: { x: 1, y: 2 }, omit: new Set<PaneMenuSection>(["clipboard", "split", "replace"]) })
        );
        expect(shape(menu)).toEqual(["Magnify Pane", "Close Pane", "-", "Inspect Element"]);
    });

    it("omitting only Replace leaves no dangling separator", () => {
        replaceItems.splice(0, replaceItems.length, { label: "Browser" });
        const menu = buildPaneContextMenu(agentBlock, opts({ omit: new Set<PaneMenuSection>(["replace"]) }));
        expect(shape(menu)).toEqual([
            "Copy",
            "-",
            "Split Up",
            "Split Down",
            "Split Left",
            "Split Right",
            "-",
            "Magnify Pane",
            "Close Pane",
        ]);
    });

    it("omitting everything yields an empty menu", () => {
        replaceItems.splice(0, replaceItems.length, { label: "Browser" });
        const menu = buildPaneContextMenu(termBlock, opts({ inspectAt: { x: 1, y: 2 }, omit: new Set(ALL_SECTIONS) }));
        expect(menu).toEqual([]);
    });

    it("never produces a leading, trailing, or doubled separator for ANY omitted subset", () => {
        replaceItems.splice(0, replaceItems.length, { label: "Browser" });
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
