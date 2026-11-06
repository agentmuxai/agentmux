// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import type { DocumentNode, HiddenRunNode } from "../../agent-model/types";
import { createSearchReveal } from "./search-reveal";

function run(id: string, hiddenIds: string[], open = false): HiddenRunNode {
    return { type: "hidden_run", id, counts: [{ kind: "tool", count: hiddenIds.length }], failed: 0, open, hiddenIds };
}

function setup(shown: DocumentNode[], pinned: string[] = []) {
    let state = { pinnedNodes: new Set(pinned) };
    const jumpTo = vi.fn();
    const reveal = createSearchReveal({
        shown: () => shown,
        setPins: (update) => {
            state = { pinnedNodes: update(state.pinnedNodes) };
        },
        jumpTo,
        afterOpen: (jump) => jump(),
    });
    return { reveal, jumpTo, pins: () => [...state.pinnedNodes].sort() };
}

describe("createSearchReveal (a search match inside a hidden run, #10718)", () => {
    it("opens the run holding the match, then jumps to the match", () => {
        const { reveal, jumpTo, pins } = setup([run("hidden-run:a", ["a", "b"]), run("hidden-run:c", ["c"])]);
        reveal.jumpTo("b");
        expect(pins()).toEqual(["hidden-run:a"]);
        expect(jumpTo).toHaveBeenCalledExactlyOnceWith("b");
    });

    it("just jumps when the match is already on screen", () => {
        const { reveal, jumpTo, pins } = setup([run("hidden-run:a", ["a"])]);
        reveal.jumpTo("shown-row");
        expect(pins()).toEqual([]);
        expect(jumpTo).toHaveBeenCalledExactlyOnceWith("shown-row");
    });

    it("waits for the opened run to be laid out before jumping", () => {
        const jumpTo = vi.fn();
        let layout: () => void = () => {};
        const reveal = createSearchReveal({
            shown: () => [run("hidden-run:a", ["a"])],
            setPins: () => {},
            jumpTo,
            afterOpen: (jump) => (layout = jump),
        });
        reveal.jumpTo("a");
        expect(jumpTo).not.toHaveBeenCalled();
        layout();
        expect(jumpTo).toHaveBeenCalledExactlyOnceWith("a");
    });

    it("drops a waiting jump when a newer match was asked for", () => {
        const jumpTo = vi.fn();
        const waiting: (() => void)[] = [];
        const reveal = createSearchReveal({
            shown: () => [run("hidden-run:a", ["a"]), run("hidden-run:c", ["c"])],
            setPins: () => {},
            jumpTo,
            afterOpen: (jump) => waiting.push(jump),
        });
        reveal.jumpTo("a");
        reveal.jumpTo("c");
        reveal.jumpTo("on-screen");
        waiting.forEach((jump) => jump());
        expect(jumpTo.mock.calls).toEqual([["on-screen"]]);
    });

    it("drops a waiting jump when the search ends before the run is laid out", () => {
        const jumpTo = vi.fn();
        let layout: () => void = () => {};
        const reveal = createSearchReveal({
            shown: () => [run("hidden-run:a", ["a"])],
            setPins: () => {},
            jumpTo,
            afterOpen: (jump) => (layout = jump),
        });
        reveal.jumpTo("a");
        reveal.release();
        layout();
        expect(jumpTo).not.toHaveBeenCalled();
    });

    it("leaves a run the person already opened alone, also when the search closes", () => {
        const { reveal, pins } = setup([run("hidden-run:a", ["a"], true)], ["hidden-run:a"]);
        reveal.jumpTo("a");
        reveal.release();
        expect(pins()).toEqual(["hidden-run:a"]);
    });

    it("closes again the runs the search opened when it ends, and no others", () => {
        const { reveal, pins } = setup(
            [run("hidden-run:a", ["a"]), run("hidden-run:c", ["c"]), run("hidden-run:e", ["e"])],
            ["hidden-run:e", "some-tool"]
        );
        reveal.jumpTo("a");
        reveal.jumpTo("c");
        expect(pins()).toEqual(["hidden-run:a", "hidden-run:c", "hidden-run:e", "some-tool"]);
        reveal.release();
        expect(pins()).toEqual(["hidden-run:e", "some-tool"]);
    });

    it("release with nothing opened changes nothing", () => {
        const setPins = vi.fn();
        const reveal = createSearchReveal({ shown: () => [], setPins, jumpTo: vi.fn() });
        reveal.release();
        expect(setPins).not.toHaveBeenCalled();
    });
});
