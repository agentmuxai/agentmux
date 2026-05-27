// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { update } from "./reducer";
import {
    BrowserPaneState,
    BrowserTab,
    deriveFaviconUrl,
    initialState,
    TITLE_FALLBACK,
} from "./types";

/**
 * Phase 1A test suite — covers the multi-tab reducer.
 *
 * The legacy single-URL tests are migrated by bootstrapping a one-tab
 * pane via `OpenTab` before each scenario; the active-tab projection
 * model means commands like `Navigate`, `UrlConfirmed`, etc. behave
 * identically against the active tab. The active-tab helper below
 * keeps the migrated assertions readable.
 */
function activeTab(state: BrowserPaneState): BrowserTab {
    if (state.activeTabId == null) {
        throw new Error("test bug: no active tab to inspect");
    }
    const t = state.tabs.find((x) => x.id === state.activeTabId);
    if (!t) throw new Error("test bug: activeTabId points to a missing tab");
    return t;
}

/** Bootstrap a pane with one tab at the given URL and return the
 *  resulting state. The tab is created via `OpenTab` and starts in
 *  the same "load just kicked off" shape that a real OpenTab would
 *  produce. */
function bootOneTab(url: string = ""): BrowserPaneState {
    return update(initialState(), { type: "OpenTab", url }).state;
}

describe("deriveFaviconUrl", () => {
    it("returns empty for empty input", () => {
        expect(deriveFaviconUrl("")).toBe("");
    });
    it("returns origin/favicon.ico for a normal https URL", () => {
        expect(deriveFaviconUrl("https://example.com/path?q=1")).toBe(
            "https://example.com/favicon.ico",
        );
    });
    it("preserves port + scheme on the origin", () => {
        expect(deriveFaviconUrl("http://localhost:3000/foo")).toBe(
            "http://localhost:3000/favicon.ico",
        );
    });
    it("returns empty for unparseable URLs", () => {
        expect(deriveFaviconUrl("not a url")).toBe("");
        expect(deriveFaviconUrl("://broken")).toBe("");
    });
    it("returns empty for about:blank (origin is 'null')", () => {
        expect(deriveFaviconUrl("about:blank")).toBe("");
    });
});

