// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot } from "solid-js";
import { describe, expect, it, vi } from "vitest";
import { createTileReorder, movedAnnouncement, REORDER_MIME } from "./attachment-tile-reorder";

type Item = { key: string; name: string };

function setup(names: string[]) {
    let items: Item[] = names.map((n) => ({ key: n, name: n }));
    const d = {
        items: () => items,
        moveNear: vi.fn((key: string, target: string, after: boolean) => {
            const rest = items.filter((i) => i.key !== key);
            rest.splice(rest.findIndex((i) => i.key === target) + (after ? 1 : 0), 0, items.find((i) => i.key === key)!);
            items = rest;
        }),
    };
    const said: string[] = [];
    const reorderFor = createRoot(() => createTileReorder(d as never, (t) => said.push(t)));
    return { d, said, reorderFor, order: () => items.map((i) => i.key) };
}

const tileEl = { getBoundingClientRect: () => ({ left: 100, width: 64 }) } as unknown as HTMLElement;
function dragEvent(x = 0, dt: Partial<DataTransfer> | null = {}) {
    return { clientX: x, currentTarget: tileEl, dataTransfer: dt, preventDefault: vi.fn(), stopPropagation: vi.fn() } as unknown as DragEvent & {
        preventDefault: ReturnType<typeof vi.fn>;
        stopPropagation: ReturnType<typeof vi.fn>;
    };
}
const start = () => dragEvent(0, { setData: vi.fn() });

describe("dragging a tile into a new order", () => {
    it("carries its own type and marks the held tile", () => {
        const { reorderFor } = setup(["a", "b"]);
        const setData = vi.fn();
        const dt = { setData } as unknown as DataTransfer;
        reorderFor("a").onDragStart(dragEvent(0, dt));
        expect(setData).toHaveBeenCalledWith(REORDER_MIME, "a");
        expect(dt.effectAllowed).toBe("move");
        expect(reorderFor("a").dragging).toBe(true);
        expect(reorderFor("b").dragging).toBe(false);
    });

    it("ignores a drag it did not start, such as a file from the desktop", () => {
        const { reorderFor } = setup(["a", "b"]);
        const e = dragEvent(150);
        reorderFor("b").onDragOver(e);
        expect(e.preventDefault).not.toHaveBeenCalled();
        expect(reorderFor("b").drop).toBeUndefined();
        const dropped = dragEvent(150);
        reorderFor("b").onDrop(dropped);
        expect(dropped.preventDefault).not.toHaveBeenCalled();
    });

    it("shows the edge it would land on by which half the pointer is over", () => {
        const { reorderFor } = setup(["a", "b", "c"]);
        reorderFor("a").onDragStart(start());
        reorderFor("c").onDragOver(dragEvent(110));
        expect(reorderFor("c").drop).toBe("before");
        reorderFor("c").onDragOver(dragEvent(150));
        expect(reorderFor("c").drop).toBe("after");
        expect(reorderFor("b").drop).toBeUndefined();
        expect(reorderFor("a").drop).toBeUndefined();
    });

    it("drops after the tile it is over, moves it, says where it went and clears", () => {
        const { d, said, reorderFor, order } = setup(["a", "b", "c"]);
        reorderFor("a").onDragStart(start());
        reorderFor("c").onDragOver(dragEvent(150));
        const e = dragEvent(150);
        reorderFor("c").onDrop(e);
        expect(e.preventDefault).toHaveBeenCalled();
        expect(d.moveNear).toHaveBeenCalledWith("a", "c", true);
        expect(order()).toEqual(["b", "c", "a"]);
        expect(said).toEqual(["Moved a to position 3 of 3."]);
        expect(reorderFor("a").dragging).toBe(false);
        expect(reorderFor("c").drop).toBeUndefined();
    });

    it("drops before the tile when the pointer is on its first half", () => {
        const { d, reorderFor, order } = setup(["a", "b", "c"]);
        reorderFor("c").onDragStart(start());
        reorderFor("a").onDragOver(dragEvent(105));
        reorderFor("a").onDrop(dragEvent(105));
        expect(d.moveNear).toHaveBeenCalledWith("c", "a", false);
        expect(order()).toEqual(["c", "a", "b"]);
    });

    it("a drop on itself, or a drag that ends elsewhere, changes nothing", () => {
        const { d, said, reorderFor } = setup(["a", "b"]);
        reorderFor("a").onDragStart(start());
        reorderFor("a").onDrop(dragEvent(150));
        expect(d.moveNear).not.toHaveBeenCalled();
        expect(said).toEqual([]);
        reorderFor("b").onDragStart(start());
        reorderFor("a").onDragOver(dragEvent(150));
        reorderFor("b").onDragEnd();
        expect(reorderFor("b").dragging).toBe(false);
        expect(reorderFor("a").drop).toBeUndefined();
    });
});

describe("the announcement after a move", () => {
    it("names the tile and its new place, or nothing for a tile that is gone", () => {
        const d = {
            items: () => [
                { key: "x", name: "x.png" },
                { key: "y", name: "y.png" },
            ],
        };
        expect(movedAnnouncement(d as never, "y")).toBe("Moved y.png to position 2 of 2.");
        expect(movedAnnouncement(d as never, "z")).toBeNull();
    });
});
