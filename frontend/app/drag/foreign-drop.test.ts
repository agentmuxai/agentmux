// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const claim = vi.fn<(label: string) => Promise<void>>(() => Promise.resolve());
let linux = true;

vi.mock("@/store/global", () => ({ getApi: () => ({ claimCrossDragTarget: claim }) }));
vi.mock("@/util/platformutil", () => ({ isLinux: () => linux }));
vi.mock("@/util/logger", () => ({ Logger: { info: vi.fn(), warn: vi.fn() } }));

import {
    WINDOW_TAB_MIME,
    acceptWindowTabsFromOtherWindows,
    maybeDroppedInAnotherWindow,
    windowTabDragData,
    type WindowTabDragData,
} from "./foreign-drop";

const tab: WindowTabDragData = { tabId: "t1", fromWsId: "ws-a", sourceWindowLabel: "window-pool-1", isLastTab: false };

/** jsdom has no DragEvent: a plain event with the parts these listeners read. */
function dragEvent(type: string, data: Record<string, string> = windowTabDragData(tab)) {
    const e = new Event(type, { bubbles: true, cancelable: true }) as any;
    e.dataTransfer = { types: Object.keys(data), dropEffect: "none", getData: (t: string) => data[t] ?? "" };
    return e as Event & { dataTransfer: { types: string[]; dropEffect: string } };
}

const merge = vi.fn();
let dispose: () => void = () => {};

beforeEach(() => {
    linux = true;
    claim.mockClear();
    merge.mockClear();
    window.dispatchEvent(dragEvent("dragstart", {})); // a fresh drag here...
    window.dispatchEvent(dragEvent("dragend", {})); // ...that has ended
});

afterEach(() => dispose());

describe("maybeDroppedInAnotherWindow", () => {
    it("is a drop elsewhere when the drag ended with move and no drop here", () => {
        window.dispatchEvent(dragEvent("dragstart", {}));
        expect(maybeDroppedInAnotherWindow("move")).toBe(true);
    });

    it("isn't once this window got the drop", () => {
        window.dispatchEvent(dragEvent("drop", {}));
        expect(maybeDroppedInAnotherWindow("move")).toBe(false);
    });

    it("isn't for a cancelled drag, or off Linux", () => {
        expect(maybeDroppedInAnotherWindow("none")).toBe(false);
        linux = false;
        expect(maybeDroppedInAnotherWindow("move")).toBe(false);
    });
});

describe("acceptWindowTabsFromOtherWindows", () => {
    it("accepts a window tab from another window, merges it, and claims the drop", () => {
        dispose = acceptWindowTabsFromOtherWindows("main", merge);
        const over = dragEvent("dragover");
        window.dispatchEvent(over);
        expect(over.defaultPrevented).toBe(true);
        expect(over.dataTransfer.dropEffect).toBe("move");

        const drop = dragEvent("drop");
        window.dispatchEvent(drop);
        expect(drop.defaultPrevented).toBe(true);
        expect(merge).toHaveBeenCalledWith(tab, drop);
        expect(claim).toHaveBeenCalledWith("main");
    });

    // The commonest redock: a torn-off window's only tab, back into main. Its
    // source has no cross-window drag-end to consume a claim, so none is made.
    it("merges a window's last tab without claiming the drop", () => {
        dispose = acceptWindowTabsFromOtherWindows("main", merge);
        const lone = { ...tab, isLastTab: true };
        window.dispatchEvent(dragEvent("drop", windowTabDragData(lone)));
        expect(merge).toHaveBeenCalledWith(lone, expect.anything());
        expect(claim).not.toHaveBeenCalled();
    });

    // The drag library ends the drag session in its own drop handler, before
    // this window-level listener sees the drop: an in-window reorder must
    // still not read as a drop from elsewhere (#4706).
    it("leaves this window's own drags alone, through to their drop", () => {
        dispose = acceptWindowTabsFromOtherWindows("main", merge);
        window.dispatchEvent(dragEvent("dragstart"));
        const over = dragEvent("dragover");
        window.dispatchEvent(over);
        window.dispatchEvent(dragEvent("drop"));
        expect(over.defaultPrevented).toBe(false);
        expect(merge).not.toHaveBeenCalled();
        expect(claim).not.toHaveBeenCalled();
    });

    it("accepts drags from elsewhere again once its own drag has ended", () => {
        dispose = acceptWindowTabsFromOtherWindows("main", merge);
        window.dispatchEvent(dragEvent("dragstart"));
        window.dispatchEvent(dragEvent("dragend"));
        window.dispatchEvent(dragEvent("drop"));
        expect(merge).toHaveBeenCalled();
    });

    it("leaves other drags alone, and a marker without readable details", () => {
        dispose = acceptWindowTabsFromOtherWindows("main", merge);
        const over = dragEvent("dragover", { Files: "" });
        window.dispatchEvent(over);
        window.dispatchEvent(dragEvent("drop", { Files: "" }));
        window.dispatchEvent(dragEvent("drop", { [WINDOW_TAB_MIME]: "not json" }));
        expect(over.defaultPrevented).toBe(false);
        expect(merge).not.toHaveBeenCalled();
        expect(claim).not.toHaveBeenCalled();
    });

    it("does nothing off Linux", () => {
        linux = false;
        dispose = acceptWindowTabsFromOtherWindows("main", merge);
        const over = dragEvent("dragover");
        window.dispatchEvent(over);
        expect(over.defaultPrevented).toBe(false);
    });
});
