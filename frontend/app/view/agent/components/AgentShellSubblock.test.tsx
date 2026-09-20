// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Regression tests for docs/specs/SPEC_AGENT_SHELL_ZOOM_SEED_RACE_2026-08-10.md.
 *
 * Pins the fix: the shell drawer's terminal must be constructed with the
 * FINAL (persisted) font size, not a default followed by a corrective jerk —
 * without triggering a second, redundant MOS fetch to get there (reagentx P1
 * on #2522). Mocks RPC/store/TermWrap at the module boundary (same approach
 * as AgentLaunchModal.integration.test.tsx); SUT is the real
 * AgentShellSubblock.
 *
 * Mock design note: mirrors the REAL mos.ts shape — one signal per oref
 * holding `{ value, loading }` together (not two independent signals), with
 * `getMuxObjectAtom` and `getMuxObjectLoadingAtom` both reading from it.
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
import { DEFAULT_TERM_SCROLLBACK } from "@/app/view/term/termscrollback";

const {
    blockDataSignals,
    seedData,
    mpsHandlers,
    mpsPersisted,
    resyncDeferreds,
    resyncRejections,
    termSettingsBag,
} = vi.hoisted(() => {
    const blockDataSignals = new Map<string, ReturnType<typeof import("solid-js").createSignal<any>>>();
    const seedData = new Map<string, Record<string, any>>();
    const mpsHandlers = new Map<string, Array<(event: any) => void>>();
    const mpsPersisted = new Map<string, Record<string, unknown>>();
    // Per-block-id controllable resolution for ControllerResyncCommand —
    // lets a test hold a specific resync open to construct an
    // out-of-order-completion race between two overlapping attach
    // attempts. Absent an entry, the mock resolves immediately (every
    // existing test's assumption).
    const resyncDeferreds = new Map<string, { resolve: () => void }>();
    // Ids for which ControllerResyncCommand rejects immediately, forcing
    // attachShell down the create-new-block fallback path.
    const resyncRejections = new Set<string>();
    const termSettingsBag: Record<string, unknown> = {};
    return {
        blockDataSignals,
        seedData,
        mpsHandlers,
        mpsPersisted,
        resyncDeferreds,
        resyncRejections,
        // Mutable `term:*` settings the global mock serves. Hoisted alongside
        // the other mock state so the vi.mock factory can close over it.
        termSettingsBag,
    };
});

// Records subscriptions by "<eventType>|<scope>" so a test can emit to ONE
// scope and prove the component isn't listening to everything. Honours
// unsubscribe, so the "stops listening on unmount" behaviour is observable
// rather than assumed.
//
// Crucially it also models `persist: 1` REPLAY: `controllerstatus` is
// published persisted specifically so a new subscriber is handed the current
// status synchronously as part of subscribing (mps.ts's `replay_to_route`).
// The first version of this mock only delivered events a test emitted AFTER
// mount, which made the replay path — where ReAgent found a P0 — structurally
// invisible to all 12 tests. `queuePersistedStatus` puts an event in that
// replay slot instead.
vi.mock("@/app/store/mps", () => ({
    muxEventSubscribe: (opts: { eventType: string; scope: string; handler: (event: any) => void }) => {
        const key = `${opts.eventType}|${opts.scope}`;
        mpsHandlers.set(key, [...(mpsHandlers.get(key) ?? []), opts.handler]);
        const persisted = mpsPersisted.get(key);
        if (persisted !== undefined) {
            // Synchronously, inside subscribe — exactly how the broker does it.
            opts.handler({ data: persisted });
        }
        return () => {
            mpsHandlers.set(
                key,
                (mpsHandlers.get(key) ?? []).filter((h) => h !== opts.handler)
            );
        };
    },
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ControllerResyncCommand: vi.fn((_client: any, params: { blockid: string }) => {
            if (resyncRejections.has(params.blockid)) {
                return Promise.reject(new Error("not found"));
            }
            return new Promise<void>((resolve) => {
                if (resyncDeferreds.has(params.blockid)) {
                    resyncDeferreds.set(params.blockid, { resolve });
                } else {
                    resolve();
                }
            });
        }),
        CreateSubBlockCommand: vi.fn(() => Promise.resolve("block:new-sub-block-id")),
        SetMetaCommand: vi.fn(() => Promise.resolve()),
        // Absent until the reattach-failure tests below needed it, which is
        // why that path — where ReAgent found the round-2 P0 — had no coverage.
        DeleteSubBlockCommand: vi.fn(() => Promise.resolve()),
    },
}));

/** Registers `id` for controllable resync resolution — the next
 *  `ControllerResyncCommand` call for this id will not resolve until
 *  `resolveDeferredResync(id)` is called. */
function deferResync(id: string) {
    resyncDeferreds.set(id, { resolve: () => {} });
}

