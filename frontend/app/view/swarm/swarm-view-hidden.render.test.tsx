// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A Swarm pane nobody can see doesn't rebuild its list on every agent event: it keeps the
 * last one and rebuilds once when it is shown. Every Swarm pane in a window used to rebuild
 * on every event, the ones in other window tabs included.
 */
import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: () => {} }));
vi.mock("@/app/store/pane-content-holds", () => ({
    trackPaneContent: () => () => {},
    holdPaneContent: () => () => {},
}));

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import type { SwarmViewModel } from "./swarm-model";
import { SwarmView } from "./swarm-root";

afterEach(() => cleanup());

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
        agentRuntime: { rows: () => [] },
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
    it("rebuilds on an agent event when shown", () => {
        const { built, setSource } = setup("active");
        const before = built.mock.calls.length;
        setSource(1);
        expect(built.mock.calls.length).toBe(before + 1);
    });

    it.each(["windowHidden", "dormant"] as const)("does not rebuild on an event while %s, and rebuilds once when shown", (hidden) => {
        const { built, setSource, setVisibility } = setup(hidden);
        const before = built.mock.calls.length;
        setSource(1);
        setSource(2);
        setSource(3);
        expect(built.mock.calls.length).toBe(before);
        setVisibility("active");
        expect(built.mock.calls.length).toBe(before + 1);
        setSource(4);
        expect(built.mock.calls.length).toBe(before + 2);
    });
});
