// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { update } from "./reducer";
import { initialState, TITLE_FALLBACK } from "./types";

describe("browser-pane-state reducer (slice #9, Phases 3a + 3b + 3c + 3e + 3d)", () => {
    describe("Navigate", () => {
        it("sets loading=true and clears any prior error", () => {
            const s0 = update(initialState(), {
                type: "LoadFailed",
                reason: "DNS fail",
            }).state;
            expect(s0.error).toBe("DNS fail");

            const r = update(s0, { type: "Navigate", url: "https://x" });
            expect(r.state.loading).toBe(true);
            expect(r.state.error).toBeNull();
            expect(r.events).toEqual([{ type: "navigate", url: "https://x" }]);
        });

        it("preserves mutual exclusion: never sets loading and error simultaneously", () => {
            const r = update(initialState(), {
                type: "Navigate",
                url: "https://x",
            });
            expect(r.state.loading && r.state.error !== null).toBe(false);
        });

        it("sets state.url atomically with the loading flip", () => {
            const r = update(initialState(), {
                type: "Navigate",
                url: "https://example.com",
            });
            expect(r.state.url).toBe("https://example.com");
            expect(r.state.loading).toBe(true);
        });

        it("supersedes a prior url", () => {
            const s0 = update(initialState(), {
                type: "Navigate",
                url: "https://a.com",
            }).state;
            const r = update(s0, { type: "Navigate", url: "https://b.com" });
            expect(r.state.url).toBe("https://b.com");
        });
    });

    describe("UrlConfirmed", () => {
        it("updates state.url to the host-confirmed value", () => {
            const s0 = update(initialState(), {
                type: "Navigate",
                url: "https://typed-by-user",
            }).state;
            const r = update(s0, {
                type: "UrlConfirmed",
                url: "https://post-redirect-final",
            });
            expect(r.state.url).toBe("https://post-redirect-final");
            expect(r.events).toEqual([
                { type: "url-confirmed", url: "https://post-redirect-final" },
            ]);
        });

        it("does NOT touch loading, error, or history", () => {
            const s0 = update(initialState(), {
                type: "Navigate",
                url: "https://x",
            }).state;
            const s1 = update(s0, {
                type: "HistoryUpdated",
                canGoBack: true,
            }).state;
            const r = update(s1, {
                type: "UrlConfirmed",
                url: "https://final",
            });
            expect(r.state.loading).toBe(s1.loading);
            expect(r.state.error).toBe(s1.error);
            expect(r.state.canGoBack).toBe(s1.canGoBack);
            expect(r.state.canGoForward).toBe(s1.canGoForward);
        });

        it("is idempotent on identical url", () => {
            const s0 = update(initialState(), {
                type: "UrlConfirmed",
                url: "https://x",
            }).state;
            const r = update(s0, { type: "UrlConfirmed", url: "https://x" });
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([]);
        });
    });

    describe("UrlCleared", () => {
        it("clears state.url to empty string", () => {
            const s0 = update(initialState(), {
                type: "Navigate",
                url: "https://x",
            }).state;
            const r = update(s0, { type: "UrlCleared" });
            expect(r.state.url).toBe("");
            expect(r.events).toEqual([{ type: "url-cleared" }]);
        });

        it("does NOT touch loading, error, history, or title", () => {
            const s0 = update(initialState(), {
                type: "Navigate",
                url: "https://x",
            }).state;
            const r = update(s0, { type: "UrlCleared" });
            expect(r.state.loading).toBe(s0.loading);
            expect(r.state.error).toBe(s0.error);
            expect(r.state.canGoBack).toBe(s0.canGoBack);
            expect(r.state.canGoForward).toBe(s0.canGoForward);
            expect(r.state.title).toBe(s0.title);
        });

        it("is idempotent when url is already empty", () => {
            const s0 = initialState();
            expect(s0.url).toBe("");
            const r = update(s0, { type: "UrlCleared" });
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([]);
        });

        it("models reload's force-reload pattern (clear → restore)", () => {
            const s0 = update(initialState(), {
                type: "Navigate",
                url: "https://page",
            }).state;
            const sCleared = update(s0, { type: "UrlCleared" }).state;
            expect(sCleared.url).toBe("");
            const sRestored = update(sCleared, {
                type: "Navigate",
                url: "https://page",
            }).state;
            expect(sRestored.url).toBe("https://page");
            expect(sRestored.loading).toBe(true);
        });
    });

    describe("LoadStarted", () => {
        it("sets loading=true and clears any prior error", () => {
            const s0 = update(initialState(), {
                type: "LoadFailed",
                reason: "ssl-error",
            }).state;
            const r = update(s0, { type: "LoadStarted" });
            expect(r.state.loading).toBe(true);
            expect(r.state.error).toBeNull();
            expect(r.events).toEqual([{ type: "load-started" }]);
        });

        it("preserves mutual exclusion on (loading, error)", () => {
            const r = update(initialState(), { type: "LoadStarted" });
            expect(r.state.loading && r.state.error !== null).toBe(false);
        });
    });

    describe("LoadFinished", () => {
        it("clears loading and error after a navigate", () => {
            const s0 = update(initialState(), {
                type: "Navigate",
                url: "https://x",
            }).state;
            const r = update(s0, { type: "LoadFinished" });
            expect(r.state.loading).toBe(false);
            expect(r.state.error).toBeNull();
            expect(r.events).toEqual([{ type: "load-finished" }]);
        });

        it("clears a stale error even when loading was already false", () => {
            const s0 = update(initialState(), {
                type: "LoadFailed",
                reason: "boom",
            }).state;
            expect(s0.loading).toBe(false);
            expect(s0.error).toBe("boom");

            const r = update(s0, { type: "LoadFinished" });
            expect(r.state.error).toBeNull();
            expect(r.events).toEqual([{ type: "load-finished" }]);
        });

        it("is a no-op on steady-state (no loading, no error)", () => {
            const s0 = initialState();
            const r = update(s0, { type: "LoadFinished" });
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([]);
        });
    });

    describe("LoadFailed", () => {
        it("sets error and clears loading", () => {
            const s0 = update(initialState(), {
                type: "Navigate",
                url: "https://x",
            }).state;
            const r = update(s0, {
                type: "LoadFailed",
                reason: "ssl-error",
            });
            expect(r.state.loading).toBe(false);
            expect(r.state.error).toBe("ssl-error");
            expect(r.events).toEqual([
                { type: "load-failed", reason: "ssl-error" },
            ]);
        });

        it("supersedes a prior error with the new reason", () => {
            const s0 = update(initialState(), {
                type: "LoadFailed",
                reason: "first",
            }).state;
            const r = update(s0, { type: "LoadFailed", reason: "second" });
            expect(r.state.error).toBe("second");
        });
    });

    describe("HistoryUpdated", () => {
        it("co-updates both flags in one transition", () => {
            const r = update(initialState(), {
                type: "HistoryUpdated",
                canGoBack: true,
                canGoForward: true,
            });
            expect(r.state.canGoBack).toBe(true);
            expect(r.state.canGoForward).toBe(true);
            expect(r.events).toEqual([
                {
                    type: "history-updated",
                    canGoBack: true,
                    canGoForward: true,
                },
            ]);
        });

        it("leaves an omitted field alone", () => {
            const s0 = update(initialState(), {
                type: "HistoryUpdated",
                canGoBack: true,
                canGoForward: true,
            }).state;
            const r = update(s0, {
                type: "HistoryUpdated",
                canGoForward: false,
            });
            expect(r.state.canGoBack).toBe(true);
            expect(r.state.canGoForward).toBe(false);
            expect(r.events).toEqual([
                {
                    type: "history-updated",
                    canGoBack: true,
                    canGoForward: false,
                },
            ]);
        });

        it("is idempotent when both fields match current state", () => {
            const s0 = update(initialState(), {
                type: "HistoryUpdated",
                canGoBack: true,
                canGoForward: false,
            }).state;
            const r = update(s0, {
                type: "HistoryUpdated",
                canGoBack: true,
                canGoForward: false,
            });
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([]);
        });

        it("is idempotent when only the supplied field matches", () => {
            const s0 = update(initialState(), {
                type: "HistoryUpdated",
                canGoBack: true,
            }).state;
            const r = update(s0, {
                type: "HistoryUpdated",
                canGoBack: true,
            });
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([]);
        });

        it("does not interact with loading or error cells", () => {
            const s0 = update(initialState(), {
                type: "Navigate",
                url: "https://x",
            }).state;
            const r = update(s0, {
                type: "HistoryUpdated",
                canGoBack: true,
                canGoForward: true,
            });
            expect(r.state.loading).toBe(true);
            expect(r.state.error).toBeNull();
        });
    });

    describe("TitleChanged", () => {
        it("initial state has the fallback title", () => {
            expect(initialState().title).toBe(TITLE_FALLBACK);
        });

        it("sets a non-empty title verbatim", () => {
            const r = update(initialState(), {
                type: "TitleChanged",
                title: "Example Domain",
            });
            expect(r.state.title).toBe("Example Domain");
            expect(r.events).toEqual([
                { type: "title-changed", title: "Example Domain" },
            ]);
        });

        it("folds empty title to the fallback", () => {
            const s0 = update(initialState(), {
                type: "TitleChanged",
                title: "Example",
            }).state;
            const r = update(s0, { type: "TitleChanged", title: "" });
            expect(r.state.title).toBe(TITLE_FALLBACK);
            expect(r.events).toEqual([
                { type: "title-changed", title: TITLE_FALLBACK },
            ]);
        });

        it("folds whitespace-only title to the fallback", () => {
            const s0 = initialState();
            const r = update(s0, {
                type: "TitleChanged",
                title: "   \t\n",
            });
            // Already at fallback so no event fires (idempotent post-fold).
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([]);
            // Force a non-fallback state then fold back.
            const s1 = update(initialState(), {
                type: "TitleChanged",
                title: "Real",
            }).state;
            const r2 = update(s1, { type: "TitleChanged", title: "  " });
            expect(r2.state.title).toBe(TITLE_FALLBACK);
        });

        it("is idempotent on identical post-fold values", () => {
            const s0 = update(initialState(), {
                type: "TitleChanged",
                title: "Same",
            }).state;
            const r = update(s0, { type: "TitleChanged", title: "Same" });
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([]);
        });

        it("does not interact with loading, error, or history cells", () => {
            const s0 = update(initialState(), {
                type: "Navigate",
                url: "https://x",
            }).state;
            const s1 = update(s0, {
                type: "HistoryUpdated",
                canGoBack: true,
            }).state;
            const r = update(s1, {
                type: "TitleChanged",
                title: "Page",
            });
            expect(r.state.loading).toBe(s1.loading);
            expect(r.state.error).toBe(s1.error);
            expect(r.state.canGoBack).toBe(s1.canGoBack);
            expect(r.state.canGoForward).toBe(s1.canGoForward);
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
            update(initialState(), { type: "Disposed" }).state;

        it("Navigate after dispose is dropped (state unchanged)", () => {
            const s0 = closed();
            const r = update(s0, { type: "Navigate", url: "https://late" });
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([
                { type: "post-close-command-dropped", commandType: "Navigate" },
            ]);
        });

        it("LoadFinished after dispose is dropped", () => {
            const s0 = closed();
            const r = update(s0, { type: "LoadFinished" });
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([
                {
                    type: "post-close-command-dropped",
                    commandType: "LoadFinished",
                },
            ]);
        });

        it("LoadFailed after dispose is dropped", () => {
            const s0 = closed();
            const r = update(s0, { type: "LoadFailed", reason: "late-fail" });
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([
                {
                    type: "post-close-command-dropped",
                    commandType: "LoadFailed",
                },
            ]);
        });

        it("HistoryUpdated after dispose is dropped", () => {
            const s0 = closed();
            const r = update(s0, {
                type: "HistoryUpdated",
                canGoBack: true,
                canGoForward: true,
            });
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([
                {
                    type: "post-close-command-dropped",
                    commandType: "HistoryUpdated",
                },
            ]);
        });

        it("TitleChanged after dispose is dropped", () => {
            const s0 = closed();
            const r = update(s0, {
                type: "TitleChanged",
                title: "Late title",
            });
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([
                {
                    type: "post-close-command-dropped",
                    commandType: "TitleChanged",
                },
            ]);
        });

        it("UrlConfirmed after dispose is dropped", () => {
            const s0 = closed();
            const r = update(s0, {
                type: "UrlConfirmed",
                url: "https://late",
            });
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([
                {
                    type: "post-close-command-dropped",
                    commandType: "UrlConfirmed",
                },
            ]);
        });

        it("UrlCleared after dispose is dropped", () => {
            const s0 = closed();
            const r = update(s0, { type: "UrlCleared" });
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([
                {
                    type: "post-close-command-dropped",
                    commandType: "UrlCleared",
                },
            ]);
        });
    });

    describe("invariants across sequences", () => {
        it("Navigate → LoadFinished → Navigate → LoadFailed → Disposed", () => {
            let s = initialState();
            s = update(s, { type: "Navigate", url: "a" }).state;
            expect(s).toMatchObject({ loading: true, error: null, closed: false });
            s = update(s, { type: "LoadFinished" }).state;
            expect(s).toMatchObject({ loading: false, error: null, closed: false });
            s = update(s, { type: "Navigate", url: "b" }).state;
            expect(s).toMatchObject({ loading: true, error: null, closed: false });
            s = update(s, { type: "LoadFailed", reason: "x" }).state;
            expect(s).toMatchObject({ loading: false, error: "x", closed: false });
            s = update(s, { type: "Disposed" }).state;
            expect(s).toMatchObject({ loading: false, error: "x", closed: true });
        });

        it("loading and error are never both truthy across all single-step transitions from every reachable state", () => {
            const starts: Array<() => any> = [
                () => initialState(),
                () => update(initialState(), { type: "Navigate", url: "u" }).state,
                () =>
                    update(
                        update(initialState(), { type: "Navigate", url: "u" }).state,
                        { type: "LoadFinished" },
                    ).state,
                () =>
                    update(initialState(), { type: "LoadFailed", reason: "e" })
                        .state,
            ];
            const cmds: any[] = [
                { type: "Navigate", url: "u2" },
                { type: "LoadStarted" },
                { type: "LoadFinished" },
                { type: "LoadFailed", reason: "e2" },
                { type: "HistoryUpdated", canGoBack: true, canGoForward: true },
                { type: "HistoryUpdated", canGoBack: false },
                { type: "TitleChanged", title: "Page" },
                { type: "TitleChanged", title: "" },
                { type: "UrlConfirmed", url: "https://final" },
                { type: "UrlCleared" },
                { type: "Disposed" },
            ];
            for (const mk of starts) {
                for (const c of cmds) {
                    const r = update(mk(), c);
                    expect(r.state.loading && r.state.error !== null).toBe(false);
                }
            }
        });
    });
});