function resolveDeferredResync(id: string) {
    const entry = resyncDeferreds.get(id);
    if (!entry) throw new Error(`no deferred resync registered for ${id} — call deferResync first`);
    entry.resolve();
    resyncDeferreds.delete(id);
}

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

    const MOS = {
        makeORef: (otype: string, oid: string) => `${otype}:${oid}`,
        getMuxObjectAtom: (oref: string) => {
            const [get] = getOrCreateDataSignal(oref);
            return () => get().value;
        },
        // Mirrors the real mos.ts implementation exactly: null while
        // loading, false once settled (regardless of resulting value).
        getMuxObjectLoadingAtom: (oref: string) => {
            const [get] = getOrCreateDataSignal(oref);
            return () => (get().loading ? null : get().loading);
        },
    };

    return {
        MOS,
        atoms: { prefersReducedMotionAtom: () => false },
        staticTabId: () => "tab-1",
        // The drawer resolves `term:scrollback` through this (see
        // termscrollback.ts). Backed by a mutable bag so a test can set the
        // value before mounting and assert what TermWrap was constructed with.
        getSettingsPrefixAtom: (prefix: string) => () => (prefix === "term" ? termSettingsBag : {}),
    };
});

// TermWrap is the real xterm.js + PTY wrapper — mocked entirely so tests
// assert on WHAT it was constructed with (specifically: fontSize and, for
// the agent-lock tests, the sendDataHandler closure), not on real terminal
// rendering.
const termWrapInstances: Array<{
    id: string;
    fontSize: number;
    scrollback: number | undefined;
    loaded: boolean;
    disposed: boolean;
    terminal: any;
    sendDataHandler: (data: string) => void;
}> = [];

