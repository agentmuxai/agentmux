// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// MuxAPI's focus path (docs/specs/PLAN_SHORTCUT_KINKS_2026_10_10.md, A1
// and A2): a target anywhere in this window is revealed (its tab switched to,
// its pane showing it) by revealBlock (this window only), and PressKeys waits a few
// frames for the caret before refusing.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    inWindow: new Set<string>(),
    reveal: vi.fn<(blockId: string) => Promise<boolean>>(),
    /** Why a reveal that returned false failed. */
    reason: "not-found",
}));
vi.mock("@/app/store/global", () => ({
    atoms: { modalOpen: () => false },
    getApi: () => ({ listen: () => Promise.resolve(() => {}) }),
    getBlockComponentModel: () => null,
    setControlShiftDelayAtom: vi.fn(),
}));
vi.mock("@/layout/index", () => ({
    getLayoutModelForStaticTab: () => ({ focusedNode: () => null }),
}));
vi.mock("@/app/util/reveal-block", () => ({
    revealBlock: async (id: string) => ((await h.reveal(id)) ? { ok: true } : { ok: false, reason: h.reason }),
}));
vi.mock("@/app/store/command-registry", () => ({ commandRegistry: { run: vi.fn(() => false) } }));

import { caretInBlock, focusPaneForApi, installShortcutApi, OPERATOR_PROMPT_OPEN, operatorPromptOpen } from "./keymodel-dispatch";

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
    h.inWindow.clear();
    h.reveal.mockReset();
    h.reason = "not-found";
    h.reveal.mockImplementation(async (id) => h.inWindow.has(id));
});
afterEach(() => {
    document.body.innerHTML = "";
});

describe("focusPaneForApi", () => {
    it("reveals a pane anywhere in this window: another tab, or a pane's hidden tab", async () => {
        h.inWindow.add("in-another-tab");
        expect(await focusPaneForApi("in-another-tab")).toBe(true);
        expect(h.reveal).toHaveBeenCalledWith("in-another-tab");
    });

    it("is false for a block that isn't in this window", async () => {
        expect(await focusPaneForApi("other-window")).toBe(false);
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
        h.inWindow.add("late");
        const input = pane("late");
        h.reveal.mockImplementation(async () => {
            setTimeout(() => input.focus(), 50);
            return true;
        });
        installShortcutApi();
        const plan = await api().plan("ctrl+shift+code:Digit1", "late");
        expect(plan.reason).toBeUndefined();
        expect(plan.events).toHaveLength(1);
    });

    it("says why a pane could not be focused: not in this window, or not brought to the front", async () => {
        installShortcutApi();
        expect((await api().plan("ctrl+shift+code:Digit1", "elsewhere")).reason).toBe("pane elsewhere is not in this window");
        h.reason = "superseded";
        expect((await api().plan("ctrl+shift+code:Digit1", "elsewhere")).reason).toBe(
            "pane elsewhere was not focused: another pane was chosen meanwhile"
        );
        h.reason = "layout-timeout";
        expect((await api().plan("ctrl+shift+code:Digit1", "elsewhere")).reason).toBe(
            "pane elsewhere did not come to the front (layout-timeout)"
        );
    });

    it("refuses when the caret never arrives", async () => {
        h.inWindow.add("stuck");
        pane("stuck");
        installShortcutApi();
        expect((await api().plan("ctrl+shift+code:Digit1", "stuck")).reason).toMatch(/didn't take keyboard focus/);
    });
});

describe("a prompt for the person", () => {
    it("holds back a command and a key press while one is on screen", async () => {
        installShortcutApi();
        const api = (window as unknown as { __agentmux_shortcuts: { run: (c: string) => unknown; plan: (k: string) => Promise<unknown> } }).__agentmux_shortcuts;
        const prompt = document.createElement("div");
        prompt.setAttribute("data-operator-only", "");
        document.body.appendChild(prompt);
        expect(operatorPromptOpen()).toBe(true);
        expect(await api.run("split:right")).toEqual({ ran: false, reason: OPERATOR_PROMPT_OPEN });
        expect(await api.plan("Enter")).toEqual({ reason: OPERATOR_PROMPT_OPEN });
        prompt.remove();
        expect(operatorPromptOpen()).toBe(false);
        expect(await api.plan("Enter")).not.toEqual({ reason: OPERATOR_PROMPT_OPEN });
    });
});
