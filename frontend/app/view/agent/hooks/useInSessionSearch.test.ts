// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { recycleBlockers } from "@/app/store/recycle-blockers";
import type { DocumentNode } from "../../../agent-model/types";
import { SEARCH_DEBOUNCE_MS, SEARCH_REFRESH_MS, SEARCH_SLOW_MS, useInSessionSearch, type UseInSessionSearchOptions } from "./useInSessionSearch";

describe("useInSessionSearch", () => {
    it("an open search blocks a renderer recycle until it is closed", () => {
        const { search, dispose } = createRoot((d) => ({ search: useInSessionSearch({ document: () => [] }), dispose: d }));
        search.setVisible(true);
        expect(recycleBlockers()).toContain("a transcript search is open");
        search.close();
        expect(recycleBlockers()).not.toContain("a transcript search is open");
        dispose();
    });

    it("tells the caller when the search ends", () => {
        const onClose = vi.fn();
        const { search, dispose } = createRoot((d) => ({ search: useInSessionSearch({ document: () => [], onClose }), dispose: d }));
        search.setVisible(true);
        expect(onClose).not.toHaveBeenCalled();
        search.close();
        expect(onClose).toHaveBeenCalledOnce();
        dispose();
    });
});

const md = (id: string, content: string): DocumentNode => ({ type: "markdown", id, content }) as DocumentNode;
const immediate = { now: () => 0, yieldNow: () => Promise.resolve() };

