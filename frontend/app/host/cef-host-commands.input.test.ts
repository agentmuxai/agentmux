// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Every request for the keyboard goes out as one numbered `input_select`, so the
// host can drop one that reaches it after a newer one
// (SPEC_BROWSER_PANE_INPUT_OWNER_2026_10_15.md §3.6).

import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/platform/ipc", () => ({
    invokeCommand: vi.fn(() => Promise.resolve(true)),
    invokeBrowserApi: vi.fn(() => Promise.resolve()),
}));

import { invokeCommand } from "@/app/platform/ipc";
import { cefBrowserPanes, cefInputOwnerState, cefReclaimWindowFocus } from "./cef-host-commands";

type Selection = { window_label: string; block_id?: string; epoch: string; seq: number };

function selections(): Selection[] {
    return vi
        .mocked(invokeCommand)
        .mock.calls.filter(([cmd]) => cmd === "input_select")
        .map(([, args]) => args as Selection);
}

describe("keyboard requests", () => {
    beforeEach(() => vi.mocked(invokeCommand).mockClear());

    it("the app and a pane are both numbered selections of one page load, in order", async () => {
        await cefReclaimWindowFocus("main");
        await cefBrowserPanes.focus("block-1");
        await cefReclaimWindowFocus("main");
        const [a, b, c] = selections();
        expect(a.block_id).toBeUndefined();
        expect(b.block_id).toBe("block-1");
        expect(c.block_id).toBeUndefined();
        expect(new Set([a.epoch, b.epoch, c.epoch]).size).toBe(1);
        expect(a.epoch).not.toBe("");
        expect(b.seq).toBe(a.seq + 1);
        expect(c.seq).toBe(b.seq + 1);
    });

    it("the old one-off commands are no longer sent", async () => {
        await cefReclaimWindowFocus("main");
        await cefBrowserPanes.focus("block-1");
        const sent = vi.mocked(invokeCommand).mock.calls.map(([cmd]) => cmd);
        expect(sent).not.toContain("main_window_focus");
        expect(sent).not.toContain("browser_pane_focus");
    });

    it("a pane's selection names the window this page is", async () => {
        await cefBrowserPanes.focus("block-1");
        expect(selections()[0].window_label).toBe("main");
    });

    it("asks the host for a window's input state", async () => {
        await cefInputOwnerState("floating-1");
        expect(invokeCommand).toHaveBeenCalledWith("input_owner_state", { window_label: "floating-1" });
    });
});