vi.mock("@/app/view/term/termwrap", () => {
    class FakeTermWrap {
        id: string;
        fontSize: number;
        scrollback: number | undefined;
        terminal = { options: { fontSize: 0 } };
        loaded = false;
        disposed = false;
        sendDataHandler: (data: string) => void;
        constructor(
            id: string,
            _container: HTMLElement,
            options: { fontSize: number; scrollback?: number },
            muxOptions: { sendDataHandler: (data: string) => void }
        ) {
            this.id = id;
            this.fontSize = options.fontSize;
            this.scrollback = options.scrollback;
            this.terminal.options.fontSize = options.fontSize;
            this.sendDataHandler = muxOptions.sendDataHandler;
            termWrapInstances.push(this as any);
        }
        async init() {
            this.loaded = true;
        }
        handleResize() {}
        handleResize_debounced() {}
        dispose() {
            this.disposed = true;
        }
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

/** The status the broker will replay synchronously to the NEXT subscriber for
 *  `blockId` — the `persist: 1` behaviour, not a post-mount emission. */
function queuePersistedStatus(blockId: string, data: Record<string, unknown>) {
    mpsPersisted.set(`controllerstatus|block:${blockId}`, data);
}

/** Emit a `controllerstatus` event to whoever subscribed for `blockId`. */
function emitControllerStatus(blockId: string, data: Record<string, unknown>) {
    for (const handler of mpsHandlers.get(`controllerstatus|block:${blockId}`) ?? []) {
        handler({ data });
    }
}

beforeEach(() => {
    blockDataSignals.clear();
    seedData.clear();
    mpsHandlers.clear();
    mpsPersisted.clear();
    resyncDeferreds.clear();
    resyncRejections.clear();
    termWrapInstances.length = 0;
    for (const key of Object.keys(termSettingsBag)) delete termSettingsBag[key];
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

describe("AgentShellSubblock — scrollback depth", () => {
    /**
     * Regression: the drawer hardcoded `scrollback: 2000` and never read
     * `term:scrollback`, so raising the setting deepened terminal panes while
     * this shell stayed capped. Because xterm counts scrollback in display
     * rows and the drawer is narrow (lines soft-wrap across 2-3 rows), that cap
     * cost far less history than it looks — a long agent session silently lost
     * the top of its own output, and the topmost reachable line was cut
     * mid-line because trimming is per-row.
     */
    it("uses the configured term:scrollback instead of the old hardcoded 2000", async () => {
        termSettingsBag["term:scrollback"] = 40000;

        render(() => (
            <AgentShellSubblock
                parentBlockId="parent-1"
                cwd="/tmp"
                existingSubBlockId={undefined}
                onSubBlockCreated={() => {}}
                agentPaneZoom={() => 1}
            />
        ));

        await waitFor(() => expect(termWrapInstances.length).toBe(1));
        expect(termWrapInstances[0].scrollback).toBe(40000);
    });

    it("falls back to the default depth when term:scrollback is unset", async () => {
        render(() => (
            <AgentShellSubblock
                parentBlockId="parent-1"
                cwd="/tmp"
                existingSubBlockId={undefined}
                onSubBlockCreated={() => {}}
                agentPaneZoom={() => 1}
            />
        ));

        await waitFor(() => expect(termWrapInstances.length).toBe(1));
        expect(termWrapInstances[0].scrollback).toBe(DEFAULT_TERM_SCROLLBACK);
    });

    /**
     * The drawer must pass its sub-block's own meta to the shared resolver, the
     * way term.tsx passes `blockData()?.meta`. Without it a per-block
     * `term:scrollback` override is silently ignored here while working on a
     * terminal pane — the resolver would be shared in name only. Caught by
     * review on #3455.
     */
    it("lets the sub-block's own term:scrollback meta override the global setting", async () => {
        const existingId = "override-sub-block";
        termSettingsBag["term:scrollback"] = 8000;
        queueSeedMeta(`block:${existingId}`, { "term:scrollback": 31000 });

        render(() => (
            <AgentShellSubblock
                parentBlockId="parent-1"
                cwd="/tmp"
                existingSubBlockId={existingId}
                onSubBlockCreated={() => {}}
                agentPaneZoom={() => 1}
            />
        ));

        await new Promise((r) => setTimeout(r, 10));
        resolveSeedFetch(`block:${existingId}`);

        await waitFor(() => expect(termWrapInstances.length).toBe(1));
        expect(termWrapInstances[0].scrollback).toBe(31000);
    });
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
        // created this oref's signal (via getMuxObjectAtom) — confirm it
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
        // network failure that leaves the loading atom stuck (per mos.ts's
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
        await waitFor(() => expect(container.querySelector(".agent-shell-loading-overlay")).toBeNull(), {
            timeout: 1000,
        });
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

describe("AgentShellSubblock — agent lock (SPEC_AGENT_INTERACTIVE_PTY_SHELL_API_2026_09_10.md)", () => {
    it("drops human keystrokes while term:agentlockuntil is in the future", async () => {
        const { sendWSCommand } = await import("@/app/store/ws");
        const existingId = "locked-sub-block";
        const oref = `block:${existingId}`;
        queueSeedMeta(oref, { "term:agentlockuntil": Date.now() + 60_000 });

        render(() => (
            <AgentShellSubblock
                parentBlockId="parent-1"
                cwd="/tmp"
                existingSubBlockId={existingId}
                onSubBlockCreated={() => {}}
                agentPaneZoom={() => 1}
            />
        ));
        resolveSeedFetch(oref);
        await waitFor(() => expect(termWrapInstances.length).toBe(1));

        termWrapInstances[0].sendDataHandler("y");
        expect(sendWSCommand).not.toHaveBeenCalled();
    });

    it("forwards human keystrokes once term:agentlockuntil has passed", async () => {
        const { sendWSCommand } = await import("@/app/store/ws");
        const existingId = "expired-lock-sub-block";
        const oref = `block:${existingId}`;
        // Already in the past — no need to wait for the component's own
        // periodic re-check to observe this; `agentLockedUntil() > nowTick()`
        // is already false the instant it's read.
        queueSeedMeta(oref, { "term:agentlockuntil": Date.now() - 1000 });

        render(() => (
            <AgentShellSubblock
                parentBlockId="parent-1"
                cwd="/tmp"
                existingSubBlockId={existingId}
                onSubBlockCreated={() => {}}
                agentPaneZoom={() => 1}
            />
        ));
        resolveSeedFetch(oref);
        await waitFor(() => expect(termWrapInstances.length).toBe(1));

        termWrapInstances[0].sendDataHandler("y");
        expect(sendWSCommand).toHaveBeenCalledWith(
            expect.objectContaining({ wscommand: "blockinput", blockid: existingId })
        );
    });

    it("shows the agent-lock badge only while locked, un-mounting once a live update clears it", async () => {
        const existingId = "live-unlock-sub-block";
        const oref = `block:${existingId}`;
        queueSeedMeta(oref, { "term:agentlockuntil": Date.now() + 60_000 });

        const { container } = render(() => (
            <AgentShellSubblock
                parentBlockId="parent-1"
                cwd="/tmp"
                existingSubBlockId={existingId}
                onSubBlockCreated={() => {}}
                agentPaneZoom={() => 1}
            />
        ));
        resolveSeedFetch(oref);
        await waitFor(() => expect(termWrapInstances.length).toBe(1));
        expect(container.querySelector(".agent-shell-agentlock-badge")).not.toBeNull();

        // Simulate the backend's own lock-release (PtyShellStop, or the
        // lease simply expiring and getting cleared) landing as a live meta
        // update — same delivery path `term:zoom` live-updates already use.
        const [, set] = blockDataSignals.get(oref)!;
        set({ value: { meta: {} }, loading: false });
        await waitFor(() => expect(container.querySelector(".agent-shell-agentlock-badge")).toBeNull());
    });
});

describe("AgentShellSubblock — shell exit collapses the drawer (SPEC_AGENT_PANE_SHELL_EXIT_COLLAPSES_DRAWER_2026_09_15)", () => {
    /** Mounts against an already-existing sub-block and settles its fetch, so
     *  the component reaches its steady state with a known id subscribed. */
    async function mountAttached(subBlockId: string, onShellExited = vi.fn()) {
        const oref = `block:${subBlockId}`;
        queueSeedMeta(oref, {});
        // A live shell's own current status, replayed on subscribe — the
        // ordinary state of affairs for a drawer open onto a running shell.
        queuePersistedStatus(subBlockId, { shellprocstatus: "running" });
        render(() => (
            <AgentShellSubblock
                parentBlockId="parent-1"
                cwd="/tmp"
                existingSubBlockId={subBlockId}
                onSubBlockCreated={() => {}}
                agentPaneZoom={() => 1}
                onShellExited={onShellExited}
            />
        ));
        resolveSeedFetch(oref);
        await waitFor(() => expect(termWrapInstances.length).toBe(1));
        return { onShellExited };
    }

    it("fires onShellExited when its own shell exits cleanly", async () => {
        const { onShellExited } = await mountAttached("exiting-shell");
        expect(onShellExited).not.toHaveBeenCalled();

        emitControllerStatus("exiting-shell", { shellprocstatus: "done", shellprocexitcode: 0 });

        expect(onShellExited).toHaveBeenCalledTimes(1);
    });

    /** `shellprocexitcode` is `#[serde(default)]` on the wire, so a clean exit
     *  can arrive with the field omitted entirely. That must read as 0, not as
     *  "unknown, assume crash" — otherwise the ordinary `exit` case (the whole
     *  point of the feature) silently does nothing. */
    it("treats an omitted exit code as a clean exit", async () => {
        const { onShellExited } = await mountAttached("omitted-code-shell");
        emitControllerStatus("omitted-code-shell", { shellprocstatus: "done" });
        expect(onShellExited).toHaveBeenCalledTimes(1);
    });

    /** A crash produces the same STATUS_DONE. Collapsing there would hide the
     *  output the human needs to read — spec §5 open question, resolved to
     *  "clean exits only". */
    it("does NOT collapse on a non-zero exit — the failure output must stay readable", async () => {
        const { onShellExited } = await mountAttached("crashing-shell");
        emitControllerStatus("crashing-shell", { shellprocstatus: "done", shellprocexitcode: 137 });
        expect(onShellExited).not.toHaveBeenCalled();
    });

    it("ignores a running status", async () => {
        const { onShellExited } = await mountAttached("running-shell");
        emitControllerStatus("running-shell", { shellprocstatus: "running" });
        expect(onShellExited).not.toHaveBeenCalled();
    });

    /** The agent pane subscribes to `controllerstatus` for its OWN block too
     *  (agent-view.tsx's turn tracking). The drawer must be listening to its
     *  sub-block's scope only — a parent-scope event killing the drawer would
     *  collapse it every time the agent's own process ended. */
    it("ignores a controllerstatus event for a different block", async () => {
        const { onShellExited } = await mountAttached("own-shell");
        emitControllerStatus("parent-1", { shellprocstatus: "done", shellprocexitcode: 0 });
        emitControllerStatus("some-other-block", { shellprocstatus: "done", shellprocexitcode: 0 });
        expect(onShellExited).not.toHaveBeenCalled();
    });

    /** STATUS_DONE can be republished for one block (a status re-publish, a
     *  resync). The parent's handler deletes a sub-block and clears a pointer;
     *  running it twice races the second delete against a fresh shell the
     *  human may have opened in between. */
    it("fires at most once even if the exit is published repeatedly", async () => {
        const { onShellExited } = await mountAttached("repeating-shell");
        emitControllerStatus("repeating-shell", { shellprocstatus: "done", shellprocexitcode: 0 });
        emitControllerStatus("repeating-shell", { shellprocstatus: "done", shellprocexitcode: 0 });
        emitControllerStatus("repeating-shell", { shellprocstatus: "done" });
        expect(onShellExited).toHaveBeenCalledTimes(1);
    });

    it("unsubscribes on unmount", async () => {
        const { onShellExited } = await mountAttached("unmounting-shell");
        cleanup();
        emitControllerStatus("unmounting-shell", { shellprocstatus: "done", shellprocexitcode: 0 });
        expect(onShellExited).not.toHaveBeenCalled();
    });

    /**
     * ReAgent P0 on PR #3253, and the reason this file's mps mock now models
     * replay at all.
     *
     * An agent drives the shared drawer shell (a first-class case, spec §3.3)
     * and it exits cleanly while the drawer is CLOSED. Nothing is mounted, so
     * nothing collapses and `term:shellsubblockid` still points at the dead
     * block. The human then opens the drawer *to read what the agent did* —
     * and the broker replays that persisted `done` synchronously as part of
     * this component's very first subscribe.
     *
     * Acting on it would collapse the drawer they just opened AND delete the
     * sub-block, taking its persisted `term` file with it: the output isn't
     * hidden behind a collapsed drawer (reopenable), it is destroyed. Strictly
     * worse than the bug this feature fixes.
     */
    it("ignores an exit REPLAYED at subscribe time — the shell was already gone before this mount", async () => {
        const onShellExited = vi.fn();
        const subBlockId = "already-exited-shell";
        queueSeedMeta(`block:${subBlockId}`, {});
        queuePersistedStatus(subBlockId, { shellprocstatus: "done", shellprocexitcode: 0 });

        render(() => (
            <AgentShellSubblock
                parentBlockId="parent-1"
                cwd="/tmp"
                existingSubBlockId={subBlockId}
                onSubBlockCreated={() => {}}
                agentPaneZoom={() => 1}
                onShellExited={onShellExited}
            />
        ));
        resolveSeedFetch(`block:${subBlockId}`);
        await waitFor(() => expect(termWrapInstances.length).toBe(1));

        expect(onShellExited).not.toHaveBeenCalled();
    });

    /** The live case must still work when the replay is the shell's RUNNING
     *  status — i.e. a drawer opened onto a healthy shell, which then exits.
     *  Without this, "ignore replays" could be implemented as "ignore the
     *  first event", which would break the ordinary path. */
    it("still collapses when a replayed running status is followed by a live exit", async () => {
        const { onShellExited } = await mountAttached("live-then-exits");
        emitControllerStatus("live-then-exits", { shellprocstatus: "done", shellprocexitcode: 0 });
        expect(onShellExited).toHaveBeenCalledTimes(1);
    });

    /** A shell that starts up and exits entirely within this mount: both
     *  statuses arrive live, no replay involved. */
    it("collapses on an exit that follows a live running status", async () => {
        const onShellExited = vi.fn();
        const subBlockId = "starts-then-exits";
        queueSeedMeta(`block:${subBlockId}`, {});
        render(() => (
            <AgentShellSubblock
                parentBlockId="parent-1"
                cwd="/tmp"
                existingSubBlockId={subBlockId}
                onSubBlockCreated={() => {}}
                agentPaneZoom={() => 1}
                onShellExited={onShellExited}
            />
        ));
        resolveSeedFetch(`block:${subBlockId}`);
        await waitFor(() => expect(termWrapInstances.length).toBe(1));

        emitControllerStatus(subBlockId, { shellprocstatus: "running" });
        emitControllerStatus(subBlockId, { shellprocstatus: "done", shellprocexitcode: 0 });
        expect(onShellExited).toHaveBeenCalledTimes(1);
    });

    /**
     * ReAgent P0 round 2 on PR #3253.
     *
     * The reattach path resyncs with `norespawn: true`, so an already-exited
     * shell fails and falls through to creating a fresh one. Deleting the old
     * block there destroys its persisted `term` file — and when the shell
     * CRASHED while the drawer was closed, that file is the only record of
     * why. The live listener already refuses to collapse on a non-zero exit
     * (spec §5); deleting here anyway reproduces the same data loss through a
     * different door.
     */
    it("does NOT delete a sub-block whose shell crashed, even though reattach failed", async () => {
        const { RpcApi } = await import("@/app/store/rpc-api");
        (RpcApi.ControllerResyncCommand as any).mockRejectedValueOnce(new Error("already exited"));

        const subBlockId = "crashed-while-closed";
        queueSeedMeta(`block:${subBlockId}`, {});
        queuePersistedStatus(subBlockId, { shellprocstatus: "done", shellprocexitcode: 137 });

        render(() => (
            <AgentShellSubblock
                parentBlockId="parent-1"
                cwd="/tmp"
                existingSubBlockId={subBlockId}
                onSubBlockCreated={() => {}}
                agentPaneZoom={() => 1}
            />
        ));
        resolveSeedFetch(`block:${subBlockId}`);
        await waitFor(() => expect(RpcApi.CreateSubBlockCommand).toHaveBeenCalled());

        expect(RpcApi.DeleteSubBlockCommand).not.toHaveBeenCalled();
    });

    /** The clean counterpart: nothing worth keeping, so the dead block goes
     *  rather than lingering until the pane closes. */
    it("deletes a sub-block whose shell exited cleanly before reattach", async () => {
        const { RpcApi } = await import("@/app/store/rpc-api");
        (RpcApi.ControllerResyncCommand as any).mockRejectedValueOnce(new Error("already exited"));

        const subBlockId = "cleanly-exited-while-closed";
        queueSeedMeta(`block:${subBlockId}`, {});
        queuePersistedStatus(subBlockId, { shellprocstatus: "done", shellprocexitcode: 0 });

        render(() => (
            <AgentShellSubblock
                parentBlockId="parent-1"
                cwd="/tmp"
                existingSubBlockId={subBlockId}
                onSubBlockCreated={() => {}}
                agentPaneZoom={() => 1}
            />
        ));
        resolveSeedFetch(`block:${subBlockId}`);
        await waitFor(() => expect(RpcApi.CreateSubBlockCommand).toHaveBeenCalled());

        expect(RpcApi.DeleteSubBlockCommand).toHaveBeenCalledWith(expect.anything(), { blockid: subBlockId });
    });

    /** A genuinely vanished block (gone from the store — no status to replay)
     *  reports no exit at all. Unknown must not be read as clean: skipping the
     *  delete costs nothing here (there is nothing to delete), while treating
     *  unknown as permission is how the crash case above gets destroyed. */
    it("does not delete when no exit status was ever observed", async () => {
        const { RpcApi } = await import("@/app/store/rpc-api");
        (RpcApi.ControllerResyncCommand as any).mockRejectedValueOnce(new Error("block not found"));

        const subBlockId = "vanished-block";
        queueSeedMeta(`block:${subBlockId}`, {});

        render(() => (
            <AgentShellSubblock
                parentBlockId="parent-1"
                cwd="/tmp"
                existingSubBlockId={subBlockId}
                onSubBlockCreated={() => {}}
                agentPaneZoom={() => 1}
            />
        ));
        resolveSeedFetch(`block:${subBlockId}`);
        await waitFor(() => expect(RpcApi.CreateSubBlockCommand).toHaveBeenCalled());

        expect(RpcApi.DeleteSubBlockCommand).not.toHaveBeenCalled();
    });
});

describe("AgentShellSubblock — re-attaches when the parent repoints term:shellsubblockid", () => {
    /**
     * Regression test for the stale-drawer bug found live 2026-09-16
     * investigating a corrupted vim pane
     * (docs/reports/REPORT_VIM_TERMINAL_PANE_HANG_INVESTIGATION_2026_09_16.md):
     * a shell's process exits (e.g. `exit`), the backend's "attach or
     * create" logic spawns a replacement sub-block and repoints the
     * parent agent block's `term:shellsubblockid` meta at it — but the
     * mounted `AgentShellSubblock` (the drawer was never closed) only ever
     * reads `existingSubBlockId` inside `onMount`'s one-shot IIFE, so it
     * stays bound to the OLD, now-dead sub-block forever. The drawer
     * renders the last frame of a shell nobody can reach any more —
     * exactly the "typed `exit`, it hung" symptom, since from the human's
     * side nothing ever changes on screen even though the backend already
     * moved on.
     *
     * This is DISTINCT from SPEC_AGENT_PANE_SHELL_EXIT_COLLAPSES_DRAWER's
     * own-clean-exit handling above (which now collapses the drawer
     * instead of leaving this exact scenario reachable via a human-typed
     * `exit`): that feature deliberately does NOT touch a non-clean exit
     * (a crash) or an exit driven by something other than watching the
     * drawer's OWN subscription — e.g. an agent's shell getting replaced
     * by a backend repoint while the drawer stays mounted, bound to a
     * crashed shell. This suite covers the general "the pointer changed
     * out from under an already-mounted drawer" class of bug.
     *
     * Uses a small reactive harness (a real Solid signal feeding the
     * prop) rather than calling `render` twice — re-rendering the same
     * component instance with a changed prop, the way `agent-view.tsx`'s
     * real `existingSubBlockId={block()?.meta?.[...]}` JSX expression
     * already does reactively, is exactly the scenario this bug lives in.
     */
    it("disposes the old TermWrap and attaches a fresh one when existingSubBlockId changes to a different id", async () => {
        const oldId = "pre-respawn-sub-block";
        const newId = "post-respawn-sub-block";
        queueSeedMeta(`block:${oldId}`, {});
        queueSeedMeta(`block:${newId}`, {});

        const [subBlockId, setSubBlockId] = createSignal(oldId);

        render(() => (
            <AgentShellSubblock
                parentBlockId="parent-1"
                cwd="/tmp"
                existingSubBlockId={subBlockId()}
                onSubBlockCreated={() => {}}
                agentPaneZoom={() => 1}
            />
        ));
        resolveSeedFetch(`block:${oldId}`);
        await waitFor(() => expect(termWrapInstances.length).toBe(1));
        expect(termWrapInstances[0].id).toBe(oldId);
        expect(termWrapInstances[0].disposed).toBe(false);

        // The backend's respawn path repoints the parent's meta at a new
        // sub-block — simulated here as the prop simply changing, the same
        // way the real reactive JSX prop in agent-view.tsx would.
        setSubBlockId(newId);
        // The buggy component never reads the new oref at all (its
        // internal `subBlockId` signal, which subBlockAtom's memo tracks,
        // never updates), so there may be no fetch signal to resolve yet —
        // only resolve one if it actually exists, and let the `waitFor`
        // below fail on the real assertion instead of crashing here.
        await new Promise((r) => setTimeout(r, 10));
        if (blockDataSignals.has(`block:${newId}`)) {
            resolveSeedFetch(`block:${newId}`);
        }

        await waitFor(() => expect(termWrapInstances.length).toBe(2), { timeout: 1000 });
        expect(termWrapInstances[0].disposed).toBe(true);
        expect(termWrapInstances[1].id).toBe(newId);
        expect(termWrapInstances[1].disposed).toBe(false);
    });

    it("does not re-attach when existingSubBlockId is merely echoing back a freshly self-created id", async () => {
        // A freshly created sub-block starts undefined, then the parent
        // feeds the created id back as a prop once its own meta write
        // round-trips — that echo must NOT be treated as an external
        // repoint (it's the same id the component itself just attached
        // to), or every fresh shell would immediately tear itself down
        // and reattach to itself on the very next render.
        const createdId = "self-created-sub-block";
        const { RpcApi } = await import("@/app/store/rpc-api");
        (RpcApi.CreateSubBlockCommand as any).mockResolvedValueOnce(`block:${createdId}`);

        const [subBlockId, setSubBlockId] = createSignal<string | undefined>(undefined);

        render(() => (
            <AgentShellSubblock
                parentBlockId="parent-1"
                cwd="/tmp"
                existingSubBlockId={subBlockId()}
                onSubBlockCreated={(id) => setSubBlockId(id)}
                agentPaneZoom={() => 1}
            />
        ));

        await waitFor(() => expect(termWrapInstances.length).toBe(1));
        expect(termWrapInstances[0].id).toBe(createdId);

        // Let any effect reacting to the prop echo settle, then confirm no
        // second attach happened.
        await new Promise((r) => setTimeout(r, 10));
        expect(termWrapInstances.length).toBe(1);
        expect(termWrapInstances[0].disposed).toBe(false);
    });

    /**
     * ReAgent P1 on PR #3257: `attachShell`'s reuse-existing-id branch
     * (the resync-succeeds path — exactly the respawn scenario this suite
     * targets) never called `setSubBlockId(id)`. TermWrap itself got the
     * right id, but the component's OWN `subBlockId` signal — which
     * `subBlockAtom`/`termZoom`/`agentLocked` and `handleCtrlWheel`'s
     * zoom-write all read — stayed stuck on the OLD, dead sub-block after
     * every reused-id re-attach.
     */
    it("reads the NEW sub-block's own persisted zoom after re-attaching, not the old sub-block's", async () => {
        const oldId = "p1-old-sub-block";
        const newId = "p1-new-sub-block";
        queueSeedMeta(`block:${oldId}`, { "term:zoom": 1.0 }); // fontSize 13
        queueSeedMeta(`block:${newId}`, { "term:zoom": 2.0 }); // fontSize 26

        const [subBlockId, setSubBlockId] = createSignal(oldId);

        render(() => (
            <AgentShellSubblock
                parentBlockId="parent-1"
                cwd="/tmp"
                existingSubBlockId={subBlockId()}
                onSubBlockCreated={() => {}}
                agentPaneZoom={() => 1}
            />
        ));
        resolveSeedFetch(`block:${oldId}`);
        await waitFor(() => expect(termWrapInstances.length).toBe(1));
        expect(termWrapInstances[0].fontSize).toBe(13);

        setSubBlockId(newId);
        await new Promise((r) => setTimeout(r, 10));
        if (blockDataSignals.has(`block:${newId}`)) {
            resolveSeedFetch(`block:${newId}`);
        }
        await waitFor(() => expect(termWrapInstances.length).toBe(2));

        // Under the bug, subBlockAtom still reads block:oldId's meta (zoom
        // 1.0 → fontSize 13) because subBlockId() was never updated on
        // this (reuse-existing-id) path.
        expect(termWrapInstances[1].fontSize).toBe(26);
    });

    /**
     * ReAgent P1 on PR #3257: no generation guard against overlapping
     * `attachShell` calls. If the re-attach effect fires again (another
     * externally-driven repoint) before a prior in-flight `attachShell`
     * finishes its awaits, both invocations run concurrently and
     * whichever resolves LAST unconditionally commits `termWrap`/
     * `resizeObserver` — so an OLDER, slower repoint that happens to
     * resolve AFTER a newer one can silently clobber the newer, correct
     * attachment. Constructs exactly that out-of-order-completion race:
     * id A's repoint fires first but its resync is held open; id B's
     * repoint fires next and resolves FIRST. A resolving afterward must
     * not be allowed to construct (or, if already committed, must
     * dispose) a second, stale TermWrap.
     */
    it("does not let an older repoint's resync, resolving after a newer one, clobber the newer attachment", async () => {
        const initialId = "race-initial-sub-block";
        const idA = "race-id-a";
        const idB = "race-id-b";
        queueSeedMeta(`block:${initialId}`, {});
        queueSeedMeta(`block:${idA}`, {});
        queueSeedMeta(`block:${idB}`, {});

        const [subBlockId, setSubBlockId] = createSignal(initialId);

        render(() => (
            <AgentShellSubblock
                parentBlockId="parent-1"
                cwd="/tmp"
                existingSubBlockId={subBlockId()}
                onSubBlockCreated={() => {}}
                agentPaneZoom={() => 1}
            />
        ));
        resolveSeedFetch(`block:${initialId}`);
        await waitFor(() => expect(termWrapInstances.length).toBe(1));

        // Both repoints' resyncs are held open — A first, then B right
        // after, before A has resolved — simulating two rapid backend
        // repoints landing close together.
        deferResync(idA);
        deferResync(idB);
        setSubBlockId(idA);
        setSubBlockId(idB);

        // B (the newer repoint) resolves FIRST.
        resolveDeferredResync(idB);
        await new Promise((r) => setTimeout(r, 0));
        if (blockDataSignals.has(`block:${idB}`)) resolveSeedFetch(`block:${idB}`);
        await waitFor(() => expect(termWrapInstances.some((t) => t.id === idB)).toBe(true));

        // A (the older, now-stale repoint) resolves SECOND — the
        // out-of-order completion that breaks a naive "last write wins".
        resolveDeferredResync(idA);
        await new Promise((r) => setTimeout(r, 0));
        if (blockDataSignals.has(`block:${idA}`)) resolveSeedFetch(`block:${idA}`);
        // Give A's now-stale attach every chance to (wrongly) proceed.
        await new Promise((r) => setTimeout(r, 20));

        // termWrapInstances accumulates every instance ever constructed
        // (including the initial attach's, already disposed by the first
        // repoint) — the property that actually matters is how many are
        // currently LIVE. Exactly one may be: B's. Under the bug, A
        // proceeds unconditionally after B already committed and its
        // TermWrap (bound to the dead idA) becomes the live one instead.
        const live = termWrapInstances.filter((t) => !t.disposed);
        expect(live.length).toBe(1);
        expect(live[0].id).toBe(idB);
    });

    /**
     * ReAgent P1 on PR #3257, round 2: when attachShell's create-branch
     * (CreateSubBlockCommand) resolves but a NEWER attach has already
     * started (isStale() true), the function bailed immediately without
     * deleting the backend sub-block it just created. Because
     * setSubBlockId/onSubBlockCreated never ran for that id, it's never
     * written to term:shellsubblockid, so nothing else — including
     * agent-view.tsx's own pane-close cleanup, which only knows about
     * whatever that meta currently points at — can ever find it to clean
     * it up. Its backend PTY process leaks for the app's lifetime.
     * Reachable when two externally-driven repoints land close together
     * and BOTH candidate ids fail ControllerResyncCommand, so both take
     * the create-new fallback — distinct from the reuse-existing-id race
     * covered above.
     */
    it("deletes the orphaned backend sub-block when a stale create-branch attempt loses the race", async () => {
        const { RpcApi } = await import("@/app/store/rpc-api");
        const initialId = "orphan-initial-sub-block";
        const idA = "orphan-race-id-a"; // resync fails -> takes the create-fallback, held open
        const idB = "orphan-race-id-b"; // a second, newer repoint that supersedes A while it's stuck creating
        queueSeedMeta(`block:${initialId}`, {});
        queueSeedMeta(`block:${idB}`, {});
        resyncRejections.add(idA);

        let resolveCreateA!: (oref: string) => void;
        (RpcApi.CreateSubBlockCommand as any).mockImplementationOnce(
            () => new Promise<string>((resolve) => (resolveCreateA = resolve))
        );

        const [subBlockId, setSubBlockId] = createSignal(initialId);

        render(() => (
            <AgentShellSubblock
                parentBlockId="parent-1"
                cwd="/tmp"
                existingSubBlockId={subBlockId()}
                onSubBlockCreated={() => {}}
                agentPaneZoom={() => 1}
            />
        ));
        resolveSeedFetch(`block:${initialId}`);
        await waitFor(() => expect(termWrapInstances.length).toBe(1));

        // A's repoint fires — its resync rejects immediately, so it takes
        // the create-fallback and gets stuck there (CreateSubBlockCommand
        // held open by the mockImplementationOnce above).
        setSubBlockId(idA);
        await new Promise((r) => setTimeout(r, 0));

        // A second, newer repoint arrives before A's create call resolves
        // — this must supersede A regardless of A's own eventual outcome.
        // Its resync succeeds instantly (the default mock behavior), so it
        // takes the ordinary reuse path.
        setSubBlockId(idB);
        await new Promise((r) => setTimeout(r, 0));
        if (blockDataSignals.has(`block:${idB}`)) resolveSeedFetch(`block:${idB}`);
        await waitFor(() => expect(termWrapInstances.some((t) => t.id === idB)).toBe(true));

        // A's stale create call finally resolves, having already created a
        // real backend sub-block before discovering it lost the race.
        const createdIdA = "orphan-created-a";
        resolveCreateA(`block:${createdIdA}`);

        // A's own orphaned creation must be deleted — otherwise nothing
        // ever records its id anywhere and its backend PTY leaks forever.
        await waitFor(() =>
            expect(RpcApi.DeleteSubBlockCommand).toHaveBeenCalledWith(
                expect.anything(),
                expect.objectContaining({ blockid: createdIdA })
            )
        );
        // And critically, it must never become the live TermWrap — the
        // orphan-and-forget failure mode ReAgent flagged would otherwise
        // still leave the pane silently attached to it.
        expect(termWrapInstances.some((t) => t.id === createdIdA)).toBe(false);
    });

    /**
     * ReAgent P1 on PR #3257, round 3: `lastObservedExitCode` is a single
     * component-scoped variable, written by the controllerstatus
     * subscription and read by attachShell's resync-failure catch block
     * (SPEC_AGENT_PANE_SHELL_EXIT_COLLAPSES_DRAWER_2026_09_15.md) — but it
     * was never reset when a re-attach switches to a new candidate id.
     * Since this PR is what makes attachShell run more than once per
     * mount, a resync failure for a FRESHLY re-attached id could consult
     * an exit code actually observed for the PREVIOUS sub-block this
     * component was bound to. In the direction this test exercises, that
     * means a resync failure for a brand-new id — whose own exit status
     * was never observed at all — gets wrongly treated as a known-clean
     * exit because the previous, unrelated sub-block happened to exit
     * cleanly, deleting a block whose scrollback might be the only record
     * of why it actually failed. Exactly the "not hidden — gone" data
     * loss this code's own comments say must never happen for an unknown
     * exit.
     */
    it("does not delete a re-attached sub-block's resync failure based on a PREVIOUS sub-block's stale exit code", async () => {
        const { RpcApi } = await import("@/app/store/rpc-api");
        const idA = "stale-exit-code-a";
        const idB = "stale-exit-code-b";
        queueSeedMeta(`block:${idA}`, {});
        // A exits CLEANLY — this sets lastObservedExitCode = 0 as a side
        // effect of being replayed at subscribe time, independent of
        // whether the drawer acts on it (sawRunning is irrelevant to this
        // write — see the subscription handler's own comment).
        queuePersistedStatus(idA, { shellprocstatus: "done", shellprocexitcode: 0 });

        const [subBlockId, setSubBlockId] = createSignal(idA);

        render(() => (
            <AgentShellSubblock
                parentBlockId="parent-1"
                cwd="/tmp"
                existingSubBlockId={subBlockId()}
                onSubBlockCreated={() => {}}
                agentPaneZoom={() => 1}
            />
        ));
        resolveSeedFetch(`block:${idA}`);
        await waitFor(() => expect(termWrapInstances.length).toBe(1));

        // Backend repoints to B — an entirely different, unrelated
        // sub-block whose OWN exit status this component has never
        // observed. B's resync fails (e.g. it already crashed), and
        // nothing has ever published a controllerstatus for B.
        resyncRejections.add(idB);
        setSubBlockId(idB);
        await waitFor(() => expect(RpcApi.CreateSubBlockCommand).toHaveBeenCalled());

        // B's own exit is genuinely unknown to this component — A's
        // stale, unrelated clean exit must not be read as permission to
        // delete B.
        expect(RpcApi.DeleteSubBlockCommand).not.toHaveBeenCalledWith(
            expect.anything(),
            expect.objectContaining({ blockid: idB })
        );
    });
});
