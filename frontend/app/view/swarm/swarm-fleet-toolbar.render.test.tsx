// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Select all" reachability and the broadcast/stop button conflict, pinned
 * through the actual `FleetToolbar` component.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/element/confirm-modal", () => ({
    ConfirmModal: () => null,
}));

import { FleetToolbar } from "./swarm-fleet-toolbar";
import type { SwarmViewModel } from "./swarm-model";
import type { FleetGroup } from "@/app/store/rpc-api";

afterEach(() => cleanup());

function modelStub(initialSelected: string[] = []) {
    const [selected, setSelected] = createSignal<Set<string>>(new Set(initialSelected));
    const [groups] = createSignal<FleetGroup[]>([]);
    const [inFlight] = createSignal(false);

    const model = {
        selectedBlockIdsAtom: selected,
        fleetGroupsAtom: groups,
        fleetActionInFlightAtom: inFlight,
        selectAll: vi.fn((ids: string[]) => setSelected(new Set(ids))),
        clearSelection: vi.fn(() => setSelected(new Set<string>())),
        toggleSelected: () => {},
        isSelected: (id: string) => selected().has(id),
        broadcastToSelection: vi.fn(),
        bulkStopSelection: vi.fn(),
        saveSelectionAsGroup: vi.fn(),
        applyGroupAsSelection: () => {},
        deleteFleetGroup: vi.fn(),
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

    it("does not render when there are no agents and no selection and no saved groups", () => {
        const model = modelStub([]);
        const { container } = render(() => <FleetToolbar model={model} allBlockIds={() => []} />);
        expect(container.querySelector(".swarm-fleet-toolbar")).toBeNull();
    });
});

describe("FleetToolbar — groups dropdown", () => {
    it("opens without throwing (portaled + floating-ui positioned)", async () => {
        const model = modelStub(["a"]);
        const { getByText } = render(() => (
            <FleetToolbar model={model} allBlockIds={() => ["a"]} />
        ));

        fireEvent.click(getByText("Groups"));
        // computeMenuPosition resolves asynchronously, and the dropdown is
        // portaled to document.body (not a descendant of the render
        // container) — `screen` queries the whole document, unlike the
        // container-scoped queries `render()` returns.
        expect(await screen.findByText("No saved groups yet")).not.toBeNull();
    });
});

describe("FleetToolbar — broadcast composer vs. Stop button", () => {
    it("hides the Stop button while the broadcast composer is open", () => {
        const model = modelStub(["a"]);
        const { getByText, queryByText } = render(() => (
            <FleetToolbar model={model} allBlockIds={() => ["a"]} />
        ));

        expect(queryByText("Stop 1")).not.toBeNull();

        fireEvent.click(getByText("Broadcast"));

        expect(queryByText("Stop 1")).toBeNull();
        expect(queryByText("Cancel")).not.toBeNull();
    });

    it("restores the Stop button once the broadcast composer is cancelled", () => {
        const model = modelStub(["a"]);
        const { getByText, queryByText } = render(() => (
            <FleetToolbar model={model} allBlockIds={() => ["a"]} />
        ));

        fireEvent.click(getByText("Broadcast"));
        fireEvent.click(getByText("Cancel"));

        expect(queryByText("Stop 1")).not.toBeNull();
    });
});
