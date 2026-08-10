// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Regression tests for docs/specs/SPEC_AGENT_SHELL_ZOOM_SEED_RACE_2026-08-10.md.
 *
 * Pins the fix: the shell drawer's terminal must be constructed with the
 * FINAL (persisted) font size, not a default followed by a corrective jerk.
 * Mocks RPC/store/TermWrap at the module boundary (same approach as
 * AgentLaunchModal.integration.test.tsx); SUT is the real AgentShellSubblock.
 *
 * Mock design note: `getWaveObjectAtom`'s backing signal for a given oref
 * starts at `null` (unresolved) and is populated ONLY when
 * `reloadWaveObject` actually "fetches" it — mirroring the real WOS
 * relationship, where nothing is available until the async GetObject
 * round-trip completes. This is deliberate: an earlier version of this file
 * pre-seeded the signal synchronously before render, which accidentally made
 * the zoom-value assertion pass against the OLD (buggy) component too — the
 * mock has to reproduce the actual async gap, not just the eventual value,
 * or it doesn't exercise the bug being fixed.
 */

import { cleanup, render, screen, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AgentShellSubblock } from "./AgentShellSubblock";

const { blockSignals, seedData, reloadWaveObjectMock } = vi.hoisted(() => {
    const blockSignals = new Map<string, ReturnType<typeof import("solid-js").createSignal>>();
    const seedData = new Map<string, Record<string, any>>();
    return { blockSignals, seedData, reloadWaveObjectMock: { fn: null as any } };
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

    function getOrCreateSignal(oref: string) {
        let sig = blockSignals.get(oref);
        if (!sig) {
            sig = realCreateSignal<any>(null);
            blockSignals.set(oref, sig);
        }
        return sig;
    }

    const WOS = {
        makeORef: (otype: string, oid: string) => `${otype}:${oid}`,
        // Unresolved (null) until reloadWaveObject "fetches" it — see file
        // header. NOT pre-populated from `seedData` directly; that map is
        // only consulted by reloadWaveObject below.
        getWaveObjectAtom: (oref: string) => {
            const [get] = getOrCreateSignal(oref);
            return get;
        },
        reloadWaveObject: vi.fn(async (oref: string) => {
            const meta = seedData.get(oref);
            const value = meta ? { meta } : null;
            const [, set] = getOrCreateSignal(oref);
            (set as (v: any) => void)(value);
            return value;
        }),
    };
    reloadWaveObjectMock.fn = WOS.reloadWaveObject;

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

/** Configures what `reloadWaveObject(oref)` will resolve with — does NOT
 *  touch the signal directly, so the atom stays unresolved until something
 *  actually awaits the fetch (matching real WOS timing). */
function queueSeedMeta(oref: string, meta: Record<string, any>) {
    seedData.set(oref, meta);
}

beforeEach(() => {
    blockSignals.clear();
    seedData.clear();
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
        // Persisted zoom of 2.0 → BASE_FONT_SIZE(13) * 2.0 / paneZoom(1) = 26.
        // Only queued, not yet "fetched" — reloadWaveObject must run for the
        // component to ever see this value.
        queueSeedMeta(`block:${existingId}`, { "term:zoom": 2.0 });

        render(() => (
            <AgentShellSubblock
                parentBlockId="parent-1"
                cwd="/tmp"
                existingSubBlockId={existingId}
                onSubBlockCreated={() => {}}
                agentPaneZoom={() => 1}
            />
        ));

        await waitFor(() => expect(termWrapInstances.length).toBe(1));

        // The bug this spec fixes: TermWrap used to be constructed BEFORE the
        // meta fetch resolved, so it always got BASE_FONT_SIZE (13) on this
        // path and had to be corrected afterward (the visible jerk).
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

        await waitFor(() => expect(termWrapInstances.length).toBe(1));
        expect(termWrapInstances[0].fontSize).toBe(13);
    });

    it("shows the loading overlay until the zoom seed resolves, then hides it", async () => {
        const existingId = "slow-sub-block";
        const oref = `block:${existingId}`;
        queueSeedMeta(oref, { "term:zoom": 1.5 });

        // Hold the seed fetch open until the test explicitly resolves it, so
        // the overlay's initial "still loading" state is observable.
        let resolveFetch: () => void;
        const fetchGate = new Promise<void>((res) => (resolveFetch = res));
        reloadWaveObjectMock.fn.mockImplementationOnce(async (o: string) => {
            await fetchGate;
            const meta = seedData.get(o);
            const value = meta ? { meta } : null;
            const [, set] = blockSignals.get(o) ?? createSignal<any>(null);
            (set as (v: any) => void)(value);
            return value;
        });

        const { container } = render(() => (
            <AgentShellSubblock
                parentBlockId="parent-1"
                cwd="/tmp"
                existingSubBlockId={existingId}
                onSubBlockCreated={() => {}}
                agentPaneZoom={() => 1}
            />
        ));

        // Still seeding — overlay present, terminal not constructed yet.
        expect(container.querySelector(".agent-shell-loading-overlay")).not.toBeNull();
        expect(termWrapInstances.length).toBe(0);

        resolveFetch!();

        await waitFor(() => expect(termWrapInstances.length).toBe(1));
        // fontSize should already be correct once the terminal exists at all.
        expect(termWrapInstances[0].fontSize).toBe(round(13 * 1.5));
    });

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
});

function round(n: number): number {
    return Math.round(n);
}
