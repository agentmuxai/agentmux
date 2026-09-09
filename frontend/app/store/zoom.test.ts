// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for the all-panes zoom stepper (Ctrl+Shift+Scroll — see
 * docs/specs/SPEC_CTRL_SHIFT_SCROLL_ZOOM_ALL_PANES_2026_09_07.md). Existing
 * single-pane zoom (zoomBlockIn/Out, chromeZoomIn/Out) is exercised
 * end-to-end already via armory-view.test.tsx / warden-view.test.tsx; this
 * file covers zoomAllPanesIn/Out specifically — the actual new logic this
 * feature adds, independent of any one view's rendering.
 */

import { createEffect, createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// blockMetas/blockViewTypes are the shared source of truth BOTH mocks below
// read from — mirrors warden-view.test.tsx's real-signal-backed SetMetaCommand
// mock, extended to multiple blocks instead of one.
let blockMetas: Map<string, Record<string, unknown>>;
let blockViewTypes: Map<string, string>;

vi.mock("@/app/store/global", () => ({
    getBlockComponentModel: (blockId: string) => {
        const vt = blockViewTypes.get(blockId);
        return vt === undefined ? undefined : { viewModel: { viewType: vt } };
    },
    getFocusedBlockId: () => undefined,
    WOS: {
        makeORef: (type: string, id: string) => `${type}:${id}`,
        getObjectValue: (oref: string) => {
            const id = oref.slice(oref.indexOf(":") + 1);
            return { meta: blockMetas.get(id) ?? {} };
        },
    },
}));

vi.mock("@/app/store/block-component-registry", () => ({
    getAllBlockComponentModelEntries: () =>
        Array.from(blockViewTypes.entries()).map(([id, vt]) => [id, { viewModel: { viewType: vt } }]),
}));

// Named so a test that overrides the mock's implementation (to reproduce
// the real async round-trip — see the "computes the summary range..." test
// below) can be reset back to this in beforeEach, rather than needing to
// manually restore it and risk leaking a no-op implementation into every
// test that runs after it if the override test fails before restoring.
function defaultSetMetaImpl(..._args: unknown[]): Promise<undefined> {
    const opts = _args[1] as { oref: string; meta: Record<string, unknown> };
    const id = opts.oref.slice(opts.oref.indexOf(":") + 1);
    const prev = blockMetas.get(id) ?? {};
    blockMetas.set(id, { ...prev, ...opts.meta });
    return Promise.resolve(undefined);
}
const setMetaMock = vi.fn(defaultSetMetaImpl);
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { SetMetaCommand: (...args: unknown[]) => setMetaMock(...args) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { zoomAllPanesIn, zoomAllPanesOut, zoomBlockIn, zoomIndicatorTextAtom } from "./zoom";

// term:fontsize is pinned to 20 for every test block so effective font size
// (round(base * zoom)) changes by exactly 1px per 0.05 WHEEL_STEP, with no
// skip-to-next-pixel micro-stepping (zoom.ts's stepZoom nudges by an extra
// MICRO_STEP whenever a step would otherwise round to the SAME pixel size —
// real, deliberate behavior for a font size like the default 15px, but it
// would make this file's expected zoom values depend on that internal
// rounding coincidence rather than on what this feature actually adds).
function setBlock(id: string, viewType: string, zoom?: number): void {
    blockViewTypes.set(id, viewType);
    const meta: Record<string, unknown> = { "term:fontsize": 20 };
    if (zoom !== undefined) meta["term:zoom"] = zoom;
    blockMetas.set(id, meta);
}

function currentZoom(id: string): number {
    return (blockMetas.get(id)?.["term:zoom"] as number | undefined) ?? 1.0;
}

beforeEach(() => {
    blockMetas = new Map();
    blockViewTypes = new Map();
    setMetaMock.mockClear();
    setMetaMock.mockImplementation(defaultSetMetaImpl);
});

afterEach(() => {
    vi.clearAllMocks();
});

describe("zoomAllPanesIn/Out", () => {
    it("steps every zoomable pane in the registry by one WHEEL_STEP", () => {
        setBlock("t1", "term");
        setBlock("a1", "agent");
        setBlock("e1", "editor");

        zoomAllPanesIn();

        expect(currentZoom("t1")).toBeCloseTo(1.05, 5);
        expect(currentZoom("a1")).toBeCloseTo(1.05, 5);
        expect(currentZoom("e1")).toBeCloseTo(1.05, 5);
    });

    it("skips pane types getBlockZoom doesn't recognize (browser) with zero new filtering", () => {
        setBlock("t1", "term");
        setBlock("b1", "browser");

        zoomAllPanesIn();

        expect(currentZoom("t1")).toBeCloseTo(1.05, 5);
        // No SetMetaCommand call should have targeted this block at all —
        // proves getBlockZoom's own viewType guard is doing the exclusion,
        // not some spec-specific filter that could drift from it.
        expect(setMetaMock).not.toHaveBeenCalledWith(undefined, expect.objectContaining({ oref: "block:b1" }));
    });

    // Codex review, PR #3090: warden-view.tsx reads/writes the identical
    // "term:zoom" meta key and applies it as CSS zoom exactly like
    // armory/swarm (both already included) — leaving it out of
    // getBlockZoom's allowlist was an oversight, not a deliberate
    // exclusion (unlike browser, which is excluded on purpose — see the
    // spec's Non-goals). A warden pane must be part of the batch.
    it("includes warden panes in the batch — they already speak term:zoom identically to armory/swarm", () => {
        setBlock("t1", "term");
        setBlock("w1", "warden");

        zoomAllPanesIn();

        expect(currentZoom("t1")).toBeCloseTo(1.05, 5);
        expect(currentZoom("w1")).toBeCloseTo(1.05, 5);
    });

    it("steps each pane relative to its OWN current zoom, not a shared baseline", () => {
        setBlock("low", "term", 0.8);
        setBlock("high", "term", 1.5);

        zoomAllPanesIn();

        // Both moved up by one step from THEIR OWN prior value — they do not
        // converge to a common new value.
        expect(currentZoom("low")).toBeCloseTo(0.85, 5);
        expect(currentZoom("high")).toBeCloseTo(1.55, 5);
        expect(currentZoom("low")).not.toBeCloseTo(currentZoom("high"), 2);
    });

    it("zoomAllPanesOut steps every zoomable pane down by one WHEEL_STEP", () => {
        setBlock("t1", "term", 1.0);
        setBlock("a1", "agent", 1.0);

        zoomAllPanesOut();

        expect(currentZoom("t1")).toBeCloseTo(0.95, 5);
        expect(currentZoom("a1")).toBeCloseTo(0.95, 5);
    });

    it("does nothing (no indicator, no RPC calls) when the window has no zoomable pane", () => {
        setBlock("b1", "browser");
        setBlock("s1", "sysinfo");

        zoomAllPanesIn();

        expect(setMetaMock).not.toHaveBeenCalled();
    });

    it("clamps at MAX_ZOOM the same way single-pane zoom does", () => {
        setBlock("t1", "term", 1.98);
        zoomAllPanesIn();
        expect(currentZoom("t1")).toBeLessThanOrEqual(2.0);
    });

    // ReAgent P1, PR #3090: the real RpcApi.SetMetaCommand is fire-and-forget
    // — WOS's local object cache is NOT updated synchronously by it, only
    // later when the backend pushes a WaveObjUpdate event back
    // (global.ts's initGlobalEventSubs). Every OTHER test in this file uses
    // a setMetaMock that updates blockMetas synchronously, which masked a
    // real bug: stepAllPanes originally re-read getBlockZoom(blockId) right
    // after stepZoom() to compute the summary range, which — against the
    // real system's actual timing — would have read the STALE, pre-step
    // value every single time. This test's mock deliberately does NOT
    // update blockMetas at all, reproducing that stale-cache condition
    // exactly, to prove the fix (using stepZoom's own return value instead
    // of re-reading) doesn't depend on the cache having updated.
    it("computes the summary range from the freshly stepped value, not a re-read of WOS's (unsynced) cache", () => {
        // Deliberately never touches blockMetas — reproduces the real
        // system's actual timing (RpcApi.SetMetaCommand is fire-and-forget;
        // WOS's cache only updates later, off a WaveObjUpdate event).
        // beforeEach resets this back to defaultSetMetaImpl for every other
        // test, so this override cannot leak.
        setMetaMock.mockImplementation(() => Promise.resolve(undefined));

        setBlock("t1", "term", 1.0);
        setBlock("a1", "agent", 1.0);

        zoomAllPanesIn();

        // blockMetas was never updated by the (deliberately inert) mock —
        // getBlockZoom would still read the OLD 1.0 for both blocks here.
        // The indicator must still reflect the NEW value regardless.
        expect(currentZoom("t1")).toBe(1.0);
        expect(zoomIndicatorTextAtom()).toBe("All panes: 105%");
    });

    describe("indicator", () => {
        it("shows one summary toast naming a shared percentage when every pane lands on the same value", () => {
            setBlock("t1", "term", 1.0);
            setBlock("a1", "agent", 1.0);

            zoomAllPanesIn();

            expect(zoomIndicatorTextAtom()).toBe("All panes: 105%");
        });

        it("shows a range when panes land on different values", () => {
            setBlock("low", "term", 0.8);
            setBlock("high", "term", 1.5);

            zoomAllPanesIn();

            expect(zoomIndicatorTextAtom()).toBe("All panes: 85%–155%");
        });

        // The bug this suppression fixes: N panes stepping in one gesture
        // used to fire N per-pane toasts, each overwriting the last before
        // it could be read, and settling on whichever pane iterated last —
        // not a useful summary of a whole-window action. Recording every
        // value the indicator text signal takes (not just its final value)
        // proves the per-pane calls were suppressed, not merely overwritten
        // an instant apart. Solid's createEffect defers its first run to a
        // microtask, so this awaits a tick after creating it (to capture the
        // baseline read) and again after the gesture (to let any effect runs
        // it triggered actually flush) before disposing and asserting.
        it("updates the indicator text signal exactly once per gesture, not once per pane", async () => {
            setBlock("t1", "term", 1.0);
            setBlock("a1", "agent", 1.0);
            setBlock("e1", "editor", 1.0);

            const seen: string[] = [];
            let disposeRoot: () => void = () => {};
            createRoot((dispose) => {
                disposeRoot = dispose;
                createEffect(() => {
                    seen.push(zoomIndicatorTextAtom());
                });
            });
            await Promise.resolve();

            zoomAllPanesIn();
            await Promise.resolve();
            disposeRoot();

            // First entry is the effect's own initial (baseline) read;
            // the gesture itself must contribute exactly one further entry,
            // not one per pane.
            expect(seen.length).toBe(2);
            expect(seen[1]).toBe("All panes: 105%");
        });

        it("a single-pane zoomBlockIn still shows its own per-pane percentage (unaffected by the all-panes suppression)", () => {
            setBlock("t1", "term", 1.0);
            zoomBlockIn("t1");
            expect(zoomIndicatorTextAtom()).toBe("105%");
        });
    });
});
