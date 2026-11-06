// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Supervise section's actions (Stop, and the mode menu), pinned through the actual
 * `SuperviseActions` component. How many are selected is beside Select all (swarm-select-toggle).
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/element/confirm-modal", () => ({
    ConfirmModal: () => null,
}));

const menu = vi.hoisted(() => ({ show: vi.fn() }));
vi.mock("@/app/element/ui/open-menu-at", async (original) => ({ ...(await original<typeof import("@/app/element/ui/open-menu-at")>()), openMenuAt: menu.show }));

import { SuperviseActions } from "./swarm-fleet-toolbar";
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

describe("SuperviseActions", () => {
    it("has Stop and the mode menu, and no count, Clear, Groups, Stats or Broadcast", () => {
        const { queryByText } = render(() => <SuperviseActions model={modelStub(["a"])} allBlockIds={() => ["a"]} />);
        expect(queryByText("Stop 1")).not.toBeNull();
        expect(queryByText("Mode")).not.toBeNull();
        expect(queryByText("1 selected")).toBeNull();
        expect(queryByText("Clear")).toBeNull();
        expect(queryByText(/Groups/)).toBeNull();
        expect(queryByText("Stats")).toBeNull();
        expect(queryByText("Broadcast")).toBeNull();
    });

    it("is there with nothing selected, with both off and saying to select agents", () => {
        const { getByText } = render(() => <SuperviseActions model={modelStub([])} allBlockIds={() => ["a"]} />);
        const stop = getByText(/Stop 0/).closest("button")!;
        const mode = getByText("Mode").closest("button")!;
        expect(stop.disabled).toBe(true);
        expect(mode.disabled).toBe(true);
        expect(stop.title).toBe("Select agents to stop");
        expect(mode.title).toBe("Select agents to supervise");
    });

    // SPEC_SWARM_SUPERVISOR_2026_10_16.md §3.8.
    it("offers the three modes for the selection and sets the one picked", () => {
        menu.show.mockClear();
        const model = modelStub(["a", "b"]);
        const { getByText } = render(() => <SuperviseActions model={model} allBlockIds={() => ["a", "b"]} />);
        fireEvent.click(getByText("Mode"));
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
});
