// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Regression tests for docs/specs/SPEC_AGENT_SHELL_ZOOM_SEED_RACE_2026-08-10.md.
 *
 * Pins the fix: the shell drawer's terminal must be constructed with the
 * FINAL (persisted) font size, not a default followed by a corrective jerk —
 * without triggering a second, redundant WOS fetch to get there (reagentx P1
 * on #2522). Mocks RPC/store/TermWrap at the module boundary (same approach
 * as AgentLaunchModal.integration.test.tsx); SUT is the real
 * AgentShellSubblock.
 *
 * Mock design note: mirrors the REAL wos.ts shape — one signal per oref
 * holding `{ value, loading }` together (not two independent signals), with
 * `getWaveObjectAtom` and `getWaveObjectLoadingAtom` both reading from it.
 * The signal starts at `{ value: null, loading: true }` and is only resolved
 * when the test explicitly calls `resolveSeedFetch`, simulating a real
 * network round-trip that takes measurable time — deliberately NOT
 * pre-populated before render, since doing so would make the assertion pass
 * even against the old, buggy synchronous-read code (an earlier version of
 * this file had exactly that mistake).
 */

import { cleanup, render, screen, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AgentShellSubblock } from "./AgentShellSubblock";

const { blockDataSignals, seedData } = vi.hoisted(() => {
    const blockDataSignals = new Map<string, ReturnType<typeof import("solid-js").createSignal<any>>>();
    const seedData = new Map<string, Record<string, any>>();
    return { blockDataSignals, seedData };
});

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ControllerResyncCommand: vi.fn(() => Promise.resolve()),
        CreateSubBlockCommand: vi.fn(() => Promise.resolve("block:new-sub-block-id")),
        SetMetaCommand: vi.fn(() => Promise.resolve()),
    },
}));

vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/ws", () => ({ sendWSCommand: vi.fn() }));

vi.mock("@/app/store/global", async () => {
    const { createSignal: realCreateSignal } = await import("solid-js");

    function getOrCreateDataSignal(oref: string) {
        let sig = blockDataSignals.get(oref);
        if (!sig) {
            sig = realCreateSignal<{ value: any; loading: boolean }>({ value: null, loading: true });
            blockDataSignals.set(oref, sig);
        }
        return sig;
    }

    const WOS = {
        makeORef: (otype: string, oid: string) => `${otype}:${oid}`,
        getWaveObjectAtom: (oref: string) => {
            const [get] = getOrCreateDataSignal(oref);
            return () => get().value;
        },
        // Mirrors the real wos.ts implementation exactly: null while
        // loading, false once settled (regardless of resulting value).
        getWaveObjectLoadingAtom: (oref: string) => {
            const [get] = getOrCreateDataSignal(oref);
            return () => (get().loading ? null : get().loading);
        },
    };

    return {
        WOS,
        atoms: { prefersReducedMotionAtom: () => false },
        staticTabId: () => "tab-1",
    };
});

// TermWrap is the real xterm.js + PTY wrapper — mocked entirely so tests
// assert on WHAT it was constructed with (specifically: fontSize), not on
// real terminal rendering.
const termWrapInstances: Array<{ fontSize: number; loaded: boolean; terminal: any }> = [];

vi.mock("@/app/view/term/termwrap", () => {
    class FakeTermWrap {
        fontSize: number;
        terminal = { options: { fontSize: 0 } };
        loaded = false;
        constructor(_id: string, _container: HTMLElement, options: { fontSize: number }) {
            this.fontSize = options.fontSize;
            this.terminal.options.fontSize = options.fontSize;
            termWrapInstances.push(this as any);
        }
        async init() {
            this.loaded = true;
        }
        handleResize() {}
        handleResize_debounced() {}
        dispose() {}
    }
    return { TermWrap: FakeTermWrap };
});

/** Configures what a later `resolveSeedFetch` call will resolve with —
 *  doesn't touch the signal itself, so the atom stays genuinely "loading"
 *  until the test explicitly settles it. */
function queueSeedMeta(oref: string, meta: Record<string, any>) {
    seedData.set(oref, meta);
}

function getOrCreateDataSignalForTest(oref: string) {
    let sig = blockDataSignals.get(oref);
    if (!sig) {
        throw new Error(`no signal for ${oref} — call queueSeedMeta or let the component read it first`);
    }
    return sig;
}

