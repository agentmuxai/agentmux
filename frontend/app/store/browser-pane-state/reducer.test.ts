// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Pure-reducer tests for the browser pane. No mocks, no SolidJS, no
// IPC — just `update(state, command) → { state, events }`. The state-
// transition rules are documented in docs/specs/browser-pane-reducer.md
// and the inline invariants in reducer.ts.

import { describe, expect, it } from "vitest";
import { deriveFavicon, normalizeUrl, update } from "./reducer";
import { initialState, type BrowserPaneCommand, type BrowserPaneState } from "./types";

const BLOCK = "test-block";

function s(overrides: Partial<BrowserPaneState> = {}): BrowserPaneState {
    return { ...initialState(BLOCK), ...overrides };
}

describe("deriveFavicon", () => {
    it("returns origin/favicon.ico for an https URL", () => {
        expect(deriveFavicon("https://example.com/some/page")).toBe("https://example.com/favicon.ico");
    });
    it("preserves subdomains", () => {
        expect(deriveFavicon("https://docs.example.com/foo")).toBe("https://docs.example.com/favicon.ico");
    });
    it("returns empty for about:blank", () => {
        expect(deriveFavicon("about:blank")).toBe("");
    });
    it("returns empty for malformed URL", () => {
        expect(deriveFavicon("::not-a-url::")).toBe("");
    });
    it("returns empty for empty string", () => {
        expect(deriveFavicon("")).toBe("");
    });
});

describe("normalizeUrl", () => {
    it("preserves https URLs", () => {
        expect(normalizeUrl("https://example.com")).toBe("https://example.com");
    });
    it("preserves about: URLs", () => {
        expect(normalizeUrl("about:blank")).toBe("about:blank");
    });
    it("adds https:// to bare domains", () => {
        expect(normalizeUrl("example.com")).toBe("https://example.com");
    });
    it("treats text-with-spaces as a search query", () => {
        expect(normalizeUrl("hello world")).toBe("https://www.google.com/search?q=hello%20world");
    });
    it("trims whitespace", () => {
        expect(normalizeUrl("  https://example.com  ")).toBe("https://example.com");
    });
    it("returns empty for whitespace-only input", () => {
        expect(normalizeUrl("   ")).toBe("");
    });
});

describe("update — closed terminal invariant", () => {
    it("ignores non-Disposed commands after Disposed", () => {
        const r1 = update(s(), { type: "Disposed" });
        expect(r1.state.closed).toBe(true);
        const r2 = update(r1.state, { type: "NavigateRequested", url: "https://example.com" });
        expect(r2.state).toBe(r1.state);
        expect(r2.events).toEqual([]);
    });
    it("Disposed is idempotent", () => {
        const r1 = update(s(), { type: "Disposed" });
        const r2 = update(r1.state, { type: "Disposed" });
        expect(r2.state.closed).toBe(true);
    });
});

describe("update — NavigateRequested", () => {
    it("normalizes URL and sets loading + clears favicon/error", () => {
        const r = update(
            s({ faviconUrl: "https://stale.com/favicon.ico", error: "boom", title: "Old" }),
            { type: "NavigateRequested", url: "example.com" },
        );
        expect(r.state.url).toBe("https://example.com");
        expect(r.state.loading).toBe(true);
        expect(r.state.faviconUrl).toBe("");
        expect(r.state.error).toBeNull();
        // Title preserved (avoids "Browser" flash mid-load)
        expect(r.state.title).toBe("Old");
    });
    it("emits ipc-navigate + meta-persist-url", () => {
        const r = update(s(), { type: "NavigateRequested", url: "https://example.com" });
        expect(r.events).toEqual([
            { type: "ipc-navigate", url: "https://example.com" },
            { type: "meta-persist-url", url: "https://example.com" },
        ]);
    });
    it("is a no-op for whitespace-only URLs", () => {
        const before = s();
        const r = update(before, { type: "NavigateRequested", url: "   " });
        expect(r.state).toBe(before);
        expect(r.events).toEqual([]);
    });
});