describe("browser-pane-state reducer (Phase 1A — multi-tab)", () => {
    describe("Navigate (active-tab)", () => {
        it("sets loading=true and clears any prior error on the active tab", () => {
            const s0 = bootOneTab("https://prev.com");
            const s1 = update(s0, { type: "LoadFailed", reason: "DNS fail" }).state;
            expect(activeTab(s1).error).toBe("DNS fail");
            const r = update(s1, { type: "Navigate", url: "https://x" });
            expect(activeTab(r.state).loading).toBe(true);
            expect(activeTab(r.state).error).toBeNull();
            expect(r.events).toEqual([{ type: "navigate", url: "https://x" }]);
        });

        it("preserves mutual exclusion: never sets loading and error simultaneously", () => {
            const s0 = bootOneTab();
            const r = update(s0, { type: "Navigate", url: "https://x" });
            const t = activeTab(r.state);
            expect(t.loading && t.error !== null).toBe(false);
        });

        it("sets active tab's url atomically with the loading flip", () => {
            const s0 = bootOneTab();
            const r = update(s0, {
                type: "Navigate",
                url: "https://example.com",
            });
            expect(activeTab(r.state).url).toBe("https://example.com");
            expect(activeTab(r.state).loading).toBe(true);
        });

        it("supersedes a prior url", () => {
            let s = bootOneTab();
            s = update(s, { type: "Navigate", url: "https://a.com" }).state;
            const r = update(s, { type: "Navigate", url: "https://b.com" });
            expect(activeTab(r.state).url).toBe("https://b.com");
        });

        it("no-ops when no tab is active", () => {
            const s0 = initialState();
            const r = update(s0, { type: "Navigate", url: "https://x" });
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([]);
        });
    });

    describe("faviconUrl derivation (per-tab)", () => {
        it("Navigate sets active tab faviconUrl from url's origin", () => {
            const s0 = bootOneTab();
            const r = update(s0, {
                type: "Navigate",
                url: "https://example.com/some/page",
            });
            expect(activeTab(r.state).faviconUrl).toBe(
                "https://example.com/favicon.ico",
            );
        });

        it("UrlConfirmed updates faviconUrl when origin changes (post-redirect)", () => {
            let s = bootOneTab();
            s = update(s, {
                type: "Navigate",
                url: "https://typed-by-user.com",
            }).state;
            const r = update(s, {
                type: "UrlConfirmed",
                url: "https://final-redirect-target.com/page",
            });
            expect(activeTab(r.state).faviconUrl).toBe(
                "https://final-redirect-target.com/favicon.ico",
            );
        });

        it("UrlCleared clears active tab faviconUrl alongside url", () => {
            let s = bootOneTab();
            s = update(s, {
                type: "Navigate",
                url: "https://example.com",
            }).state;
            expect(activeTab(s).faviconUrl).toBe(
                "https://example.com/favicon.ico",
            );
            const r = update(s, { type: "UrlCleared" });
            expect(activeTab(r.state).faviconUrl).toBe("");
        });

        it("Navigate to an unparseable URL leaves active tab faviconUrl empty", () => {
            const s0 = bootOneTab();
            const r = update(s0, { type: "Navigate", url: "not a url" });
            expect(activeTab(r.state).faviconUrl).toBe("");
        });
    });

    describe("UrlConfirmed (active-tab)", () => {
        it("updates active tab url to the host-confirmed value", () => {
            let s = bootOneTab();
            s = update(s, {
                type: "Navigate",
                url: "https://typed-by-user",
            }).state;
            const r = update(s, {
                type: "UrlConfirmed",
                url: "https://post-redirect-final",
            });
            expect(activeTab(r.state).url).toBe("https://post-redirect-final");
            expect(r.events).toEqual([
                { type: "url-confirmed", url: "https://post-redirect-final" },
            ]);
        });

        it("does NOT touch loading, error, or history", () => {
            let s = bootOneTab();
            s = update(s, { type: "Navigate", url: "https://x" }).state;
            s = update(s, { type: "HistoryUpdated", canGoBack: true }).state;
            const before = activeTab(s);
            const r = update(s, {
                type: "UrlConfirmed",
                url: "https://final",
            });
            const after = activeTab(r.state);
            expect(after.loading).toBe(before.loading);
            expect(after.error).toBe(before.error);
            expect(after.canGoBack).toBe(before.canGoBack);
            expect(after.canGoForward).toBe(before.canGoForward);
        });

        it("is idempotent on identical url", () => {
            let s = bootOneTab();
            s = update(s, { type: "UrlConfirmed", url: "https://x" }).state;
            const r = update(s, { type: "UrlConfirmed", url: "https://x" });
            expect(r.state).toBe(s);
            expect(r.events).toEqual([]);
        });
    });

    describe("UrlCleared (active-tab)", () => {
        it("clears active tab url to empty string", () => {
            let s = bootOneTab();
            s = update(s, { type: "Navigate", url: "https://x" }).state;
            const r = update(s, { type: "UrlCleared" });
            expect(activeTab(r.state).url).toBe("");
            expect(r.events).toEqual([{ type: "url-cleared" }]);
        });

        it("is idempotent when url is already empty", () => {
            const s0 = bootOneTab("");
            expect(activeTab(s0).url).toBe("");
            const r = update(s0, { type: "UrlCleared" });
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([]);
        });

        it("models reload's force-reload pattern (clear → restore)", () => {
            let s = bootOneTab();
            s = update(s, { type: "Navigate", url: "https://page" }).state;
            s = update(s, { type: "UrlCleared" }).state;
            expect(activeTab(s).url).toBe("");
            s = update(s, { type: "Navigate", url: "https://page" }).state;
            expect(activeTab(s).url).toBe("https://page");
            expect(activeTab(s).loading).toBe(true);
        });
    });

    describe("LoadStarted (active-tab)", () => {
        it("sets loading=true and clears any prior error", () => {
            let s = bootOneTab();
            s = update(s, { type: "LoadFailed", reason: "ssl-error" }).state;
            const r = update(s, { type: "LoadStarted" });
            expect(activeTab(r.state).loading).toBe(true);
            expect(activeTab(r.state).error).toBeNull();
            expect(r.events).toEqual([{ type: "load-started" }]);
        });

        it("preserves mutual exclusion on (loading, error)", () => {
            const s0 = bootOneTab();
            const r = update(s0, { type: "LoadStarted" });
            const t = activeTab(r.state);
            expect(t.loading && t.error !== null).toBe(false);
        });
    });

    describe("LoadFinished (active-tab)", () => {
        it("clears loading and error after a navigate", () => {
            let s = bootOneTab();
            s = update(s, { type: "Navigate", url: "https://x" }).state;
            const r = update(s, { type: "LoadFinished" });
            expect(activeTab(r.state).loading).toBe(false);
            expect(activeTab(r.state).error).toBeNull();
            expect(r.events).toEqual([{ type: "load-finished" }]);
        });

        it("clears a stale error even when loading was already false", () => {
            let s = bootOneTab();
            s = update(s, { type: "LoadFailed", reason: "boom" }).state;
            expect(activeTab(s).loading).toBe(false);
            expect(activeTab(s).error).toBe("boom");
            const r = update(s, { type: "LoadFinished" });
            expect(activeTab(r.state).error).toBeNull();
            expect(r.events).toEqual([{ type: "load-finished" }]);
        });

        it("is a no-op on steady-state (no loading, no error)", () => {
            // bootOneTab("") creates a tab with loading=false (empty url),
            // error=null — i.e. the steady-state.
            const s0 = bootOneTab("");
            expect(activeTab(s0).loading).toBe(false);
            expect(activeTab(s0).error).toBeNull();
            const r = update(s0, { type: "LoadFinished" });
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([]);
        });
    });

    describe("LoadFailed (active-tab)", () => {
        it("sets error and clears loading", () => {
            let s = bootOneTab();
            s = update(s, { type: "Navigate", url: "https://x" }).state;
            const r = update(s, { type: "LoadFailed", reason: "ssl-error" });
            expect(activeTab(r.state).loading).toBe(false);
            expect(activeTab(r.state).error).toBe("ssl-error");
            expect(r.events).toEqual([
                { type: "load-failed", reason: "ssl-error" },
            ]);
        });

        it("supersedes a prior error with the new reason", () => {
            let s = bootOneTab();
            s = update(s, { type: "LoadFailed", reason: "first" }).state;
            const r = update(s, { type: "LoadFailed", reason: "second" });
            expect(activeTab(r.state).error).toBe("second");
        });
    });

    describe("HistoryUpdated (active-tab)", () => {
        it("co-updates both flags in one transition", () => {
            const s0 = bootOneTab();
            const r = update(s0, {
                type: "HistoryUpdated",
                canGoBack: true,
                canGoForward: true,
            });
            expect(activeTab(r.state).canGoBack).toBe(true);
            expect(activeTab(r.state).canGoForward).toBe(true);
            expect(r.events).toEqual([
                {
                    type: "history-updated",
                    canGoBack: true,
                    canGoForward: true,
                },
            ]);
        });

        it("leaves an omitted field alone", () => {
            let s = bootOneTab();
            s = update(s, {
                type: "HistoryUpdated",
                canGoBack: true,
                canGoForward: true,
            }).state;
            const r = update(s, {
                type: "HistoryUpdated",
                canGoForward: false,
            });
            expect(activeTab(r.state).canGoBack).toBe(true);
            expect(activeTab(r.state).canGoForward).toBe(false);
            expect(r.events).toEqual([
                {
                    type: "history-updated",
                    canGoBack: true,
                    canGoForward: false,
                },
            ]);
        });

        it("is idempotent when both fields match current state", () => {
            let s = bootOneTab();
            s = update(s, {
                type: "HistoryUpdated",
                canGoBack: true,
                canGoForward: false,
            }).state;
            const r = update(s, {
                type: "HistoryUpdated",
                canGoBack: true,
                canGoForward: false,
            });
            expect(r.state).toBe(s);
            expect(r.events).toEqual([]);
        });
    });

    describe("TitleChanged (active-tab)", () => {
        it("initial tab has the fallback title (for empty-url open)", () => {
            const s0 = bootOneTab("");
            expect(activeTab(s0).title).toBe(TITLE_FALLBACK);
        });

        it("sets a non-empty title verbatim on the active tab", () => {
            const s0 = bootOneTab();
            const r = update(s0, {
                type: "TitleChanged",
                title: "Example Domain",
            });
            expect(activeTab(r.state).title).toBe("Example Domain");
            expect(r.events).toEqual([
                { type: "title-changed", title: "Example Domain" },
            ]);
        });

        it("folds empty title to the fallback", () => {
            let s = bootOneTab();
            s = update(s, { type: "TitleChanged", title: "Example" }).state;
            const r = update(s, { type: "TitleChanged", title: "" });
            expect(activeTab(r.state).title).toBe(TITLE_FALLBACK);
        });

        it("is idempotent on identical post-fold values", () => {
            let s = bootOneTab();
            s = update(s, { type: "TitleChanged", title: "Same" }).state;
            const r = update(s, { type: "TitleChanged", title: "Same" });
            expect(r.state).toBe(s);
            expect(r.events).toEqual([]);
        });
    });

    describe("PaneClicked", () => {
        it("emits pane-clicked event without changing state", () => {
            const s0 = bootOneTab("https://x");
            const r = update(s0, { type: "PaneClicked" });
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([{ type: "pane-clicked" }]);
        });
    });

    describe("Disposed", () => {
        it("flips closed=true and emits disposed event once", () => {
            const r = update(initialState(), { type: "Disposed" });
            expect(r.state.closed).toBe(true);
            expect(r.events).toEqual([{ type: "disposed" }]);
        });
        it("is idempotent — second Disposed is a no-op", () => {
            const s0 = update(initialState(), { type: "Disposed" }).state;
            const r = update(s0, { type: "Disposed" });
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([]);
        });
    });

    describe("post-close gating", () => {
        const closed = () =>
            update(bootOneTab(), { type: "Disposed" }).state;

        it("Navigate after dispose is dropped (state unchanged)", () => {
            const s0 = closed();
            const r = update(s0, { type: "Navigate", url: "https://late" });
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([
                { type: "post-close-command-dropped", commandType: "Navigate" },
            ]);
        });
        it("OpenTab after dispose is dropped", () => {
            const s0 = closed();
            const r = update(s0, { type: "OpenTab", url: "https://late" });
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([
                { type: "post-close-command-dropped", commandType: "OpenTab" },
            ]);
        });
        it("CloseTab after dispose is dropped", () => {
            const s0 = closed();
            const r = update(s0, { type: "CloseTab", tabId: "x" });
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([
                { type: "post-close-command-dropped", commandType: "CloseTab" },
            ]);
        });
        it("PaneClicked after dispose is dropped", () => {
            const s0 = closed();
            const r = update(s0, { type: "PaneClicked" });
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([
                { type: "post-close-command-dropped", commandType: "PaneClicked" },
            ]);
        });
    });

    describe("UrlConfirmed cross-origin handling (per-tab)", () => {
        const setupRealFavicon = (origin: string): BrowserPaneState => {
            let s = bootOneTab();
            s = update(s, { type: "Navigate", url: `${origin}/` }).state;
            return update(s, {
                type: "FaviconUrlsReceived",
                urls: [`${origin}/real.ico`],
            }).state;
        };

        it("cross-origin URL change resets favicon override + derives fresh favicon", () => {
            const s0 = setupRealFavicon("https://wikipedia.org");
            expect(activeTab(s0).faviconUrl).toBe("https://wikipedia.org/real.ico");
            expect(activeTab(s0).faviconOverridden).toBe(true);
            const r = update(s0, {
                type: "UrlConfirmed",
                url: "https://google.com/",
            });
            expect(activeTab(r.state).faviconUrl).toBe(
                "https://google.com/favicon.ico",
            );
            expect(activeTab(r.state).faviconOverridden).toBe(false);
        });

        it("same-origin URL change preserves favicon override", () => {
            const s0 = setupRealFavicon("https://wikipedia.org");
            const r = update(s0, {
                type: "UrlConfirmed",
                url: "https://wikipedia.org/wiki/Foo",
            });
            expect(activeTab(r.state).faviconUrl).toBe(
                "https://wikipedia.org/real.ico",
            );
            expect(activeTab(r.state).faviconOverridden).toBe(true);
        });

        it("cross-origin resets title to hostname placeholder", () => {
            let s = bootOneTab();
            s = update(s, {
                type: "TitleChanged",
                title: "Wikipedia, the free encyclopedia",
            }).state;
            // Manually set active tab url to wikipedia for the test
            // setup (a real navigation would clear titleOverridden).
            s = update(s, {
                type: "UrlConfirmed",
                url: "https://wikipedia.org/",
            }).state;
            // titleOverridden should still be true (UrlConfirmed
            // preserves real title same-origin; from initial empty
            // URL the "same-origin" path goes to wikipedia.org. The
            // active tab's url was "" before this; sameOriginUrl("",
            // url) === false, so titleOverridden resets… wait the
            // test setup needs care. Re-do the setup deliberately:
            // start with wikipedia loaded + real title, then
            // UrlConfirm to google.
            //
            // Easier: bootOneTab("https://wikipedia.org/") + title
            // change, then confirm to google.
            const s2 = update(
                update(bootOneTab("https://wikipedia.org/"), {
                    type: "TitleChanged",
                    title: "Wikipedia, the free encyclopedia",
                }).state,
                { type: "UrlConfirmed", url: "https://google.com/" },
            );
            expect(activeTab(s2.state).title).toBe("google.com");
            expect(activeTab(s2.state).titleOverridden).toBe(false);
        });

        it("same-origin preserves real title (no flash)", () => {
            let s = bootOneTab("https://wikipedia.org/wiki/Cat");
            s = update(s, {
                type: "TitleChanged",
                title: "Wikipedia, the free encyclopedia",
            }).state;
            const r = update(s, {
                type: "UrlConfirmed",
                url: "https://wikipedia.org/wiki/Dog",
            });
            expect(activeTab(r.state).title).toBe(
                "Wikipedia, the free encyclopedia",
            );
            expect(activeTab(r.state).titleOverridden).toBe(true);
        });
    });

    describe("TitleChanged sets titleOverridden flag", () => {
        it("non-empty title sets titleOverridden=true", () => {
            const s0 = bootOneTab();
            const r = update(s0, { type: "TitleChanged", title: "Hello" });
            expect(activeTab(r.state).titleOverridden).toBe(true);
        });

        it("empty/whitespace folds to TITLE_FALLBACK with titleOverridden=false", () => {
            let s = bootOneTab();
            s = update(s, { type: "TitleChanged", title: "Hello" }).state;
            const r = update(s, { type: "TitleChanged", title: "  " });
            expect(activeTab(r.state).title).toBe(TITLE_FALLBACK);
            expect(activeTab(r.state).titleOverridden).toBe(false);
        });

        it("Navigate clears titleOverridden", () => {
            let s = bootOneTab();
            s = update(s, { type: "TitleChanged", title: "Hello" }).state;
            const r = update(s, {
                type: "Navigate",
                url: "https://example.com/",
            });
            expect(activeTab(r.state).titleOverridden).toBe(false);
            expect(activeTab(r.state).title).toBe("example.com");
        });
    });

    // ─────────────────────────────────────────────────────────────
    // Phase 1A — tab-list management
    // ─────────────────────────────────────────────────────────────
    describe("OpenTab", () => {
        it("appends a tab and activates it on an empty pane", () => {
            const s0 = initialState();
            const r = update(s0, { type: "OpenTab", url: "https://a.com" });
            expect(r.state.tabs.length).toBe(1);
            expect(r.state.tabs[0].url).toBe("https://a.com");
            expect(r.state.activeTabId).toBe(r.state.tabs[0].id);
            const ids = new Set(r.events.map((e) => e.type));
            expect(ids.has("tab-opened")).toBe(true);
            expect(ids.has("tab-activated")).toBe(true);
        });

        it("second OpenTab appends + activates, deactivates the prior tab", () => {
            let s = update(initialState(), {
                type: "OpenTab",
                url: "https://a.com",
            }).state;
            const firstId = s.activeTabId;
            s = update(s, { type: "OpenTab", url: "https://b.com" }).state;
            expect(s.tabs.length).toBe(2);
            expect(s.activeTabId).toBe(s.tabs[1].id);
            expect(s.activeTabId).not.toBe(firstId);
        });

        it("background mode appends without activating", () => {
            let s = update(initialState(), {
                type: "OpenTab",
                url: "https://a.com",
            }).state;
            const firstId = s.activeTabId;
            const r = update(s, {
                type: "OpenTab",
                url: "https://b.com",
                mode: "background",
            });
            expect(r.state.tabs.length).toBe(2);
            // Active stays on the first tab.
            expect(r.state.activeTabId).toBe(firstId);
            // tab-activated NOT emitted for a background open.
            const evTypes = r.events.map((e) => e.type);
            expect(evTypes).toContain("tab-opened");
            expect(evTypes).not.toContain("tab-activated");
        });

        it("background mode on empty pane still activates (no other choice)", () => {
            const s0 = initialState();
            const r = update(s0, {
                type: "OpenTab",
                url: "https://a.com",
                mode: "background",
            });
            expect(r.state.activeTabId).toBe(r.state.tabs[0].id);
            const evTypes = r.events.map((e) => e.type);
            expect(evTypes).toContain("tab-activated");
        });
    });

    describe("CloseTab", () => {
        it("drops the tab and activates the right neighbour when closing the active leftmost", () => {
            let s = initialState();
            s = update(s, { type: "OpenTab", url: "https://a" }).state;
            s = update(s, { type: "OpenTab", url: "https://b" }).state;
            s = update(s, { type: "OpenTab", url: "https://c" }).state;
            const ids = s.tabs.map((t) => t.id);
            // Active is the last opened (c). Switch to a.
            s = update(s, { type: "SwitchTab", tabId: ids[0] }).state;
            expect(s.activeTabId).toBe(ids[0]);
            const r = update(s, { type: "CloseTab", tabId: ids[0] });
            expect(r.state.tabs.length).toBe(2);
            expect(r.state.activeTabId).toBe(ids[1]); // right neighbour
            const evTypes = r.events.map((e) => e.type);
            expect(evTypes).toEqual(["tab-closed", "tab-activated"]);
        });

        it("activates the left neighbour when closing the active rightmost", () => {
            let s = initialState();
            s = update(s, { type: "OpenTab", url: "https://a" }).state;
            s = update(s, { type: "OpenTab", url: "https://b" }).state;
            const ids = s.tabs.map((t) => t.id);
            // Active is b (last opened). Close b.
            const r = update(s, { type: "CloseTab", tabId: ids[1] });
            expect(r.state.tabs.length).toBe(1);
            expect(r.state.activeTabId).toBe(ids[0]);
        });

        it("closing an inactive tab leaves activeTabId untouched", () => {
            let s = initialState();
            s = update(s, { type: "OpenTab", url: "https://a" }).state;
            s = update(s, { type: "OpenTab", url: "https://b" }).state;
            const ids = s.tabs.map((t) => t.id);
            const activeBefore = s.activeTabId;
            const r = update(s, { type: "CloseTab", tabId: ids[0] });
            expect(r.state.activeTabId).toBe(activeBefore);
            const evTypes = r.events.map((e) => e.type);
            expect(evTypes).toEqual(["tab-closed"]);
        });

        it("closing the only tab emits last-tab-closed, empties the pane", () => {
            let s = initialState();
            s = update(s, { type: "OpenTab", url: "https://a" }).state;
            const id = s.activeTabId!;
            const r = update(s, { type: "CloseTab", tabId: id });
            expect(r.state.tabs.length).toBe(0);
            expect(r.state.activeTabId).toBeNull();
            const evTypes = r.events.map((e) => e.type);
            expect(evTypes).toContain("tab-closed");
            expect(evTypes).toContain("last-tab-closed");
        });

        it("pushes a ClosedBrowserTab entry onto recentlyClosed", () => {
            let s = initialState();
            s = update(s, { type: "OpenTab", url: "https://a" }).state;
            const id = s.activeTabId!;
            const r = update(s, { type: "CloseTab", tabId: id });
            expect(r.state.recentlyClosed.length).toBe(1);
            expect(r.state.recentlyClosed[0].url).toBe("https://a");
        });

        it("closing an unknown tabId is a no-op", () => {
            let s = initialState();
            s = update(s, { type: "OpenTab", url: "https://a" }).state;
            const before = s;
            const r = update(s, { type: "CloseTab", tabId: "nope" });
            expect(r.state).toBe(before);
            expect(r.events).toEqual([]);
        });
    });

    describe("SwitchTab", () => {
        it("activates an existing tab", () => {
            let s = initialState();
            s = update(s, { type: "OpenTab", url: "https://a" }).state;
            s = update(s, { type: "OpenTab", url: "https://b" }).state;
            const firstId = s.tabs[0].id;
            const r = update(s, { type: "SwitchTab", tabId: firstId });
            expect(r.state.activeTabId).toBe(firstId);
            expect(r.events).toEqual([
                { type: "tab-activated", tabId: firstId },
            ]);
        });

        it("is a no-op on an unknown tabId", () => {
            let s = initialState();
            s = update(s, { type: "OpenTab", url: "https://a" }).state;
            const before = s;
            const r = update(s, { type: "SwitchTab", tabId: "nope" });
            expect(r.state).toBe(before);
            expect(r.events).toEqual([]);
        });

        it("is a no-op when the requested tab is already active", () => {
            let s = initialState();
            s = update(s, { type: "OpenTab", url: "https://a" }).state;
            const id = s.activeTabId!;
            const r = update(s, { type: "SwitchTab", tabId: id });
            expect(r.state).toBe(s);
            expect(r.events).toEqual([]);
        });
    });

    describe("ReorderTab", () => {
        it("moves a tab to the target index", () => {
            let s = initialState();
            s = update(s, { type: "OpenTab", url: "https://a" }).state;
            s = update(s, { type: "OpenTab", url: "https://b" }).state;
            s = update(s, { type: "OpenTab", url: "https://c" }).state;
            const ids = s.tabs.map((t) => t.id);
            const r = update(s, {
                type: "ReorderTab",
                tabId: ids[0],
                toIndex: 2,
            });
            expect(r.state.tabs.map((t) => t.id)).toEqual([
                ids[1],
                ids[2],
                ids[0],
            ]);
        });

        it("clamps index to [0, tabs.length - 1]", () => {
            let s = initialState();
            s = update(s, { type: "OpenTab", url: "https://a" }).state;
            s = update(s, { type: "OpenTab", url: "https://b" }).state;
            const ids = s.tabs.map((t) => t.id);
            const r1 = update(s, {
                type: "ReorderTab",
                tabId: ids[0],
                toIndex: 99,
            });
            expect(r1.state.tabs.map((t) => t.id)).toEqual([ids[1], ids[0]]);
            const r2 = update(s, {
                type: "ReorderTab",
                tabId: ids[1],
                toIndex: -7,
            });
            expect(r2.state.tabs.map((t) => t.id)).toEqual([ids[1], ids[0]]);
        });

        it("is a no-op when only one tab exists", () => {
            let s = initialState();
            s = update(s, { type: "OpenTab", url: "https://a" }).state;
            const id = s.activeTabId!;
            const r = update(s, { type: "ReorderTab", tabId: id, toIndex: 5 });
            expect(r.state).toBe(s);
        });
    });

    describe("Tab*-prefixed backend events", () => {
        function setupTwoTabs() {
            let s = initialState();
            s = update(s, { type: "OpenTab", url: "https://a" }).state;
            s = update(s, { type: "OpenTab", url: "https://b" }).state;
            return { state: s, idA: s.tabs[0].id, idB: s.tabs[1].id };
        }

        it("TabUrlChanged updates ONLY the matching tab", () => {
            const { state, idA, idB } = setupTwoTabs();
            const r = update(state, {
                type: "TabUrlChanged",
                tabId: idA,
                url: "https://new-a",
                source: "backend",
            });
            const tA = r.state.tabs.find((t) => t.id === idA)!;
            const tB = r.state.tabs.find((t) => t.id === idB)!;
            expect(tA.url).toBe("https://new-a");
            expect(tB.url).toBe("https://b");
        });

        it("TabTitleChanged updates ONLY the matching tab", () => {
            const { state, idA, idB } = setupTwoTabs();
            const r = update(state, {
                type: "TabTitleChanged",
                tabId: idA,
                title: "A Page",
            });
            expect(r.state.tabs.find((t) => t.id === idA)!.title).toBe("A Page");
            expect(r.state.tabs.find((t) => t.id === idB)!.title).not.toBe(
                "A Page",
            );
        });

        it("TabFaviconChanged updates ONLY the matching tab", () => {
            const { state, idA, idB } = setupTwoTabs();
            const r = update(state, {
                type: "TabFaviconChanged",
                tabId: idA,
                faviconUrl: "https://a/real.ico",
            });
            expect(r.state.tabs.find((t) => t.id === idA)!.faviconUrl).toBe(
                "https://a/real.ico",
            );
            expect(r.state.tabs.find((t) => t.id === idA)!.faviconOverridden).toBe(
                true,
            );
            expect(r.state.tabs.find((t) => t.id === idB)!.faviconUrl).not.toBe(
                "https://a/real.ico",
            );
        });

        it("TabLoadingChanged updates loading + canGoBack + canGoForward atomically", () => {
            const { state, idA } = setupTwoTabs();
            const r = update(state, {
                type: "TabLoadingChanged",
                tabId: idA,
                loading: true,
                canGoBack: true,
                canGoForward: true,
            });
            const t = r.state.tabs.find((tt) => tt.id === idA)!;
            expect(t.loading).toBe(true);
            expect(t.canGoBack).toBe(true);
            expect(t.canGoForward).toBe(true);
            expect(r.events).toEqual([
                {
                    type: "tab-loading-changed",
                    tabId: idA,
                    loading: true,
                    canGoBack: true,
                    canGoForward: true,
                },
            ]);
        });

        it("TabLoadFailed sets error and clears loading", () => {
            const { state, idA } = setupTwoTabs();
            const r = update(state, {
                type: "TabLoadFailed",
                tabId: idA,
                error: "ssl-error",
            });
            const t = r.state.tabs.find((tt) => tt.id === idA)!;
            expect(t.loading).toBe(false);
            expect(t.error).toBe("ssl-error");
        });

        it("TabBackendCreated flips the flag exactly once", () => {
            const { state, idA } = setupTwoTabs();
            const r1 = update(state, {
                type: "TabBackendCreated",
                tabId: idA,
            });
            expect(r1.state.tabs.find((t) => t.id === idA)!.backendCreated).toBe(
                true,
            );
            const r2 = update(r1.state, {
                type: "TabBackendCreated",
                tabId: idA,
            });
            // Already true — idempotent no-op.
            expect(r2.state).toBe(r1.state);
            expect(r2.events).toEqual([]);
        });

        it("Tab* commands with unknown tabId are no-ops", () => {
            const { state } = setupTwoTabs();
            for (const cmd of [
                { type: "TabUrlChanged" as const, tabId: "nope", url: "x" },
                { type: "TabTitleChanged" as const, tabId: "nope", title: "x" },
                { type: "TabFaviconChanged" as const, tabId: "nope", faviconUrl: "x" },
                {
                    type: "TabLoadingChanged" as const,
                    tabId: "nope",
                    loading: true,
                    canGoBack: false,
                    canGoForward: false,
                },
                { type: "TabLoadFailed" as const, tabId: "nope", error: "e" },
                { type: "TabBackendCreated" as const, tabId: "nope" },
            ]) {
                const r = update(state, cmd);
                expect(r.state).toBe(state);
                expect(r.events).toEqual([]);
            }
        });
    });

    describe("recentlyClosed stack", () => {
        it("is capped at MAX_RECENTLY_CLOSED (10), oldest evicted", () => {
            let s = initialState();
            // Open + close 12 tabs.
            for (let i = 0; i < 12; i++) {
                s = update(s, {
                    type: "OpenTab",
                    url: `https://u${i}`,
                }).state;
                const id = s.activeTabId!;
                s = update(s, { type: "CloseTab", tabId: id }).state;
            }
            expect(s.recentlyClosed.length).toBe(10);
            // Newest entry at the end; oldest (u0, u1) evicted.
            expect(s.recentlyClosed[s.recentlyClosed.length - 1].url).toBe(
                "https://u11",
            );
            expect(s.recentlyClosed[0].url).toBe("https://u2");
        });
    });

    describe("ReopenLastClosed", () => {
        it("is a no-op when the stack is empty", () => {
            const s0 = initialState();
            const r = update(s0, { type: "ReopenLastClosed" });
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([]);
        });

        it("pops the newest entry and opens it as a new tab", () => {
            let s = initialState();
            s = update(s, { type: "OpenTab", url: "https://a" }).state;
            s = update(s, { type: "OpenTab", url: "https://b" }).state;
            // Close b (the active tab — last opened).
            const bId = s.activeTabId!;
            s = update(s, { type: "CloseTab", tabId: bId }).state;
            expect(s.recentlyClosed.length).toBe(1);
            expect(s.recentlyClosed[0].url).toBe("https://b");
            const r = update(s, { type: "ReopenLastClosed" });
            expect(r.state.recentlyClosed.length).toBe(0);
            expect(r.state.tabs.length).toBe(2);
            expect(r.state.tabs[1].url).toBe("https://b");
            expect(r.state.activeTabId).toBe(r.state.tabs[1].id);
        });
    });

    describe("HydrateFromMeta", () => {
        it("bulk-restores tabs; all start with backendCreated:false, loading:false, no error", () => {
            const s0 = initialState();
            const r = update(s0, {
                type: "HydrateFromMeta",
                tabs: [
                    { id: "id-a", url: "https://a" },
                    { id: "id-b", url: "https://b" },
                ],
                activeTabId: "id-b",
            });
            expect(r.state.tabs.length).toBe(2);
            for (const t of r.state.tabs) {
                expect(t.backendCreated).toBe(false);
                expect(t.loading).toBe(false);
                expect(t.error).toBeNull();
            }
            expect(r.state.activeTabId).toBe("id-b");
            const evTypes = r.events.map((e) => e.type);
            expect(evTypes).toContain("tabs-restored");
        });

        it("falls back to the first tab when activeTabId doesn't match", () => {
            const s0 = initialState();
            const r = update(s0, {
                type: "HydrateFromMeta",
                tabs: [
                    { id: "id-a", url: "https://a" },
                    { id: "id-b", url: "https://b" },
                ],
                activeTabId: "missing",
            });
            expect(r.state.activeTabId).toBe("id-a");
        });

        it("yields null activeTabId when the tabs array is empty", () => {
            const s0 = initialState();
            const r = update(s0, {
                type: "HydrateFromMeta",
                tabs: [],
                activeTabId: null,
            });
            expect(r.state.tabs.length).toBe(0);
            expect(r.state.activeTabId).toBeNull();
        });
    });
});
