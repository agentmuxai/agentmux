// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
    getComposerDraft,
    getSentHistory,
    resetReloadPersistenceForTests,
    saveSentHistory,
    setComposerDraft,
} from "./reload-persistence";

// `resetReloadPersistenceForTests` does what a reload does to this module: the
// pending writes are flushed (pagehide) and the in-memory copies are gone.
describe("reload persistence", () => {
    beforeEach(() => {
        vi.useFakeTimers();
        sessionStorage.clear();
        resetReloadPersistenceForTests();
    });
    afterEach(() => {
        vi.useRealTimers();
        sessionStorage.clear();
    });

    it("keeps a composer draft across a reload", () => {
        setComposerDraft("b1", "half a thought");
        resetReloadPersistenceForTests();
        expect(getComposerDraft("b1")).toBe("half a thought");
        expect(getComposerDraft("b2")).toBeUndefined();
    });

    it("debounces the stored draft, and pagehide flushes it", () => {
        setComposerDraft("b1", "a");
        setComposerDraft("b1", "ab");
        expect(sessionStorage.getItem("agentmux:composer-draft:b1")).toBeNull();
        window.dispatchEvent(new Event("pagehide"));
        expect(JSON.parse(sessionStorage.getItem("agentmux:composer-draft:b1")!)).toBe("ab");
        setComposerDraft("b1", "abc");
        vi.advanceTimersByTime(300);
        expect(JSON.parse(sessionStorage.getItem("agentmux:composer-draft:b1")!)).toBe("abc");
    });

    it("an emptied draft is removed, not restored", () => {
        setComposerDraft("b1", "text");
        vi.advanceTimersByTime(300);
        setComposerDraft("b1", "");
        resetReloadPersistenceForTests();
        expect(sessionStorage.getItem("agentmux:composer-draft:b1")).toBeNull();
        expect(getComposerDraft("b1")).toBeUndefined();
    });

    it("keeps the sent-message history across a reload, in order", () => {
        const history = getSentHistory("b1");
        history.push("first", "second");
        saveSentHistory("b1");
        expect(getSentHistory("b1")).toBe(history); // one array per block for the page's lifetime
        resetReloadPersistenceForTests();
        expect(getSentHistory("b1")).toEqual(["first", "second"]);
        expect(getSentHistory("b2")).toEqual([]);
    });

    it("ignores stored values it can't read", () => {
        sessionStorage.setItem("agentmux:sent-history:b1", "{not json");
        sessionStorage.setItem("agentmux:sent-history:b2", JSON.stringify(["ok", 3, null]));
        expect(getSentHistory("b1")).toEqual([]);
        expect(getSentHistory("b2")).toEqual(["ok"]);
    });
});