describe("update — NavStateReceived", () => {
    it("derives favicon from URL origin", () => {
        const r = update(s(), {
            type: "NavStateReceived",
            url: "https://example.com/path?q=1",
            urlOnly: false,
        });
        expect(r.state.faviconUrl).toBe("https://example.com/favicon.ico");
        expect(r.state.url).toBe("https://example.com/path?q=1");
    });
    it("clears favicon for about:blank", () => {
        const r = update(s({ faviconUrl: "https://stale.com/favicon.ico" }), {
            type: "NavStateReceived",
            url: "about:blank",
            urlOnly: false,
        });
        expect(r.state.faviconUrl).toBe("");
    });
    it("clears favicon (no throw) for malformed URL", () => {
        const r = update(s({ faviconUrl: "https://stale.com/favicon.ico" }), {
            type: "NavStateReceived",
            url: "::not-a-url::",
            urlOnly: false,
        });
        expect(r.state.faviconUrl).toBe("");
    });
    it("urlOnly=true does NOT touch history gates (kimi race finding)", () => {
        const before = s({ canGoBack: false, canGoForward: false });
        const r = update(before, {
            type: "NavStateReceived",
            url: "https://example.com",
            canGoBack: true,
            canGoForward: true,
            urlOnly: true,
        });
        expect(r.state.canGoBack).toBe(false);
        expect(r.state.canGoForward).toBe(false);
    });
    it("urlOnly=false updates history gates from payload", () => {
        const r = update(s(), {
            type: "NavStateReceived",
            url: "https://example.com",
            canGoBack: true,
            canGoForward: false,
            urlOnly: false,
        });
        expect(r.state.canGoBack).toBe(true);
        expect(r.state.canGoForward).toBe(false);
    });
    it("clears loading and error", () => {
        const r = update(s({ loading: true, error: "fizz" }), {
            type: "NavStateReceived",
            url: "https://example.com",
            urlOnly: false,
        });
        expect(r.state.loading).toBe(false);
        expect(r.state.error).toBeNull();
    });
    it("emits meta-persist-url", () => {
        const r = update(s(), {
            type: "NavStateReceived",
            url: "https://example.com",
            urlOnly: false,
        });
        expect(r.events).toEqual([{ type: "meta-persist-url", url: "https://example.com" }]);
    });
});

describe("update — TitleChangeReceived", () => {
    it("sets title when non-empty", () => {
        const r = update(s(), { type: "TitleChangeReceived", title: "Live Page" });
        expect(r.state.title).toBe("Live Page");
        expect(r.events).toEqual([]);
    });
    it("falls back to 'Browser' on empty title", () => {
        const r = update(s({ title: "Old" }), { type: "TitleChangeReceived", title: "" });
        expect(r.state.title).toBe("Browser");
    });
});

describe("update — Back / Forward / Reload", () => {
    it("BackRequested sets loading + emits ipc-back", () => {
        const r = update(s({ error: "old" }), { type: "BackRequested" });
        expect(r.state.loading).toBe(true);
        expect(r.state.error).toBeNull();
        expect(r.events).toEqual([{ type: "ipc-back" }]);
    });
    it("ForwardRequested sets loading + emits ipc-forward", () => {
        const r = update(s(), { type: "ForwardRequested" });
        expect(r.state.loading).toBe(true);
        expect(r.events).toEqual([{ type: "ipc-forward" }]);
    });
    it("ReloadRequested re-issues current URL via ipc-navigate", () => {
        const r = update(s({ url: "https://example.com" }), { type: "ReloadRequested" });
        expect(r.state.loading).toBe(true);
        expect(r.events).toEqual([{ type: "ipc-navigate", url: "https://example.com" }]);
    });
});

describe("update — LoadError", () => {
    it("sets error and clears loading", () => {
        const r = update(s({ loading: true }), { type: "LoadError", message: "ERR_NAME_NOT_RESOLVED" });
        expect(r.state.loading).toBe(false);
        expect(r.state.error).toBe("ERR_NAME_NOT_RESOLVED");
    });
});

describe("update — Clicked", () => {
    it("emits focus-block, no state change", () => {
        const before = s();
        const r = update(before, { type: "Clicked" });
        expect(r.state).toBe(before);
        expect(r.events).toEqual([{ type: "focus-block" }]);
    });
});

describe("update — Disposed", () => {
    it("flips closed=true and emits shutdown", () => {
        const r = update(s(), { type: "Disposed" });
        expect(r.state.closed).toBe(true);
        expect(r.events).toEqual([{ type: "shutdown" }]);
    });
});
