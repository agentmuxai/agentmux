// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The fleet toolbar with agents of other instances selected: the count says
 * where they are, an action counts only what it can reach, the stop
 * confirmation names every target and why one can't be reached, results use
 * those names.
 * docs/specs/SPEC_SWARM_REMOTE_AGENTS_PLATFORM_TAG_AND_SELECTION_2026_10_03.md §4, §5.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/element/confirm-modal", () => ({ ConfirmModal: () => null }));

import { FleetResultPanel, FleetToolbar, StopTargetRows } from "./swarm-fleet-toolbar";
import { localFleetTargets, remoteFleetTargets } from "./swarm-fleet-targets";
import type { FleetResultEntry, SwarmViewModel } from "./swarm-model";
import { remoteSections, type SwarmOtherInstances } from "./swarm-remote";

afterEach(() => cleanup());

const LOAP = "blk-loap";
const MANOZ = "remote:lan:area54#1/stable/Manoz";

const others: SwarmOtherInstances = {
    hostname: "narko",
    channel: "stable",
    hosts: [
        {
            host_id: "host:narko",
            display_name: "narko",
            tier: "host",
            channels: [{ channel: "dev", seen_at_ms: 1, stale: false, os: "windows", agents: [{ name: "Loap", block_id: LOAP }] }],
        },
        {
            host_id: "lan:area54#1",
            display_name: "Area54",
            tier: "lan",
            channels: [{ channel: "stable", seen_at_ms: 1, stale: false, os: "macos", agents: [{ name: "Manoz", block_id: "" }] }],
        },
    ],
};

function modelStub(selectedKeys: string[]) {
    const [selected] = createSignal<Set<string>>(new Set(selectedKeys));
    const known = new Map([
        ...localFleetTargets([{ blockId: "blk-korp", agentName: "Korp" }]),
        ...remoteFleetTargets(remoteSections(others)),
    ]);
    return {
        selectedBlockIdsAtom: selected,
        otherInstancesAtom: () => others,
        fleetTargets: () => known,
        ambientOutcomesAtom: () => null,
        statsOpenAtom: () => false,
        fleetActionInFlightAtom: () => false,
        selectAll: vi.fn(),
        clearSelection: vi.fn(),
        toggleSelected: () => {},
        isSelected: (k: string) => selected().has(k),
        broadcastToSelection: vi.fn(),
        bulkStopSelection: vi.fn(),
        lastFleetResultAtom: () => null,
        dismissFleetResult: () => {},
    } as unknown as SwarmViewModel;
}

const toolbar = (model: SwarmViewModel) => render(() => <FleetToolbar model={model} allBlockIds={() => ["blk-korp"]} />);

describe("FleetToolbar with agents of other instances selected", () => {
    it("says how many of the selected are on other machines, not counting this machine's other channels", () => {
        toolbar(modelStub(["blk-korp", LOAP, MANOZ]));
        expect(document.querySelector(".swarm-fleet-toolbar-count")!.textContent!.replace(/\s+/g, " ").trim()).toBe(
            "3 selected · 1 on other machines"
        );
    });

    it("shows a bare count when the others selected are on this machine", () => {
        toolbar(modelStub(["blk-korp", LOAP]));
        expect(document.querySelector(".swarm-fleet-toolbar-count")!.textContent!.trim()).toBe("2 selected");
    });

    it("shows a bare count when everything selected is here", () => {
        toolbar(modelStub(["blk-korp"]));
        expect(document.querySelector(".swarm-fleet-toolbar-count")!.textContent!.trim()).toBe("1 selected");
    });

    it("counts only the agents a stop can reach", () => {
        toolbar(modelStub(["blk-korp", LOAP, MANOZ]));
        expect(screen.getByText(/Stop 2/)).toBeTruthy();
    });

    it("can't stop or broadcast when everything selected is on another machine, and says why", () => {
        toolbar(modelStub([MANOZ]));
        const stop = screen.getByText(/Stop 0/).closest("button")!;
        const broadcast = screen.getByText("Broadcast").closest("button")!;
        expect(stop.disabled).toBe(true);
        expect(broadcast.disabled).toBe(true);
        expect(stop.title).toMatch(/other machines/);
    });

    it("leaves Broadcast and Stop enabled when something selected can be reached", () => {
        toolbar(modelStub([MANOZ, LOAP]));
        expect((screen.getByText("Broadcast").closest("button") as HTMLButtonElement).disabled).toBe(false);
        expect((screen.getByText(/Stop 1/).closest("button") as HTMLButtonElement).disabled).toBe(false);
    });
});

describe("StopTargetRows", () => {
    const rows = (keys: string[]) => {
        render(() => <StopTargetRows model={modelStub(keys)} />);
        return [...document.querySelectorAll(".swarm-fleet-confirm-target-row")];
    };
    // The spans sit side by side with CSS gaps, so join them as a reader sees them.
    const text = (el: Element) => [...el.children].map((c) => c.textContent).join(" ");

    it("names an agent here by its name, not its block id", () => {
        expect(rows(["blk-korp"]).map(text)).toEqual(["Korp"]);
    });

    it("names an agent on another channel of this machine with its machine, platform and how it was found", () => {
        expect(rows([LOAP]).map(text)).toEqual(["Loap narko · dev Windows this machine"]);
    });

    it("lists an agent on another machine with why it can't be stopped, marked as left out", () => {
        const [row] = rows([MANOZ]);
        expect(text(row)).toMatch(/^Manoz Area54 macOS LAN can't stop: .*verified link/);
        expect(row.classList.contains("swarm-fleet-confirm-target-row--unavailable")).toBe(true);
    });

    it("lists every selected agent, reachable or not", () => {
        expect(rows(["blk-korp", LOAP, MANOZ])).toHaveLength(3);
    });
});

describe("FleetResultPanel", () => {
    it("names each target with its machine, and never prints a key", () => {
        const entry: FleetResultEntry = {
            action: "broadcast",
            result: { succeeded: [LOAP], failed: [{ id: MANOZ, error: "can't message: no verified link to other machines yet" }], aborted_early: false },
            labels: { [LOAP]: "Loap · narko · dev", [MANOZ]: "Manoz · Area54" },
        };
        const model = { lastFleetResultAtom: () => entry, dismissFleetResult: () => {} } as unknown as SwarmViewModel;
        render(() => <FleetResultPanel model={model} />);
        const rows = [...document.querySelectorAll(".swarm-fleet-result-row")].map((r) => r.textContent!.replace(/\s+/g, " ").trim());
        expect(rows).toEqual(["Loap · narko · dev", "Manoz · Area54 — can't message: no verified link to other machines yet"]);
        expect(document.body.textContent).not.toContain("remote:lan");
    });

    it("falls back to the id for a target with no label", () => {
        const entry: FleetResultEntry = {
            action: "bulk-stop",
            result: { succeeded: ["blk-x"], failed: [], aborted_early: false },
            labels: {},
        };
        const model = { lastFleetResultAtom: () => entry, dismissFleetResult: () => {} } as unknown as SwarmViewModel;
        render(() => <FleetResultPanel model={model} />);
        expect(document.querySelector(".swarm-fleet-result-row")!.textContent).toContain("blk-x");
    });
});
