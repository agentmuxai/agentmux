// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * One set of window drag listeners, dispatching by drag kind, and the file
 * drop backstop. SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md §5.2.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { beginDrag, endDrag } from "./drag-session";
import { onWindowDrag } from "./window-drag-events";

const stops: (() => void)[] = [];
afterEach(() => {
    stops.splice(0).forEach((stop) => stop());
    endDrag("cancel");
});

function subscribe(kinds: Parameters<typeof onWindowDrag>[0]["kinds"]) {
    const over = vi.fn();
    const drop = vi.fn();
    stops.push(onWindowDrag({ kinds, over, drop }));
    return { over, drop };
}

/** A drag event bubbling from the body, with the given DataTransfer types. */
function fire(type: "dragover" | "drop", types: string[] = []): Event {
    const e = new Event(type, { bubbles: true, cancelable: true });
    Object.defineProperty(e, "dataTransfer", { value: { types, dropEffect: "none" } });
    document.body.dispatchEvent(e);
    return e;
}

describe("window drag events", () => {
    it("dispatches only to subscribers for the current drag's kind", () => {
        const tile = subscribe(["tile"]);
        const tab = subscribe(["window-tab"]);
        beginDrag("tile", { nodeId: "n1" });
        fire("dragover");
        expect(tile.over).toHaveBeenCalledOnce();
        expect(tab.over).not.toHaveBeenCalled();
    });

    it("dispatches drop as well as dragover", () => {
        const tab = subscribe(["window-tab"]);
        beginDrag("window-tab", { tabId: "t1" });
        fire("drop");
        expect(tab.drop).toHaveBeenCalledOnce();
    });

    it("with no drag in this window nothing is dispatched, and the event is left alone", () => {
        const any = subscribe(["tile", "window-tab", "pane-tab"]);
        const e = fire("dragover", ["text/plain"]);
        expect(any.over).not.toHaveBeenCalled();
        expect(e.defaultPrevented).toBe(false);
    });

    it("an OS file drag counts as files, even with another session stranded", () => {
        const files = subscribe(["files"]);
        const tile = subscribe(["tile"]);
        beginDrag("tile", { nodeId: "n1" });
        fire("dragover", ["Files"]);
        expect(files.over).toHaveBeenCalledOnce();
        expect(tile.over).not.toHaveBeenCalled();
    });

    it("an unhandled file dragover or drop never reaches the browser's default (it would navigate the window away)", () => {
        expect(fire("dragover", ["Files"]).defaultPrevented).toBe(true);
        expect(fire("drop", ["Files"]).defaultPrevented).toBe(true);
    });

    it("leaves a text drag to the browser, e.g. dropping text into the composer", () => {
        expect(fire("drop", ["text/plain"]).defaultPrevented).toBe(false);
    });

    it("a disposed subscriber gets nothing more", () => {
        const over = vi.fn();
        const stop = onWindowDrag({ kinds: ["tile"], over });
        stop();
        beginDrag("tile", { nodeId: "n1" });
        fire("dragover");
        expect(over).not.toHaveBeenCalled();
    });
});