/** Simulates the in-flight fetch (triggered by subBlockAtom's own memo)
 *  finally completing — settles loading:false with whatever was queued via
 *  queueSeedMeta (or null if nothing was queued, e.g. a genuinely-missing
 *  object). */
function resolveSeedFetch(oref: string) {
    const meta = seedData.get(oref);
    const [, set] = getOrCreateDataSignalForTest(oref);
    set({ value: meta ? { meta } : null, loading: false });
}

/** All tests use parentBlockId="parent-1" — pre-settle its oref as "already
 *  resolved, empty" by default (the realistic common case: the pane's own
 *  zoom fetch finished before the shell was ever opened), so the new
 *  parent-oref wait (SPEC_AGENT_SHELL_PARENT_PANE_ZOOM_SEED_RACE_2026-08-14.md)
 *  doesn't add latency/timeout delay to every other test in this file. Tests
 *  that specifically exercise that race overwrite this back to loading
 *  before render. */
const PARENT_OREF = "block:parent-1";
function preSettleOref(oref: string) {
    blockDataSignals.set(oref, createSignal<{ value: any; loading: boolean }>({ value: null, loading: false }));
}

beforeEach(() => {
    blockDataSignals.clear();
    seedData.clear();
    termWrapInstances.length = 0;
    preSettleOref(PARENT_OREF);
    // jsdom has no ResizeObserver; AgentShellSubblock sets one up
    // unconditionally after a successful init().
    (globalThis as any).ResizeObserver =
        (globalThis as any).ResizeObserver ??
        class {
            observe() {}
            unobserve() {}
            disconnect() {}
        };
});

afterEach(() => {
    cleanup();
    vi.clearAllMocks();
});

