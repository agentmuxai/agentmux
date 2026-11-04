// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/platform/pane-overlay", () => ({ usePaneOverlay: vi.fn(), useHiddenTabMarker: () => () => undefined }));
vi.mock("@/app/util/menu-position", () => ({
    computeMenuPosition: vi.fn(async () => ({ style: { position: "fixed", left: "0px", top: "0px" } })),
}));

import { MenuTrigger } from "./MenuTrigger";

let frames: FrameRequestCallback[] = [];

/** Run the menu's first frame and let the position it asked for land. */
const placed = async (): Promise<void> => {
    const due = frames;
    frames = [];
    for (const cb of due) cb(0);
    await Promise.resolve();
    await Promise.resolve();
};

function mount(over: { onOpenChange?: (open: boolean) => void; onPick?: () => void } = {}) {
    render(() => (
        <MenuTrigger
            items={[{ label: "One", testId: "one", onSelect: over.onPick }]}
            onOpenChange={over.onOpenChange}
            aria-label="Things"
        >
            <button type="button" data-testid="trigger">
                Open
            </button>
        </MenuTrigger>
    ));
    return screen.getByTestId("trigger");
}

// A level that has closed leaves a picture of itself fading out (aria-hidden); it is not a menu.
const open = () => document.querySelectorAll('.anchored-popover:not([aria-hidden="true"]) [role="menu"]').length;

describe("MenuTrigger", () => {
    beforeEach(() => {
        frames = [];
        vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => frames.push(cb));
        vi.stubGlobal("cancelAnimationFrame", () => {});
    });
    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
        document.body.innerHTML = "";
    });

    it("opens a menu on a press, and closes it on a second press", async () => {
        const onOpenChange = vi.fn();
        const trigger = mount({ onOpenChange });
        expect(screen.queryByRole("menu")).toBeNull();
        fireEvent.click(trigger);
        await placed();
        expect(screen.getByRole("menu").getAttribute("aria-label")).toBe("Things");
        expect(onOpenChange).toHaveBeenLastCalledWith(true);
        fireEvent.mouseDown(trigger);
        fireEvent.click(trigger);
        expect(document.querySelector('.anchored-popover:not([aria-hidden="true"]) [role="menu"]')).toBeNull();
        expect(onOpenChange).toHaveBeenLastCalledWith(false);
    });

    it("closes on a pick and runs it, and on an outside press", async () => {
        const onPick = vi.fn();
        const trigger = mount({ onPick });
        fireEvent.click(trigger);
        await placed();
        fireEvent.click(screen.getByTestId("one"));
        expect(onPick).toHaveBeenCalledTimes(1);
        expect(open()).toBe(0);
        fireEvent.click(trigger);
        await placed();
        expect(open()).toBe(1);
        fireEvent.mouseDown(document.body);
        expect(open()).toBe(0);
    });

    it("warms the browser panes' freeze-frame as the press starts, once, while closed", () => {
        const warm = vi.fn();
        window.addEventListener("pane-freeze-prewarm", warm);
        const trigger = mount();
        fireEvent.pointerDown(trigger);
        expect(warm).toHaveBeenCalledTimes(1);
        fireEvent.click(trigger);
        fireEvent.pointerDown(trigger);
        expect(warm).toHaveBeenCalledTimes(1);
        window.removeEventListener("pane-freeze-prewarm", warm);
    });

    it("gives focus back to the trigger when it closes without a pick that moved it", async () => {
        const trigger = mount();
        trigger.focus();
        fireEvent.click(trigger);
        await placed();
        (document.activeElement as HTMLElement | null)?.blur();
        fireEvent.keyDown(window, { key: "Escape" });
        await Promise.resolve();
        expect(document.activeElement).toBe(trigger);
    });
});
