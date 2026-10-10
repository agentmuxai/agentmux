// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The App API's focus path (docs/specs/PLAN_SHORTCUT_KINKS_2026_10_10.md, A1):
// a target that names a pane tab becomes that pane's visible tab, and
// PressKeys waits a few frames for the caret before refusing.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    nodes: new Map<string, { id: string; data: { blockId: string; blockStack?: string[]; activeBlockId?: string } }>(),
    setActive: vi.fn(),
    refocus: vi.fn<(blockId: string) => void>(),
}));
vi.mock("@/app/store/global", () => ({
    atoms: { modalOpen: () => false },
    getApi: () => ({ listen: () => Promise.resolve(() => {}) }),
    getBlockComponentModel: () => null,
    refocusNode: h.refocus,
    setControlShiftDelayAtom: vi.fn(),
}));
vi.mock("@/layout/index", () => ({
    getLayoutModelForStaticTab: () => ({ focusedNode: () => null, getNodeByBlockId: (id: string) => h.nodes.get(id) }),
}));
vi.mock("@/layout/lib/layoutStack", () => ({
    effectiveStack: (d: { blockId: string; blockStack?: string[] }) => (d.blockStack?.length ? d.blockStack : [d.blockId]),
    setActiveBlockInStack: h.setActive,
}));
vi.mock("@/app/store/command-registry", () => ({ commandRegistry: { run: vi.fn(() => false) } }));

import { caretInBlock, focusPaneForApi, installShortcutApi } from "./keymodel-dispatch";

/** A pane element for `blockId` with a focusable input inside. */
function pane(blockId: string): HTMLInputElement {
    const el = document.createElement("div");
    el.setAttribute("data-blockid", blockId);
    const input = document.createElement("input");
    el.appendChild(input);
    document.body.appendChild(el);
    return input;
}

beforeEach(() => {
    h.nodes.clear();
    h.setActive.mockClear();
    h.refocus.mockReset();
});
afterEach(() => {
    document.body.innerHTML = "";
});

describe("focusPaneForApi", () => {
    it("makes a hidden pane tab the visible one before focusing it", () => {
        const node = { id: "n1", data: { blockId: "host", blockStack: ["tab-a", "tab-b"], activeBlockId: "tab-b" } };
        h.nodes.set("tab-a", node);
        expect(focusPaneForApi("tab-a")).toBe(true);
        expect(h.setActive).toHaveBeenCalledWith(expect.anything(), "n1", "tab-a");
        expect(h.refocus).toHaveBeenCalledWith("tab-a");
    });

    it("focuses a plain pane without touching tabs", () => {
        h.nodes.set("p1", { id: "n2", data: { blockId: "p1" } });
        expect(focusPaneForApi("p1")).toBe(true);
        expect(h.setActive).toHaveBeenCalledWith(expect.anything(), "n2", "p1");
        expect(h.refocus).toHaveBeenCalledWith("p1");
    });

    it("is false for a block in no pane of the active tab", () => {
        expect(focusPaneForApi("elsewhere")).toBe(false);
        expect(h.refocus).not.toHaveBeenCalled();
    });
});

describe("caretInBlock", () => {
    it("finds the caret in any element of the block", () => {
        const input = pane("b1");
        pane("b2");
        input.focus();
        expect(caretInBlock("b1")).toBe(true);
        expect(caretInBlock("b2")).toBe(false);
    });
});

describe("plan()", () => {
    type Api = { plan: (keys: string, target?: string) => Promise<{ reason?: string; events?: unknown[] }> };
    const api = (): Api => (window as unknown as { __agentmux_shortcuts: Api }).__agentmux_shortcuts;

    it("waits for a caret that lands a few frames late", async () => {
        h.nodes.set("late", { id: "n3", data: { blockId: "late" } });
        const input = pane("late");
        h.refocus.mockImplementation(() => void setTimeout(() => input.focus(), 50));
        installShortcutApi();
        const plan = await api().plan("ctrl+shift+code:Digit1", "late");
        expect(plan.reason).toBeUndefined();
        expect(plan.events).toHaveLength(1);
    });

    it("refuses when the caret never arrives", async () => {
        h.nodes.set("stuck", { id: "n4", data: { blockId: "stuck" } });
        pane("stuck");
        installShortcutApi();
        expect((await api().plan("ctrl+shift+code:Digit1", "stuck")).reason).toMatch(/didn't take keyboard focus/);
    });
});