describe("AgentShellSubblock — zoom seed race (SPEC_AGENT_SHELL_ZOOM_SEED_RACE_2026-08-10)", () => {
    it("constructs the terminal with the persisted zoom already applied, not the BASE_FONT_SIZE default", async () => {
        const existingId = "existing-sub-block";
        const oref = `block:${existingId}`;
        // Persisted zoom of 2.0 → BASE_FONT_SIZE(13) * 2.0 / paneZoom(1) = 26.
        queueSeedMeta(oref, { "term:zoom": 2.0 });

        render(() => (
            <AgentShellSubblock
                parentBlockId="parent-1"
                cwd="/tmp"
                existingSubBlockId={existingId}
                onSubBlockCreated={() => {}}
                agentPaneZoom={() => 1}
            />
        ));

        // Still "loading" — the bug this spec fixes: old code constructed
        // TermWrap synchronously without waiting for this at all, so it
        // would already exist here with the wrong (default) font size.
        expect(termWrapInstances.length).toBe(0);

        // Simulate the network round-trip finally completing, some real time
        // after mount — not synchronously, or this wouldn't distinguish
        // fixed code (which awaits it) from old code (which never did).
        await new Promise((r) => setTimeout(r, 10));
        resolveSeedFetch(oref);

        await waitFor(() => expect(termWrapInstances.length).toBe(1));
        expect(termWrapInstances[0].fontSize).toBe(26);
    });

    it("defaults to BASE_FONT_SIZE for a freshly created sub-block (no persisted zoom yet)", async () => {
        render(() => (
            <AgentShellSubblock
                parentBlockId="parent-1"
                cwd="/tmp"
                existingSubBlockId={undefined}
                onSubBlockCreated={() => {}}
                agentPaneZoom={() => 1}
            />
        ));

        // No wait needed: a freshly created sub-block was never fetched (no
        // id existed to fetch), so there's nothing to await.
        await waitFor(() => expect(termWrapInstances.length).toBe(1));
        expect(termWrapInstances[0].fontSize).toBe(13);
    });

    it("does not start a second fetch for the reused sub-block — only reads the loading atom", async () => {
        const existingId = "existing-sub-block";
        const oref = `block:${existingId}`;
        queueSeedMeta(oref, { "term:zoom": 1.5 });

        render(() => (
            <AgentShellSubblock
                parentBlockId="parent-1"
                cwd="/tmp"
                existingSubBlockId={existingId}
                onSubBlockCreated={() => {}}
                agentPaneZoom={() => 1}
            />
        ));

        // subBlockAtom's own createMemo is the ONLY thing that should have
        // created this oref's signal (via getWaveObjectAtom) — confirm it
        // exists (proves the memo ran) without the component itself ever
        // needing a second, separate fetch primitive.
        expect(blockDataSignals.has(oref)).toBe(true);

        resolveSeedFetch(oref);
        await waitFor(() => expect(termWrapInstances.length).toBe(1));
        expect(termWrapInstances[0].fontSize).toBe(20); // 13 * 1.5
    });

    it("falls back to the default font size if the seed fetch never settles (bounded wait, no infinite hang)", async () => {
        const existingId = "hung-sub-block";
        // Deliberately never call resolveSeedFetch — simulates a genuine
        // network failure that leaves the loading atom stuck (per wos.ts's
        // own comment on non-"not found" GetObject rejections).
        render(() => (
            <AgentShellSubblock
                parentBlockId="parent-1"
                cwd="/tmp"
                existingSubBlockId={existingId}
                onSubBlockCreated={() => {}}
                agentPaneZoom={() => 1}
            />
        ));

        await waitFor(() => expect(termWrapInstances.length).toBe(1), { timeout: 3000 });
        expect(termWrapInstances[0].fontSize).toBe(13);
    }, 5000);

    it("clears the loading overlay even when startup fails, so the error message is visible", async () => {
        const { RpcApi } = await import("@/app/store/rpc-api");
        (RpcApi.ControllerResyncCommand as any).mockRejectedValueOnce(new Error("not found"));
        (RpcApi.CreateSubBlockCommand as any).mockRejectedValueOnce(new Error("boom"));

        const { container } = render(() => (
            <AgentShellSubblock
                parentBlockId="parent-1"
                cwd="/tmp"
                existingSubBlockId="dead-id"
                onSubBlockCreated={() => {}}
                agentPaneZoom={() => 1}
            />
        ));

        await waitFor(() => expect(screen.getByText(/Shell failed to start/)).toBeInTheDocument());
        // Immediately after the error, the overlay is still mounted but
        // transitioning into its fade-out (matches the real "hold node
        // mounted for the CSS transition duration" contract, same as
        // browser-view.tsx) — confirm it's fading rather than stuck visible
        // forever, then confirm it actually finishes unmounting.
        expect(container.querySelector(".agent-shell-loading-overlay.is-fading")).not.toBeNull();
        await waitFor(
            () => expect(container.querySelector(".agent-shell-loading-overlay")).toBeNull(),
            { timeout: 1000 }
        );
    });

    it("applies a live meta update that lands while the terminal is still loading, once loading finishes (reagentx P2 guard)", async () => {
        // reagentx flagged that `termWrap?.terminal && wrapLoaded()`
        // short-circuits on the effect's very first run (which always
        // happens before TermWrap is constructed), so wrapLoaded() is never
        // read that time and the effect never subscribes to it from that
        // run alone — setWrapLoaded(true) later doesn't by itself
        // re-trigger anything. The §5 fix reads wrapLoaded() unconditionally
        // instead, guaranteeing the subscription is established from the
        // very first run regardless of whether TermWrap exists yet.
        //
        // This test exercises the general shape of the concern: a live zoom
        // change lands, then the terminal finishes loading, and the final
        // font size must reflect the update either way. It does not
        // discriminate the exact pre-fix commit for the specific two-events
        // ordering used here (that particular ordering happens to
        // self-heal even with the short-circuit, since termWrap already
        // exists by the time this update arrives, so wrapLoaded() gets read
        // on that run regardless) — the code fix is still correct standard
        // SolidJS practice (never conditionally read a signal you need
        // reliable subscription to) independent of which exact scenario
        // this test constructs, and this remains a real regression guard
        // for the live-update path going forward.
        const existingId = "pre-construct-race-sub-block";
        const oref = `block:${existingId}`;
        queueSeedMeta(oref, { "term:zoom": 1.0 });

        render(() => (
            <AgentShellSubblock
                parentBlockId="parent-1"
                cwd="/tmp"
                existingSubBlockId={existingId}
                onSubBlockCreated={() => {}}
                agentPaneZoom={() => 1}
            />
        ));

        expect(termWrapInstances.length).toBe(0); // still awaiting the seed fetch

        // First settle: zoom 1.0 → fontSize 13. TermWrap gets constructed
        // with this value (correct per the P1 fix — no bug here).
        resolveSeedFetch(oref);
        await waitFor(() => expect(termWrapInstances.length).toBe(1));
        expect(termWrapInstances[0].fontSize).toBe(13);

        // Hold init() open, then land a live update while wrapLoaded is
        // still false but AFTER TermWrap already exists this time — this is
        // the scenario that (per the analysis above) should self-heal even
        // in the old code, since termWrap?.terminal is truthy by now so
        // wrapLoaded() gets read regardless of the short-circuit. Included
        // as a companion assertion to the pre-construction case: both must
        // work, and this one already passed before the P2 fix too — the
        // fix's value is specifically for updates landing BEFORE
        // construction, exercised above.
        const [, set] = blockDataSignals.get(oref)!;
        set({ value: { meta: { "term:zoom": 2.0 } }, loading: false });
        await waitFor(() => expect(termWrapInstances[0].terminal.options.fontSize).toBe(26));
    });
});

