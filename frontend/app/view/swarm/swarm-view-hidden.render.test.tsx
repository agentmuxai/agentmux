// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A Swarm pane nobody can see doesn't rebuild its list on every agent event: it keeps the
 * last one and rebuilds once when it is shown. Every Swarm pane in a window used to rebuild
 * on every event, the ones in other window tabs included. A pane that can be seen rebuilds
 * at most once a frame, however many events the frame holds (a broadcast starting a turn on
 * many agents used to rebuild the list once per event).
 */
import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: () => {} }));
vi.mock("@/app/store/pane-content-holds", () => ({
    trackPaneContent: () => () => {},
    holdPaneContent: () => () => {},
}));

import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";
import type { SwarmViewModel } from "./swarm-model";
import { SwarmView } from "./swarm-root";

/** Animation frames the test fires by hand. */
let queued = new Map<number, () => void>();
let nextFrame = 1;
function frame(): void {
    const runs = [...queued.values()];
    queued = new Map();
    runs.forEach((run) => run());
}

beforeEach(() => {
    queued = new Map();
    nextFrame = 1;
    vi.stubGlobal("requestAnimationFrame", (run: () => void) => {
        const id = nextFrame++;
        queued.set(id, run);
        return id;
    });
    vi.stubGlobal("cancelAnimationFrame", (id: number) => void queued.delete(id));
});

afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
});

function setup(initial: "active" | "windowHidden" | "dormant") {
    const [source, setSource] = createSignal(0);
    const [visibility, setVisibility] = createSignal<"active" | "windowHidden" | "dormant">(initial);
    const built = vi.fn();
    const model = {
        blockId: "swarm-1",
        loadingAtom: () => false,
        buildTree: () => {
            source();
            built();
            return [];
        },
        otherInstancesAtom: () => null,
        pruneSelection: () => {},
        selectedBlockIdsAtom: () => new Set<string>(),
        isSelected: () => false,
        toggleSelected: () => {},
        setManySelected: () => {},
        fleetActionInFlightAtom: () => false,
        lastFleetResultAtom: () => null,
        dismissFleetResult: () => {},
        clearSelection: () => {},
        selectAll: () => {},
    } as unknown as SwarmViewModel;
    const ctx = { meta: () => ({}), setMeta: async () => {}, visibility } as unknown as PaneTabHostContext;
    render(() => <SwarmView model={model} ctx={ctx} />);
    return { built, setSource, setVisibility };
}

describe("a Swarm pane's list while it can't be seen", () => {
    it("rebuilds on an agent event when shown, in the next frame", () => {
        const { built, setSource } = setup("active");
        frame();
        const before = built.mock.calls.length;
        setSource(1);
        expect(built.mock.calls.length).toBe(before);
        frame();
        expect(built.mock.calls.length).toBe(before + 1);
    });

    it.each(["windowHidden", "dormant"] as const)("does not rebuild on an event while %s, and rebuilds once when shown", (hidden) => {
        const { built, setSource, setVisibility } = setup(hidden);
        const before = built.mock.calls.length;
        setSource(1);
        setSource(2);
        setSource(3);
        frame();
        expect(built.mock.calls.length).toBe(before);
        setVisibility("active");
        frame();
        expect(built.mock.calls.length).toBe(before + 1);
        setSource(4);
        frame();
        expect(built.mock.calls.length).toBe(before + 2);
    });
});

describe("a Swarm pane's list while it is shown", () => {
    it("rebuilds once for a burst of events in one frame, not once per event", () => {
        const { built, setSource } = setup("active");
        frame();
        const before = built.mock.calls.length;
        for (let event = 1; event <= 40; event++) setSource(event);
        expect(built.mock.calls.length).toBe(before);
        frame();
        expect(built.mock.calls.length).toBe(before + 1);
    });

    it("rebuilds again for the next frame's events", () => {
        const { built, setSource } = setup("active");
        frame();
        const before = built.mock.calls.length;
        setSource(1);
        setSource(2);
        frame();
        setSource(3);
        setSource(4);
        frame();
        expect(built.mock.calls.length).toBe(before + 2);
    });
});
