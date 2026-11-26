// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Kanban's place for the window's trail: what it reads, and how a restore puts a card back. */

import { createRoot } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/mos", () => ({ getObjectValue: () => undefined, makeORef: (t: string, id: string) => `${t}:${id}` }));

import { placeSourceOf, resetPlaceSourcesForTests } from "@/app/store/place-sources";
import { withKanbanPlace } from "./kanban-place";
import type { KanbanSectionState, SectionProps } from "./kanban-section-state";

const everything = {
    default_board: "main",
    boards: [{ slug: "main" }, { slug: "work" }],
    tasks: [
        { id: "t1", card: { board: "work", num: 1 } },
        { id: "t2", card: { board: "main", num: 2 } },
    ],
};

function mount(loaded: unknown) {
    const calls: string[] = [];
    let selected = "main";
    const base = {
        selectedSlug: () => selected,
        selectBoard: (slug: string) => void ((selected = slug), calls.push(`board:${slug}`)),
        setOpenId: (id: string | null) => void calls.push(`open:${id}`),
        openTask: () => null,
        model: { everything: () => loaded },
    } as unknown as KanbanSectionState;
    const dispose = createRoot((d) => {
        withKanbanPlace(base, { placeBlockId: "K" } as SectionProps);
        return d;
    });
    return { calls, dispose };
}

afterEach(() => resetPlaceSourcesForTests());

describe("the Kanban's place restore", () => {
    it("opens a card on the board it is on now, even when it moved since the place was recorded", () => {
        const { calls, dispose } = mount(everything);
        // The place was recorded on board "main"; the task has since moved to "work".
        expect(placeSourceOf("K")?.restore({ board: "main", task: "t1" })).toBe(true);
        expect(calls).toEqual(["board:work", "open:t1"]);
        dispose();
    });

    it("says the place is gone for a task that is on no board, and shows the plain board", () => {
        const { calls, dispose } = mount(everything);
        expect(placeSourceOf("K")?.restore({ board: "main", task: "deleted" })).toBe(false);
        expect(calls).toEqual([]);
        expect(placeSourceOf("K")?.restore({ board: "work" })).toBe(true);
        expect(calls).toEqual(["board:work", "open:null"]);
        dispose();
    });

    it("shows a card once the board arrives when it is not loaded yet", () => {
        const { calls, dispose } = mount(null);
        expect(placeSourceOf("K")?.restore({ board: "main", task: "t2" })).toBe(true);
        expect(calls).toEqual(["open:t2"]);
        dispose();
    });
});