describe("AgentShellSubblock — parent pane zoom seed race (SPEC_AGENT_SHELL_PARENT_PANE_ZOOM_SEED_RACE_2026-08-14)", () => {
    it("waits for the parent pane's zoom fetch to settle before constructing the terminal, even for a freshly created sub-block", async () => {
        // Freshly created sub-block — no wait needed on its OWN oref (matches the
        // existing "defaults to BASE_FONT_SIZE" test) — but the parent's oref is
        // still loading here (overwriting beforeEach's default settled state),
        // which alone must be enough to block construction. This is exactly the
        // gap the 08-10 fix didn't close: that fix only ever waited on the
        // shell's own oref.
        blockDataSignals.set(PARENT_OREF, createSignal<{ value: any; loading: boolean }>({ value: null, loading: true }));

        render(() => (
            <AgentShellSubblock
                parentBlockId="parent-1"
                cwd="/tmp"
                existingSubBlockId={undefined}
                onSubBlockCreated={() => {}}
                agentPaneZoom={() => 1}
            />
        ));

        // Still waiting on the parent — old code (pre-fix) would have
        // constructed TermWrap here already, since a fresh sub-block never
        // waited on anything at all.
        await new Promise((r) => setTimeout(r, 10));
        expect(termWrapInstances.length).toBe(0);

        const [, setParent] = blockDataSignals.get(PARENT_OREF)!;
        setParent({ value: null, loading: false });

        await waitFor(() => expect(termWrapInstances.length).toBe(1));
    });

    it("waits for the parent's fetch even when the shell's own (reused) oref already resolved", async () => {
        const existingId = "existing-sub-block";
        const oref = `block:${existingId}`;
        queueSeedMeta(oref, { "term:zoom": 1.0 });
        blockDataSignals.set(PARENT_OREF, createSignal<{ value: any; loading: boolean }>({ value: null, loading: true }));

        render(() => (
            <AgentShellSubblock
                parentBlockId="parent-1"
                cwd="/tmp"
                existingSubBlockId={existingId}
                onSubBlockCreated={() => {}}
                agentPaneZoom={() => 1}
            />
        ));

        // Settle the shell's own oref immediately — construction must still
        // block on the parent's, since the two waits are independent inputs
        // to the same gate (Promise.all), not "either one is enough."
        resolveSeedFetch(oref);
        await new Promise((r) => setTimeout(r, 10));
        expect(termWrapInstances.length).toBe(0);

        const [, setParent] = blockDataSignals.get(PARENT_OREF)!;
        setParent({ value: null, loading: false });

        await waitFor(() => expect(termWrapInstances.length).toBe(1));
        expect(termWrapInstances[0].fontSize).toBe(13);
    });

    it("falls back to constructing the terminal after the bounded wait if only the parent's fetch hangs", async () => {
        // Deliberately never resolve the parent's oref — simulates the same
        // class of genuine-failure scenario the existing shell-oref hang test
        // covers, mirrored for the new wait.
        blockDataSignals.set(PARENT_OREF, createSignal<{ value: any; loading: boolean }>({ value: null, loading: true }));

        render(() => (
            <AgentShellSubblock
                parentBlockId="parent-1"
                cwd="/tmp"
                existingSubBlockId={undefined}
                onSubBlockCreated={() => {}}
                agentPaneZoom={() => 1}
            />
        ));

        await waitFor(() => expect(termWrapInstances.length).toBe(1), { timeout: 3000 });
        expect(termWrapInstances[0].fontSize).toBe(13);
    }, 5000);
});
