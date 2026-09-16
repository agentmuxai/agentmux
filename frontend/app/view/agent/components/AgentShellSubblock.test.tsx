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
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AgentShellSubblock } from "./AgentShellSubblock";

const { blockDataSignals, seedData, wpsHandlers, wpsPersisted } = vi.hoisted(() => {
    const blockDataSignals = new Map<string, ReturnType<typeof import("solid-js").createSignal<any>>>();
    const seedData = new Map<string, Record<string, any>>();
    const wpsHandlers = new Map<string, Array<(event: any) => void>>();
    const wpsPersisted = new Map<string, Record<string, unknown>>();
    return { blockDataSignals, seedData, wpsHandlers, wpsPersisted };
});

// Records subscriptions by "<eventType>|<scope>" so a test can emit to ONE
// scope and prove the component isn't listening to everything. Honours
// unsubscribe, so the "stops listening on unmount" behaviour is observable
// rather than assumed.
//
// Crucially it also models `persist: 1` REPLAY: `controllerstatus` is
// published persisted specifically so a new subscriber is handed the current
// status synchronously as part of subscribing (wps.ts's `replay_to_route`).
// The first version of this mock only delivered events a test emitted AFTER
// mount, which made the replay path — where ReAgent found a P0 — structurally
// invisible to all 12 tests. `queuePersistedStatus` puts an event in that
// replay slot instead.
vi.mock("@/app/store/wps", () => ({
    waveEventSubscribe: (opts: { eventType: string; scope: string; handler: (event: any) => void }) => {
        const key = `${opts.eventType}|${opts.scope}`;
        wpsHandlers.set(key, [...(wpsHandlers.get(key) ?? []), opts.handler]);
        const persisted = wpsPersisted.get(key);
        if (persisted !== undefined) {
            // Synchronously, inside subscribe — exactly how the broker does it.
            opts.handler({ data: persisted });
        }
        return () => {
            wpsHandlers.set(key, (wpsHandlers.get(key) ?? []).filter((h) => h !== opts.handler));
        };
    },
}));

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
// assert on WHAT it was constructed with (specifically: fontSize and, for
// the agent-lock tests, the sendDataHandler closure), not on real terminal
// rendering.
const termWrapInstances: Array<{ fontSize: number; loaded: boolean; terminal: any; sendDataHandler: (data: string) => void }> = [];

vi.mock("@/app/view/term/termwrap", () => {
    class FakeTermWrap {
        fontSize: number;
        terminal = { options: { fontSize: 0 } };
        loaded = false;
        sendDataHandler: (data: string) => void;
        constructor(
            _id: string,
            _container: HTMLElement,
            options: { fontSize: number },
            waveOptions: { sendDataHandler: (data: string) => void }
        ) {
            this.fontSize = options.fontSize;
            this.terminal.options.fontSize = options.fontSize;
            this.sendDataHandler = waveOptions.sendDataHandler;
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

/** The status the broker will replay synchronously to the NEXT subscriber for
 *  `blockId` — the `persist: 1` behaviour, not a post-mount emission. */
function queuePersistedStatus(blockId: string, data: Record<string, unknown>) {
    wpsPersisted.set(`controllerstatus|block:${blockId}`, data);
}

/** Emit a `controllerstatus` event to whoever subscribed for `blockId`. */
function emitControllerStatus(blockId: string, data: Record<string, unknown>) {
    for (const handler of wpsHandlers.get(`controllerstatus|block:${blockId}`) ?? []) {
        handler({ data });
    }
}

beforeEach(() => {
    blockDataSignals.clear();
    seedData.clear();
    wpsHandlers.clear();
    wpsPersisted.clear();
    termWrapInstances.length = 0;
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
     * ReAgent P0 on PR #3253, and the reason this file's wps mock now models
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
});
