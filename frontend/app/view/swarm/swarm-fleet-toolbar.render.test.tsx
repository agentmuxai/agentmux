// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Select all" and Stats reachability and the broadcast/stop button conflict,
 * pinned through the actual `FleetToolbar` component.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/element/confirm-modal", () => ({
    ConfirmModal: () => null,
}));

import { FleetToolbar } from "./swarm-fleet-toolbar";
import type { SwarmViewModel } from "./swarm-model";
import type { AmbientOutcomes } from "@/app/store/ambient-outcomes";

afterEach(() => cleanup());

function modelStub(initialSelected: string[] = [], outcomes: AmbientOutcomes | null = null) {
    const [selected, setSelected] = createSignal<Set<string>>(new Set(initialSelected));
    const [statsOpen, setStatsOpen] = createSignal(false);
    const [inFlight] = createSignal(false);

    const model = {
        selectedBlockIdsAtom: selected,
        otherInstancesAtom: () => null,
        ambientOutcomesAtom: () => outcomes,
        statsOpenAtom: statsOpen,
        toggleStats: vi.fn(() => setStatsOpen(!statsOpen())),
        fleetActionInFlightAtom: inFlight,
        selectAll: vi.fn((ids: string[]) => setSelected(new Set(ids))),
        clearSelection: vi.fn(() => setSelected(new Set<string>())),
        toggleSelected: () => {},
        isSelected: (id: string) => selected().has(id),
        broadcastToSelection: vi.fn(),
        bulkStopSelection: vi.fn(),
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

    it("does not render when there are no agents, no selection and no stats", () => {
        const model = modelStub([]);
        const { container } = render(() => <FleetToolbar model={model} allBlockIds={() => []} />);
        expect(container.querySelector(".swarm-fleet-toolbar")).toBeNull();
    });
});

describe("FleetToolbar — Stats", () => {
    it("has no Groups button any more", () => {
        const { queryByText } = render(() => <FleetToolbar model={modelStub(["a"])} allBlockIds={() => ["a"]} />);
        expect(queryByText(/Groups/)).toBeNull();
    });

    it("shows Stats once there are counts, even with no agents here, and toggles the panel", () => {
        const model = modelStub([], { activity_summary: { accepted: 3 } });
        const { getByText, container } = render(() => <FleetToolbar model={model} allBlockIds={() => []} />);
        const stats = getByText("Stats").closest("button")!;
        expect(stats.getAttribute("aria-expanded")).toBe("false");
        fireEvent.click(stats);
        expect(model.toggleStats).toHaveBeenCalled();
        expect(stats.getAttribute("aria-expanded")).toBe("true");
        expect(stats.getAttribute("aria-pressed")).toBe("true");
        expect(container.querySelector(".swarm-stats-failing")).toBeNull();
    });

    it("says how many kinds of call are failing", () => {
        const model = modelStub([], { activity_summary_pushed: { accepted: 1, cli_failed: 4 } });
        const { getByText } = render(() => <FleetToolbar model={model} allBlockIds={() => []} />);
        expect(getByText("1 failing")).not.toBeNull();
    });

    it("has no Stats button before any call has ended", () => {
        const { queryByText } = render(() => <FleetToolbar model={modelStub(["a"], {})} allBlockIds={() => ["a"]} />);
        expect(queryByText("Stats")).toBeNull();
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
