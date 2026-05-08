// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { update } from "./reducer";
import { initialState } from "./types";

describe("browser-pane-state reducer (slice #9, Phase 3a + 3b)", () => {
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
