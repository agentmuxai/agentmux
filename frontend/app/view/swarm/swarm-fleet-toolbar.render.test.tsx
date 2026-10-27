// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Select all" reachability and the broadcast/stop button conflict,
 * pinned through the actual `FleetToolbar` component.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/element/confirm-modal", () => ({
    ConfirmModal: () => null,
}));

const menu = vi.hoisted(() => ({ show: vi.fn() }));
vi.mock("@/app/store/contextmenu", () => ({
    ContextMenuModel: { showContextMenu: menu.show },
}));

import { FleetToolbar } from "./swarm-fleet-toolbar";
import type { SwarmViewModel } from "./swarm-model";

afterEach(() => cleanup());

function modelStub(initialSelected: string[] = []) {
    const [selected, setSelected] = createSignal<Set<string>>(new Set(initialSelected));
    const [inFlight] = createSignal(false);

    const model = {
        selectedBlockIdsAtom: selected,
        otherInstancesAtom: () => null,
        fleetActionInFlightAtom: inFlight,
        selectAll: vi.fn((ids: string[]) => setSelected(new Set(ids))),
        clearSelection: vi.fn(() => setSelected(new Set<string>())),
        toggleSelected: () => {},
        isSelected: (id: string) => selected().has(id),
        broadcastToSelection: vi.fn(),
        bulkStopSelection: vi.fn(),
        superviseSelection: vi.fn(),
    } as unknown as SwarmViewModel;

    return model;
}

describe("FleetToolbar — select all", () => {
    it("is reachable with nothing selected yet, and selects every listed agent", () => {
        const model = modelStub([]);
        const { getByText, queryByText } = render(() => (
            <FleetToolbar model={model} allBlockIds={() => ["a", "b", "c"]} />
        ));

        const btn = getByText("Select all");
        fireEvent.click(btn);

        expect(model.selectAll).toHaveBeenCalledWith(["a", "b", "c"]);
        expect(queryByText("3 selected")).not.toBeNull();
    });

    it("reads 'Select none' once everything is selected, and clears on click", () => {
        const model = modelStub(["a", "b"]);
        const { getByText } = render(() => (
            <FleetToolbar model={model} allBlockIds={() => ["a", "b"]} />
        ));

        const btn = getByText("Select none");
        fireEvent.click(btn);

        expect(model.clearSelection).toHaveBeenCalled();
    });

    it("does not render when there are no agents and no selection", () => {
        const model = modelStub([]);
        const { container } = render(() => <FleetToolbar model={model} allBlockIds={() => []} />);
        expect(container.querySelector(".swarm-fleet-toolbar")).toBeNull();
    });
});

describe("FleetToolbar — no Groups or Stats", () => {
    // The Stats panel moved to Warden's Ambient section (warden-ambient-manager.tsx).
    it("has neither a Groups nor a Stats button", () => {
        const { queryByText } = render(() => <FleetToolbar model={modelStub(["a"])} allBlockIds={() => ["a"]} />);
        expect(queryByText(/Groups/)).toBeNull();
        expect(queryByText("Stats")).toBeNull();
    });
});

describe("FleetToolbar — by section", () => {
    it("the Supervisor section has Stop and the mode menu and no Broadcast button", () => {
        const { queryByText } = render(() => <FleetToolbar model={modelStub(["a"])} allBlockIds={() => ["a"]} section="supervisor" />);
        expect(queryByText("Stop 1")).not.toBeNull();
        expect(queryByText("Supervisor")).not.toBeNull();
        expect(queryByText("Broadcast")).toBeNull();
    });

    it("follows the section while it stays mounted", () => {
        const [section, setSection] = createSignal<"supervisor" | "broadcast">("supervisor");
        const { queryByText } = render(() => <FleetToolbar model={modelStub(["a"])} allBlockIds={() => ["a"]} section={section()} />);
        expect(queryByText("Stop 1")).not.toBeNull();
        setSection("broadcast");
        expect(queryByText("Stop 1")).toBeNull();
        expect(queryByText("Supervisor")).toBeNull();
        setSection("supervisor");
        expect(queryByText("Stop 1")).not.toBeNull();
        expect(queryByText("Supervisor")).not.toBeNull();
    });

    it("the Broadcast section keeps the selection and has neither Stop nor the mode menu", () => {
        const { queryByText } = render(() => <FleetToolbar model={modelStub(["a"])} allBlockIds={() => ["a"]} section="broadcast" />);
        expect(queryByText("1 selected")).not.toBeNull();
        expect(queryByText("Select none")).not.toBeNull();
        expect(queryByText("Stop 1")).toBeNull();
        expect(queryByText("Supervisor")).toBeNull();
    });
});

// SPEC_SWARM_SUPERVISOR_2026_10_16.md §3.8.
describe("FleetToolbar — Supervisor", () => {
    it("offers the three modes for the selection and sets the one picked", () => {
        menu.show.mockClear();
        const model = modelStub(["a", "b"]);
        const { getByText } = render(() => <FleetToolbar model={model} allBlockIds={() => ["a", "b"]} />);
        fireEvent.click(getByText("Supervisor"));
        expect(menu.show).toHaveBeenCalledTimes(1);
        const items = menu.show.mock.calls[0][0] as ContextMenuItem[];
        expect(items.map((i) => i.label)).toEqual([
            "Supervisor: Off for 2 agents",
            "Supervisor: Steady for 2 agents",
            "Supervisor: Drive for 2 agents",
        ]);
        items[2].click!();
        expect(model.superviseSelection).toHaveBeenCalledWith("drive");
    });

    it("is not offered with nothing selected", () => {
        const { queryByText } = render(() => <FleetToolbar model={modelStub([])} allBlockIds={() => ["a"]} />);
        expect(queryByText("Supervisor")).toBeNull();
    });
});
