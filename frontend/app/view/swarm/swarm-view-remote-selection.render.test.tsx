// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The whole Swarm view when this instance has no agents of its own but other
 * instances do: their agents can be selected, so the fleet toolbar must be there
 * to act on the selection. It used to mount only inside the branch that renders
 * this instance's tree, so a selection led nowhere.
 * docs/specs/SPEC_SWARM_REMOTE_AGENTS_PLATFORM_TAG_AND_SELECTION_2026_10_03.md §4.2.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { AmbientOutcomesCommand: () => Promise.resolve({}) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: () => {} }));
vi.mock("@/app/store/pane-content-holds", () => ({
    trackPaneContent: () => () => {},
    holdPaneContent: () => () => {},
}));

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import type { SwarmViewModel } from "./swarm-model";
import type { SwarmOtherInstances } from "./swarm-remote";
import { SwarmView } from "./swarm-view";

afterEach(cleanup);

const others: SwarmOtherInstances = {
    hostname: "narko",
    channel: "stable",
    hosts: [
        {
            host_id: "lan:area54#1",
            display_name: "Area54",
            tier: "lan",
            channels: [{ channel: "stable", seen_at_ms: Date.now(), stale: false, os: "macos", agents: [{ name: "Manoz", block_id: "" }] }],
        },
    ],
};

function setup() {
    const [selected, setSelected] = createSignal<Set<string>>(new Set());
    const model = {
        blockId: "swarm-1",
        loadingAtom: () => false,
        buildTree: () => [],
        otherInstancesAtom: () => others,
        isRemoteCollapsed: () => false,
        toggleRemoteCollapsed: () => {},
        selectedBlockIdsAtom: selected,
        isSelected: (k: string) => selected().has(k),
        toggleSelected: (k: string) =>
            setSelected((prev) => {
                const next = new Set(prev);
                if (next.has(k)) next.delete(k);
                else next.add(k);
                return next;
            }),
        setManySelected: () => {},
        fleetGroupsAtom: () => [],
        fleetActionInFlightAtom: () => false,
        lastFleetResultAtom: () => null,
        dismissFleetResult: () => {},
        clearSelection: () => setSelected(new Set<string>()),
        selectAll: () => {},
    } as unknown as SwarmViewModel;
    const ctx = { meta: () => ({}), setMeta: async () => {} } as unknown as PaneTabHostContext;
    return render(() => <SwarmView model={model} ctx={ctx} />);
}

describe("SwarmView with no agents here and agents on another machine", () => {
    it("shows no toolbar until something is selected", () => {
        const { container } = setup();
        expect(container.querySelector(".swarm-empty-title")?.textContent).toBe("No active agent panes");
        expect(container.querySelector(".swarm-fleet-toolbar")).toBeNull();
    });

    it("shows the toolbar once a remote agent is selected, so there is something to act with", () => {
        const { container } = setup();
        fireEvent.click(container.querySelector<HTMLInputElement>(".swarm-remote-agent input[type=checkbox]")!);
        const count = container.querySelector(".swarm-fleet-toolbar-count");
        expect(count?.textContent?.replace(/\s+/g, " ").trim()).toBe("1 selected · 1 on other machines");
        // This instance's empty state is still said.
        expect(container.querySelector(".swarm-empty-title")).not.toBeNull();
    });
});
