// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * showCopyContextMenu — the shared "Copy <thing>" menu for `user-select: none`
 * rows (Swarm sub-rows, Drone nodes). Goes through the real ContextMenuModel;
 * only the native bridge (`getApi().showContextMenu`) and the clipboard are
 * mocked.
 */

import { afterEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
    showContextMenu: vi.fn(),
    writeText: vi.fn(() => Promise.resolve()),
}));

vi.mock("@/util/clipboard", () => ({ writeText: mocks.writeText, readText: () => Promise.resolve("") }));
vi.mock("@/app/store/global", () => ({
    openLink: vi.fn(),
    getApi: () => ({ showContextMenu: mocks.showContextMenu }),
    atoms: { workspace: () => ({ oid: "ws1" }) },
}));

import { ContextMenuModel, showCopyContextMenu } from "./contextmenu";

afterEach(() => vi.clearAllMocks());

function rightClick(): MouseEvent {
    return new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 10, clientY: 20 });
}

function shownLabels(): string[] {
    return mocks.showContextMenu.mock.calls.at(-1)![1].map((i: { label: string }) => i.label);
}

describe("showCopyContextMenu", () => {
    it("shows one item per entry, claims the event, and copies the entry's value on click", () => {
        const e = rightClick();
        const stop = vi.spyOn(e, "stopPropagation");
        const shown = showCopyContextMenu(
            [
                { label: "Copy a", value: "AAA" },
                { label: "Copy b", value: "BBB" },
            ],
            e
        );
        expect(shown).toBe(true);
        expect(e.defaultPrevented).toBe(true);
        expect(stop).toHaveBeenCalled();
        expect(shownLabels()).toEqual(["Copy a", "Copy b"]);
        expect(mocks.showContextMenu.mock.calls[0][2]).toEqual({ x: 10, y: 20 });

        const items = mocks.showContextMenu.mock.calls[0][1] as { label: string; id: string }[];
        ContextMenuModel.handleContextMenuClick(items[1].id);
        expect(mocks.writeText).toHaveBeenCalledExactlyOnceWith("BBB");
    });

    it("drops entries with an empty or missing value", () => {
        showCopyContextMenu(
            [
                { label: "Copy a", value: "AAA" },
                { label: "Copy empty", value: "" },
                { label: "Copy null", value: null },
                { label: "Copy undefined", value: undefined },
            ],
            rightClick()
        );
        expect(shownLabels()).toEqual(["Copy a"]);
    });

    it("does NOT consume the event when there is nothing to copy, so the pane menu still gets it", () => {
        const e = rightClick();
        const stop = vi.spyOn(e, "stopPropagation");
        expect(showCopyContextMenu([{ label: "Copy a", value: "" }], e)).toBe(false);
        expect(showCopyContextMenu([], e)).toBe(false);
        expect(e.defaultPrevented).toBe(false);
        expect(stop).not.toHaveBeenCalled();
        expect(mocks.showContextMenu).not.toHaveBeenCalled();
    });
});
