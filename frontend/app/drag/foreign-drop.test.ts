// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const claim = vi.fn<(label: string) => Promise<void>>(() => Promise.resolve());
let linux = true;

vi.mock("@/store/global", () => ({ getApi: () => ({ claimCrossDragTarget: claim }) }));
vi.mock("@/util/platformutil", () => ({ isLinux: () => linux }));
vi.mock("@/util/logger", () => ({ Logger: { info: vi.fn(), warn: vi.fn() } }));

import { WINDOW_TAB_MIME, acceptWindowTabsFromOtherWindows, maybeDroppedInAnotherWindow } from "./foreign-drop";

/** jsdom has no DragEvent: a plain event with the parts these listeners read. */
function dragEvent(type: string, types: string[] = [WINDOW_TAB_MIME]): Event & { dataTransfer: { types: string[]; dropEffect: string } } {
    const e = new Event(type, { bubbles: true, cancelable: true }) as any;
    e.dataTransfer = { types, dropEffect: "none" };
    return e;
}

let dispose: () => void = () => {};

beforeEach(() => {
    linux = true;
    claim.mockClear();
    window.dispatchEvent(dragEvent("dragstart", [])); // a fresh drag here...
    window.dispatchEvent(dragEvent("dragend", [])); // ...that has ended
});

afterEach(() => dispose());

describe("maybeDroppedInAnotherWindow", () => {
    it("is a drop elsewhere when the drag ended with move and no drop here", () => {
        window.dispatchEvent(dragEvent("dragstart", []));
        expect(maybeDroppedInAnotherWindow("move")).toBe(true);
    });

    it("isn't once this window got the drop", () => {
        window.dispatchEvent(dragEvent("drop", []));
        expect(maybeDroppedInAnotherWindow("move")).toBe(false);
    });

    it("isn't for a cancelled drag, or off Linux", () => {
        expect(maybeDroppedInAnotherWindow("none")).toBe(false);
        linux = false;
        expect(maybeDroppedInAnotherWindow("move")).toBe(false);
    });
});

describe("acceptWindowTabsFromOtherWindows", () => {
    it("accepts a window tab dragged from another window, and claims its drop", () => {
        dispose = acceptWindowTabsFromOtherWindows("window-pool-2");
        const over = dragEvent("dragover");
        window.dispatchEvent(over);
        expect(over.defaultPrevented).toBe(true);
        expect(over.dataTransfer.dropEffect).toBe("move");

        const drop = dragEvent("drop");
        window.dispatchEvent(drop);
        expect(drop.defaultPrevented).toBe(true);
        expect(claim).toHaveBeenCalledWith("window-pool-2");
    });

    // The drag library ends the drag session in its own drop handler, before
    // this window-level listener sees the drop: an in-window reorder must
    // still not read as a drop from elsewhere (#4706).
    it("leaves this window's own drags alone, through to their drop", () => {
        dispose = acceptWindowTabsFromOtherWindows("main");
        window.dispatchEvent(dragEvent("dragstart"));
        const over = dragEvent("dragover");
        window.dispatchEvent(over);
        window.dispatchEvent(dragEvent("drop"));
        expect(over.defaultPrevented).toBe(false);
        expect(claim).not.toHaveBeenCalled();
    });

    it("accepts drags from elsewhere again once its own drag has ended", () => {
        dispose = acceptWindowTabsFromOtherWindows("main");
        window.dispatchEvent(dragEvent("dragstart"));
        window.dispatchEvent(dragEvent("dragend"));
        window.dispatchEvent(dragEvent("drop"));
        expect(claim).toHaveBeenCalledWith("main");
    });

    it("leaves other drags alone (no window-tab marker)", () => {
        dispose = acceptWindowTabsFromOtherWindows("main");
        const over = dragEvent("dragover", ["Files"]);
        window.dispatchEvent(over);
        window.dispatchEvent(dragEvent("drop", ["Files"]));
        expect(over.defaultPrevented).toBe(false);
        expect(claim).not.toHaveBeenCalled();
    });

    it("does nothing off Linux", () => {
        linux = false;
        dispose = acceptWindowTabsFromOtherWindows("main");
        const over = dragEvent("dragover");
        window.dispatchEvent(over);
        expect(over.defaultPrevented).toBe(false);
    });
});
