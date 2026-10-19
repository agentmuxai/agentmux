// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Help pane's zoom (helpview.tsx): the shared pane zoom (`term:zoom`), which it
 * joins with its `paneZoom` capability, and the one-time move of the level it used
 * to keep in its own `help:zoom`.
 * SPEC_SYSINFO_TEXT_ZOOM_DENSITY_AND_ONE_ZOOM_PATH_2026_10_18.md §3 (A.5).
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

// Help is the first NATIVE pane tab (Pane Tab contract Phase 2b): it reads
// and writes its block only through the host context.
const setMetaMock = vi.fn((..._args: unknown[]) => Promise.resolve(undefined));

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { helpPaneTab, HelpView } from "./helpview";

describe("HelpView zoom", () => {
    afterEach(() => {
        cleanup();
        setMetaMock.mockClear();
    });

    function renderHelp(meta: Record<string, unknown> = {}) {
        const ctx: PaneTabHostContext = {
            blockId: "test-block",
            meta: () => meta as MetaType,
            setMeta: (patch) => setMetaMock(patch),
            isFocused: () => false,
            visibility: () => "active",
        };
        return render(() => <HelpView ctx={ctx} />);
    }

    const tips = (container: Element) => container.querySelector("div[style*='zoom']") as HTMLElement;

    it("takes part in the shared pane zoom", () => {
        expect(helpPaneTab.capabilities?.paneZoom).toEqual({});
    });

    it("applies term:zoom to the tips, not to the filter bar", () => {
        const { container } = renderHelp({ "term:zoom": 1.3 });
        expect(tips(container).style.zoom).toBe("1.3");
        expect((container.querySelector(".help-filter-bar") as HTMLElement).style.zoom).toBe("");
    });

    const settle = () => new Promise((r) => setTimeout(r, 0));

    it("moves a saved help:zoom to term:zoom, then clears help:zoom", async () => {
        renderHelp({ "help:zoom": 1.5 });
        await settle();
        expect(setMetaMock.mock.calls.map((c) => c[0])).toEqual([{ "term:zoom": 1.5 }, { "help:zoom": null }]);
    });

    it("keeps a term:zoom already set, and still clears help:zoom", async () => {
        renderHelp({ "help:zoom": 1.5, "term:zoom": 0.8 });
        await settle();
        expect(setMetaMock.mock.calls.map((c) => c[0])).toEqual([{ "help:zoom": null }]);
    });

    // Codex review, #5128: the old key goes only after the new one is written.
    it("leaves help:zoom in place when writing term:zoom fails", async () => {
        setMetaMock.mockImplementationOnce(() => Promise.reject(new Error("offline")));
        renderHelp({ "help:zoom": 1.5 });
        await settle();
        expect(setMetaMock).toHaveBeenCalledTimes(1);
        expect(setMetaMock).not.toHaveBeenCalledWith({ "help:zoom": null });
    });

    it("writes nothing when there is no help:zoom", () => {
        renderHelp({ "term:zoom": 1.2 });
        expect(setMetaMock).not.toHaveBeenCalled();
    });

    it("registers as a native pane tab with Help's label and icon", () => {
        expect(helpPaneTab.view).toBe("help");
        expect(helpPaneTab.label).toBe("Help");
        expect(helpPaneTab.icon).toBe("circle-question");
        expect(helpPaneTab.create).toBeTypeOf("function");
    });

    // No zoom handler of its own: Ctrl+wheel reaches the window's shared handler.
    it("leaves Ctrl+Wheel to the window's shared zoom handler", () => {
        const { container } = renderHelp();
        const seen = vi.fn();
        window.addEventListener("wheel", seen);
        const ev = new WheelEvent("wheel", { ctrlKey: true, deltaY: -100, bubbles: true, cancelable: true });
        (container.querySelector("[tabindex]") as HTMLElement).dispatchEvent(ev);
        window.removeEventListener("wheel", seen);
        expect(seen).toHaveBeenCalledTimes(1);
        expect(ev.defaultPrevented).toBe(false);
        expect(setMetaMock).not.toHaveBeenCalled();
    });
});

describe("HelpView filter", () => {
    it("has a filter box that is the pane's typing target, and typing filters the list", async () => {
        const ctx = {
            blockId: "help-1",
            meta: () => ({}),
            setMeta: vi.fn(),
            isFocused: () => true,
            visibility: () => "active",
        } as unknown as PaneTabHostContext;
        const { getByLabelText, queryByText } = render(() => <HelpView ctx={ctx} />);
        const input = getByLabelText("Filter help") as HTMLInputElement;
        expect(input.hasAttribute("data-pane-focus")).toBe(true);
        expect(queryByText("Need More Help?")).not.toBeNull();
        input.value = "discord";
        input.dispatchEvent(new Event("input", { bubbles: true }));
        expect(queryByText("Join Our Discord")).not.toBeNull();
        expect(queryByText("Keyboard Shortcuts")).toBeNull();
    });
});
