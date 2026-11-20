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
    type Api = { run: (c: string, target?: string) => Promise<unknown>; plan: (k: string, target?: string) => Promise<unknown>; set: (r: object, a: string) => Promise<unknown> };
    const api = (): Api => (window as unknown as { __agentmux_shortcuts: Api }).__agentmux_shortcuts;
    /** An element marked as a prompt for the person, inside pane `blockId` (or over the window, with none). */
    function prompt(blockId?: string): HTMLElement {
        const el = document.createElement("div");
        el.setAttribute("data-operator-only", "");
        if (blockId) {
            const host = document.querySelector(`[data-blockid="${blockId}"]`) ?? (pane(blockId), document.querySelector(`[data-blockid="${blockId}"]`)!);
            host.appendChild(el);
        } else document.body.appendChild(el);
        return el;
    }

    it("holds back a command and a key press while one is on screen over the window", async () => {
        installShortcutApi();
        const p = prompt();
        expect(operatorPromptOpen()).toBe(true);
        expect(await api().run("split:right")).toEqual({ ran: false, reason: OPERATOR_PROMPT_OPEN });
        expect(await api().plan("Enter")).toEqual({ reason: OPERATOR_PROMPT_OPEN });
        p.remove();
        expect(operatorPromptOpen()).toBe(false);
        expect(await api().plan("Enter")).not.toEqual({ reason: OPERATOR_PROMPT_OPEN });
    });

    it("holds back only the pane it is in: a prompt open in another pane refuses nothing here", async () => {
        installShortcutApi();
        pane("here");
        prompt("elsewhere");
        expect(operatorPromptOpen("here")).toBe(false);
        expect(operatorPromptOpen("elsewhere")).toBe(true);
        expect(await api().run("split:right", "here")).not.toEqual({ ran: false, reason: OPERATOR_PROMPT_OPEN });
        expect(await api().run("split:right", "elsewhere")).toEqual({ ran: false, reason: OPERATOR_PROMPT_OPEN });
        h.inWindow.add("here");
        expect(await api().plan("Enter", "here")).not.toEqual({ reason: OPERATOR_PROMPT_OPEN });
        expect(await api().plan("Enter", "elsewhere")).toEqual({ reason: OPERATOR_PROMPT_OPEN });
        expect(await api().set({}, "agent")).toEqual({ changed: false, reason: OPERATOR_PROMPT_OPEN });
    });

    it("always lets a cancel through: the Escape command and the Escape key, in the pane that holds the prompt", async () => {
        installShortcutApi();
        h.inWindow.add("asking");
        pane("asking");
        prompt("asking");
        expect(await api().run("app:escape", "asking")).not.toEqual({ ran: false, reason: OPERATOR_PROMPT_OPEN });
        expect(await api().plan("Escape", "asking")).not.toEqual({ reason: OPERATOR_PROMPT_OPEN });
        expect(await api().plan(" escape ", "asking")).not.toEqual({ reason: OPERATOR_PROMPT_OPEN });
        // Nothing else is: a key that could answer it, or a command.
        expect(await api().plan("Enter", "asking")).toEqual({ reason: OPERATOR_PROMPT_OPEN });
        expect(await api().plan("Escape+Enter", "asking")).toEqual({ reason: OPERATOR_PROMPT_OPEN });
        expect(await api().run("pane:close", "asking")).toEqual({ ran: false, reason: OPERATOR_PROMPT_OPEN });
    });

    it("sees no prompt in a confirm that is the person's own action (nothing is marked)", async () => {
        installShortcutApi();
        pane("replace");
        const confirm = document.createElement("div");
        confirm.className = "modal-root";
        document.querySelector('[data-blockid="replace"]')!.appendChild(confirm);
        expect(operatorPromptOpen("replace")).toBe(false);
        expect(await api().run("app:escape", "replace")).not.toEqual({ ran: false, reason: OPERATOR_PROMPT_OPEN });
        expect(await api().run("split:right", "replace")).not.toEqual({ ran: false, reason: OPERATOR_PROMPT_OPEN });
    });
});