describe("useInSessionSearch: the pass", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    function open(initial: DocumentNode[], extra: Partial<UseInSessionSearchOptions> = {}) {
        const [doc, setDoc] = createSignal(initial);
        const jumpTo = vi.fn();
        const onNoMatch = vi.fn();
        const made = createRoot((dispose) => ({
            search: useInSessionSearch({ document: doc, jumpTo, onNoMatch, clock: immediate, ...extra }),
            dispose,
        }));
        made.search.setVisible(true);
        return { ...made, setDoc, jumpTo, onNoMatch };
    }

    it("waits for a pause in typing, then searches once for the last text", async () => {
        const { search, jumpTo, dispose } = open([md("a", "alpha"), md("b", "beta")]);
        search.performSearch("a");
        search.performSearch("al");
        search.performSearch("alp");
        await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS - 1);
        expect(search.matches()).toEqual([]);
        await vi.advanceTimersByTimeAsync(1);
        expect(search.matches()).toEqual(["a"]);
        expect(jumpTo).toHaveBeenCalledExactlyOnceWith("a");
        dispose();
    });

    it("starts at the first match from the row at the top of the screen", async () => {
        const doc = [md("a", "hit"), md("b", "no"), md("c", "hit"), md("d", "hit")];
        const { search, jumpTo, dispose } = open(doc, { topNodeId: () => "b" });
        search.performSearch("hit");
        await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS);
        expect(search.matches()).toEqual(["a", "c", "d"]);
        expect(search.currentIndex()).toBe(1);
        expect(jumpTo).toHaveBeenCalledExactlyOnceWith("c");
        dispose();
    });

    it("keeps the current match by id while it still matches, and moves on from the top row when it does not", async () => {
        const doc = [md("a", "hello"), md("b", "help"), md("c", "hello again")];
        const { search, jumpTo, dispose } = open(doc, { topNodeId: () => "b" });
        search.performSearch("hel");
        await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS);
        expect(search.highlightId()).toBe("b");
        search.performSearch("hell");
        await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS);
        // "b" no longer matches: the first from the top row ("b") is "c".
        expect(search.highlightId()).toBe("c");
        search.performSearch("hello a");
        await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS);
        expect(search.highlightId()).toBe("c");
        expect(jumpTo.mock.calls).toEqual([["b"], ["c"]]);
        dispose();
    });

    it("adds rows that arrive while it is open, without moving the view or the current match", async () => {
        const doc = [md("a", "hit one"), md("b", "hit two")];
        const { search, setDoc, jumpTo, dispose } = open(doc);
        search.performSearch("hit");
        await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS);
        search.next();
        expect(search.highlightId()).toBe("b");
        jumpTo.mockClear();
        setDoc([md("z", "hit zero"), ...doc, md("c", "hit three")]);
        await vi.advanceTimersByTimeAsync(SEARCH_REFRESH_MS);
        expect(search.matches()).toEqual(["z", "a", "b", "c"]);
        expect(search.highlightId()).toBe("b");
        expect(search.currentIndex()).toBe(2);
        expect(jumpTo).not.toHaveBeenCalled();
        dispose();
    });

    it("refreshes a growing transcript at most every refresh interval, not once per change", async () => {
        const base = [md("a", "hit")];
        const { search, setDoc, dispose } = open(base);
        search.performSearch("hit");
        await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS);
        for (let i = 0; i < 20; i++) {
            setDoc([...base, md(`n${i}`, "hit")]);
            await vi.advanceTimersByTimeAsync(5);
        }
        // 100 ms of changes: no refresh has run yet.
        expect(search.matches()).toEqual(["a"]);
        await vi.advanceTimersByTimeAsync(SEARCH_REFRESH_MS);
        expect(search.matches()).toEqual(["a", "n19"]);
        dispose();
    });

    it("an empty query clears the matches and says no match is current", async () => {
        const { search, onNoMatch, dispose } = open([md("a", "hit")]);
        search.performSearch("hit");
        await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS);
        expect(search.matchCount()).toBe(1);
        search.performSearch("   ");
        await vi.advanceTimersByTimeAsync(0);
        expect(search.matches()).toEqual([]);
        expect(search.currentIndex()).toBe(-1);
        expect(onNoMatch).toHaveBeenCalled();
        dispose();
    });

    it("a query with no match says so too", async () => {
        const { search, onNoMatch, dispose } = open([md("a", "hit")]);
        search.performSearch("zzz");
        await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS);
        expect(search.matchCount()).toBe(0);
        expect(onNoMatch).toHaveBeenCalledOnce();
        dispose();
    });

    it("Enter pressed before the pause is over runs the search now", async () => {
        const { search, jumpTo, dispose } = open([md("a", "hit")]);
        search.performSearch("hit");
        search.next();
        await vi.advanceTimersByTimeAsync(0);
        expect(search.matches()).toEqual(["a"]);
        expect(jumpTo).toHaveBeenCalledExactlyOnceWith("a");
        dispose();
    });

    it("steps forward and back around the matches", async () => {
        const { search, jumpTo, dispose } = open([md("a", "hit"), md("b", "hit"), md("c", "hit")]);
        search.performSearch("hit");
        await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS);
        search.prev();
        search.prev();
        search.next();
        search.next();
        search.next();
        expect(jumpTo.mock.calls.map((c) => c[0])).toEqual(["a", "c", "b", "c", "a", "b"]);
        dispose();
    });

    it("says Searching only when a pass takes longer than the slow limit, and drops a pass the bar outlived", async () => {
        let release: () => void = () => {};
        let t = 0;
        const clock = {
            now: () => (t += 100),
            yieldNow: () => new Promise<void>((resolve) => (release = resolve)),
        };
        const nodes = Array.from({ length: 5000 }, (_, i) => md(`n${i}`, "x"));
        const { search, jumpTo, dispose } = open(nodes, { clock });
        search.performSearch("x");
        await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS);
        expect(search.searching()).toBe(false);
        await vi.advanceTimersByTimeAsync(SEARCH_SLOW_MS);
        expect(search.searching()).toBe(true);
        search.close();
        release();
        await vi.advanceTimersByTimeAsync(0);
        expect(search.searching()).toBe(false);
        expect(search.matches()).toEqual([]);
        expect(jumpTo).not.toHaveBeenCalled();
        dispose();
    });
});
