// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for the all-panes zoom stepper (Ctrl+Shift+Scroll — see
 * docs/specs/SPEC_CTRL_SHIFT_SCROLL_ZOOM_ALL_PANES_2026_09_07.md). Existing
 * single-pane zoom (zoomBlockIn/Out, chromeZoomIn/Out) is exercised
 * end-to-end already via section-pane.test.tsx / warden-view.test.tsx; this
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
    MOS: {
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

// Which views take part in pane zoom is their manifest's `paneZoom`
// capability (Pane Tab contract Phase 5); declared here as block-registry.ts
// declares it for the built-ins.
import { registerPaneTab } from "@/app/block/pane-tab-registry";
import { stubPaneTab } from "@/app/block/pane-tab-test-utils";
for (const view of ["term", "agent", "swarm", "memory", "warden"]) {
    registerPaneTab(stubPaneTab(view, { capabilities: { paneZoom: {} } }));
}
registerPaneTab(stubPaneTab("editor", { capabilities: { paneZoom: { baseFontSize: 13 } } }));
registerPaneTab(stubPaneTab("browser"));

import { setPlatform } from "@/util/platformutil";
import { chromeZoomAtom } from "./chrome-zoom";
import { LOCAL_ZOOM_HOLD_MS, paneZoom, resetPaneZoomForTests } from "./pane-zoom-local";
import { useZoomAnchor } from "./zoom-anchor";
import { chromeZoomIn, initChromeZoom, usePaneZoomWheel, zoomAllPanesIn, zoomAllPanesOut, zoomBlockByWheel, zoomBlockIn, zoomIndicatorTextAtom, setZoomStepRatio } from "./zoom";
import { setZoomMax } from "./zoom-factor";

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
    resetPaneZoomForTests();
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
    // memory/swarm (both already included) — leaving it out of
    // getBlockZoom's allowlist was an oversight, not a deliberate
    // exclusion (unlike browser, which is excluded on purpose — see the
    // spec's Non-goals). A warden pane must be part of the batch.
    it("includes warden panes in the batch — they already speak term:zoom identically to memory/swarm", () => {
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
    // — MOS's local object cache is NOT updated synchronously by it, only
    // later when the backend pushes a MuxObjUpdate event back
    // (global.ts's initGlobalEventSubs). Every OTHER test in this file uses
    // a setMetaMock that updates blockMetas synchronously, which masked a
    // real bug: stepAllPanes originally re-read getBlockZoom(blockId) right
    // after stepZoom() to compute the summary range, which — against the
    // real system's actual timing — would have read the STALE, pre-step
    // value every single time. This test's mock deliberately does NOT
    // update blockMetas at all, reproducing that stale-cache condition
    // exactly, to prove the fix (using stepZoom's own return value instead
    // of re-reading) doesn't depend on the cache having updated.
    it("computes the summary range from the freshly stepped value, not a re-read of MOS's (unsynced) cache", () => {
        // Deliberately never touches blockMetas — reproduces the real
        // system's actual timing (RpcApi.SetMetaCommand is fire-and-forget;
        // MOS's cache only updates later, off a MuxObjUpdate event).
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

describe("chrome zoom across a reload", () => {
    beforeEach(() => sessionStorage.clear());

    it("comes back at the factor the window had", () => {
        initChromeZoom();
        chromeZoomIn(0.1);
        const before = chromeZoomAtom();
        expect(before).toBeCloseTo(1.1);
        // A reload: the module's signal starts over, sessionStorage doesn't.
        initChromeZoom();
        expect(chromeZoomAtom()).toBe(before);
        expect(document.documentElement.style.getPropertyValue("--zoomfactor")).toBe(String(before));
    });

    it("starts at 100% on a pool boot, and forgets the previous window's zoom", () => {
        initChromeZoom();
        chromeZoomIn(0.2);
        // The window is demoted back to the pool: same renderer, same sessionStorage.
        initChromeZoom({ fresh: true });
        expect(chromeZoomAtom()).toBe(1);
        // The window promoted from it next, and its own reloads, start from 100%.
        initChromeZoom();
        expect(chromeZoomAtom()).toBe(1);
    });

    it("starts at 100% in a new session", () => {
        initChromeZoom();
        expect(chromeZoomAtom()).toBe(1);
    });
});

// The one Ctrl+scroll path for content that would stop the wheel before the window
// sees it (xterm, CodeMirror, the agent pane's shell).
// SPEC_SYSINFO_TEXT_ZOOM_DENSITY_AND_ONE_ZOOM_PATH_2026_10_18.md §3.
describe("zoomBlockByWheel / usePaneZoomWheel", () => {
    it("steps the block by the shared wheel step and shows the indicator", () => {
        setBlock("t1", "term");
        zoomBlockByWheel("t1", -100);
        expect(currentZoom("t1")).toBeCloseTo(1.05, 5);
        expect(zoomIndicatorTextAtom()).toBe("105%");
        zoomBlockByWheel("t1", 100);
        expect(currentZoom("t1")).toBeCloseTo(1.0, 5);
    });

    // A headless shell draws at 13px: at 50%, a step must change that font, not a
    // default 15px one (#5128).
    it("skips steps by the font size the block says it draws at", () => {
        blockMetas.set("sub1", { "term:zoom": 0.5 });
        zoomBlockByWheel("sub1", -100, 13);
        // round(13 × 0.55) = 7 is still 13 × 0.5's 7px, so it skips on to 0.58 (8px).
        expect(currentZoom("sub1")).toBeCloseTo(0.58, 5);
    });

    it("zooms a block with no pane capability (a headless sub-block)", () => {
        blockMetas.set("sub1", { "term:fontsize": 20 });
        zoomBlockByWheel("sub1", -100);
        expect(currentZoom("sub1")).toBeCloseTo(1.05, 5);
    });

    function mountWheel(blockId: string) {
        const el = document.createElement("div");
        document.body.appendChild(el);
        const dispose = createRoot((d) => {
            usePaneZoomWheel(() => el, () => blockId);
            return d;
        });
        return { el, dispose };
    }

    it("takes Ctrl+wheel first and stops it, so the content and the window don't also act", async () => {
        setPlatform("win32"); // mod+wheel: Ctrl here, Cmd on macOS
        setBlock("t1", "term");
        const { el, dispose } = mountWheel("t1");
        await Promise.resolve();
        const ev = new WheelEvent("wheel", { ctrlKey: true, deltaY: -100, bubbles: true, cancelable: true });
        el.dispatchEvent(ev);
        expect(ev.defaultPrevented).toBe(true);
        expect(currentZoom("t1")).toBeCloseTo(1.05, 5);
        dispose();
        el.remove();
    });

    it("leaves a wheel the pane says it does not apply to for the window's handler", async () => {
        setPlatform("win32");
        setBlock("t1", "term");
        const el = document.createElement("div");
        document.body.appendChild(el);
        const dispose = createRoot((d) => {
            usePaneZoomWheel(() => el, () => "t1", undefined, (ev) => ev.deltaY > 0);
            return d;
        });
        await Promise.resolve();
        const up = new WheelEvent("wheel", { ctrlKey: true, deltaY: -100, bubbles: true, cancelable: true });
        el.dispatchEvent(up);
        expect(up.defaultPrevented).toBe(false);
        expect(currentZoom("t1")).toBe(1);
        const down = new WheelEvent("wheel", { ctrlKey: true, deltaY: 100, bubbles: true, cancelable: true });
        el.dispatchEvent(down);
        expect(down.defaultPrevented).toBe(true);
        dispose();
        el.remove();
    });

    it("leaves a plain wheel and Ctrl+Shift+wheel (all panes) alone", async () => {
        setBlock("t1", "term");
        const { el, dispose } = mountWheel("t1");
        await Promise.resolve();
        el.dispatchEvent(new WheelEvent("wheel", { deltaY: -100, bubbles: true, cancelable: true }));
        el.dispatchEvent(new WheelEvent("wheel", { ctrlKey: true, shiftKey: true, deltaY: -100, bubbles: true, cancelable: true }));
        expect(setMetaMock).not.toHaveBeenCalled();
        dispose();
        el.remove();
    });
});

describe("a zoom is held until its write settles", () => {
    const meta = (id: string) => blockMetas.get(id);

    it("two wheel ticks before the update arrives are two steps, not one", () => {
        setBlock("t1", "term");
        setMetaMock.mockImplementation(() => new Promise(() => {})); // the backend has not answered

        zoomBlockByWheel("t1", -1, 20);
        zoomBlockByWheel("t1", -1, 20);

        expect(meta("t1")?.["term:zoom"]).toBeUndefined();
        expect(paneZoom("t1", meta("t1"))).toBeCloseTo(1.1, 5);
    });

    it("lets go once the write has settled and its update has had time to arrive", async () => {
        vi.useFakeTimers();
        try {
            setBlock("t1", "term");
            zoomBlockByWheel("t1", -1, 20);
            expect(paneZoom("t1", { "term:zoom": 1 })).toBeCloseTo(1.05, 5); // held
            await vi.advanceTimersByTimeAsync(LOCAL_ZOOM_HOLD_MS + 1);
            expect(paneZoom("t1", { "term:zoom": 1 })).toBe(1); // the meta rules again
        } finally {
            vi.useRealTimers();
        }
    });

    it("goes back to the stored level at once when the write is refused", async () => {
        setBlock("t1", "term", 1.2);
        setMetaMock.mockImplementation(() => Promise.reject(new Error("refused")));
        const errors = vi.spyOn(console, "error").mockImplementation(() => {});

        zoomBlockByWheel("t1", -1, 20);
        expect(paneZoom("t1", meta("t1"))).toBeCloseTo(1.25, 5);
        await new Promise((r) => setTimeout(r, 0));

        expect(paneZoom("t1", meta("t1"))).toBe(1.2);
        errors.mockRestore();
    });

    it("an older write settling does not let go of a newer level", async () => {
        vi.useFakeTimers();
        try {
            setBlock("t1", "term");
            zoomBlockByWheel("t1", -1, 20);
            await vi.advanceTimersByTimeAsync(LOCAL_ZOOM_HOLD_MS - 100);
            zoomBlockByWheel("t1", -1, 20);
            await vi.advanceTimersByTimeAsync(200); // the first write's hold would have ended by now
            expect(paneZoom("t1", { "term:zoom": 1 })).toBeCloseTo(1.1, 5);
        } finally {
            vi.useRealTimers();
        }
    });
});

describe("zoom takes an anchor from the pointer", () => {
    function pane(id: string) {
        const capture = vi.fn(() => ({ held: true }));
        const restore = vi.fn(() => true);
        let dispose = () => {};
        createRoot((d) => {
            dispose = d;
            useZoomAnchor({ blockId: id, zoom: () => 1, scroller: () => undefined, following: () => false, adapter: { capture, restore } });
        });
        return { capture, restore, dispose: () => dispose() };
    }

    it("a wheel step hands the pane the pointer's position", () => {
        setBlock("a1", "agent");
        const p = pane("a1");

        zoomBlockByWheel("a1", -1, 20, { clientX: 40, clientY: 90 });

        expect(p.capture).toHaveBeenCalledWith({ clientX: 40, clientY: 90 });
        p.dispose();
    });

    it("the window's Ctrl+wheel handler and the all-panes step pass it on as well", () => {
        setBlock("a1", "agent");
        const p = pane("a1");

        zoomBlockIn("a1", 0.05, { clientX: 1, clientY: 2 });
        expect(p.capture).toHaveBeenLastCalledWith({ clientX: 1, clientY: 2 });
        zoomAllPanesOut(0.05, { clientX: 7, clientY: 8 });
        expect(p.capture).toHaveBeenLastCalledWith({ clientX: 7, clientY: 8 });
        p.dispose();
    });

    it("a step that lands on the limit moves nothing, so nothing is held", () => {
        setBlock("a1", "agent", 2);
        const p = pane("a1");

        zoomBlockByWheel("a1", -1, 20, { clientX: 40, clientY: 90 });

        expect(p.capture).not.toHaveBeenCalled();
        p.dispose();
    });
});

describe("a block that steps by ratio (a PDF)", () => {
    it("multiplies by the ratio per notch and divides back, whatever the level", () => {
        setBlock("a1", "agent", 1);
        const release = setZoomStepRatio("a1", 1.25);
        zoomBlockByWheel("a1", -100);
        expect(currentZoom("a1")).toBeCloseTo(1.25, 1);
        zoomBlockByWheel("a1", 100);
        expect(currentZoom("a1")).toBeCloseTo(1, 1);
        release();
    });

    it("scales the step by how much of a notch the event is, so a trackpad pinch stays gentle", () => {
        setBlock("a1", "agent", 1);
        const release = setZoomStepRatio("a1", 1.25);
        zoomBlockByWheel("a1", -10);
        expect(currentZoom("a1")).toBeGreaterThan(1);
        expect(currentZoom("a1")).toBeLessThan(1.05);
        release();
    });

    it("steps by the shared 5% again once the viewer lets go", () => {
        setBlock("a1", "agent", 1);
        setZoomStepRatio("a1", 1.25)();
        zoomBlockByWheel("a1", -100);
        expect(currentZoom("a1")).toBeCloseTo(1.05, 2);
    });

    it("leaves other blocks on the shared step", () => {
        setBlock("a1", "agent", 1);
        setBlock("a2", "agent", 1);
        const release = setZoomStepRatio("a1", 1.25);
        zoomBlockByWheel("a2", -100);
        expect(currentZoom("a2")).toBeCloseTo(1.05, 2);
        release();
    });
});

describe("a block with a higher top (a PDF)", () => {
    it("steps past the shared top, up to its own, and no further", () => {
        setBlock("a1", "agent", 1.9);
        const release = setZoomMax("a1", 15);
        const releaseRatio = setZoomStepRatio("a1", 1.25);
        zoomBlockByWheel("a1", -100);
        expect(currentZoom("a1")).toBeCloseTo(2.38, 1);
        for (let i = 0; i < 20; i++) zoomBlockByWheel("a1", -100);
        expect(currentZoom("a1")).toBe(15);
        releaseRatio();
        release();
    });

    it("keeps its steps small where the font size would stop changing (a big zoom, a small pinch)", () => {
        // 13px at 5x is past the 64px font ceiling: a step that skipped to a new font size would run to the top.
        setBlock("e1", "editor", 5);
        blockMetas.get("e1")!["term:fontsize"] = 13;
        const release = setZoomMax("e1", 15);
        const releaseRatio = setZoomStepRatio("e1", 1.25);
        zoomBlockByWheel("e1", -100);
        expect(currentZoom("e1")).toBeCloseTo(6.25, 1);
        zoomBlockByWheel("e1", 100);
        expect(currentZoom("e1")).toBeCloseTo(5, 1);
        setBlock("e2", "editor", 1);
        blockMetas.get("e2")!["term:fontsize"] = 13;
        const releaseRatio2 = setZoomStepRatio("e2", 1.25);
        zoomBlockByWheel("e2", -10);
        expect(currentZoom("e2")).toBeLessThan(1.03);
        releaseRatio2();
        releaseRatio();
        release();
    });
});
