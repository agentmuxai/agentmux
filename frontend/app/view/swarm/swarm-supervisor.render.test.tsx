// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Supervisor's chip and decision list on a Swarm agent row.
 * docs/specs/SPEC_SWARM_SUPERVISOR_2026_10_16.md §3.8.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { SupervisorState } from "@/types/rpc/SupervisorState";

const menu = vi.hoisted(() => ({ show: vi.fn() }));
vi.mock("@/app/store/contextmenu", () => ({
    ContextMenuModel: { showContextMenu: menu.show },
}));

import { SupervisorBucket, SupervisorChip, supervisorMenuItems, supervisorRows } from "./swarm-supervisor";

afterEach(() => {
    cleanup();
    menu.show.mockClear();
});

const state = (over: Partial<SupervisorState>): SupervisorState => ({
    mode: "drive",
    status: "watching",
    continuations: 0,
    limit: 15,
    run_started_at_ms: 0,
    no_progress: 0,
    updated_at_ms: 0,
    history: [],
    ...over,
});

const claude = (over: Record<string, unknown> = {}) => ({ agentProvider: "claude", ...over });

describe("SupervisorChip", () => {
    it("is only on a Claude agent's row", () => {
        const { container } = render(() => <SupervisorChip meta={{ agentProvider: "codex" }} live={false} onSet={vi.fn()} />);
        expect(container.querySelector(".swarm-supervisor-chip")).toBeNull();
    });

    it("off: the shield alone; on: the mode and what it is doing", () => {
        const off = render(() => <SupervisorChip meta={claude()} live={false} onSet={vi.fn()} />);
        const offChip = off.container.querySelector(".swarm-supervisor-chip")!;
        expect(offChip.classList.contains("swarm-supervisor-chip--off")).toBe(true);
        expect(offChip.textContent).toBe("");
        cleanup();

        const meta = claude({
            "supervisor:mode": "drive",
            "supervisor:state": state({ status: "continued", continuations: 3 }),
        });
        const on = render(() => <SupervisorChip meta={meta} live={true} onSet={vi.fn()} />);
        expect(on.container.querySelector(".swarm-supervisor-chip")!.textContent).toBe("Drive3/15");
    });

    it("marks a pause, which is what asks something of the user", () => {
        const meta = claude({
            "supervisor:mode": "steady",
            "supervisor:state": state({ mode: "steady", status: "paused", reason: "Pick a cache." }),
        });
        const { container } = render(() => <SupervisorChip meta={meta} live={false} onSet={vi.fn()} />);
        const chip = container.querySelector(".swarm-supervisor-chip")!;
        expect(chip.classList.contains("swarm-supervisor-chip--paused")).toBe(true);
        expect(chip.getAttribute("title")).toContain("Pick a cache.");
    });

    it("opens the mode menu on click and sets the mode picked, showing a refusal", async () => {
        const onSet = vi.fn(async () => "the Supervisor works with Claude agents only for now");
        const { container } = render(() => <SupervisorChip meta={claude()} live={false} onSet={onSet} />);
        const chip = container.querySelector(".swarm-supervisor-chip")!;
        fireEvent.click(chip);
        const items = menu.show.mock.calls[0][0] as ContextMenuItem[];
        expect(items.map((i) => [i.label, i.checked])).toEqual([
            ["Supervisor: Off", true],
            ["Supervisor: Steady", false],
            ["Supervisor: Drive", false],
        ]);
        items[1].click!();
        expect(onSet).toHaveBeenCalledWith("steady");
        await Promise.resolve();
        await Promise.resolve();
        expect(chip.classList.contains("swarm-supervisor-chip--error")).toBe(true);
        expect(chip.getAttribute("title")).toBe("the Supervisor works with Claude agents only for now");
    });
});

describe("supervisorMenuItems", () => {
    it("checks the current mode", () => {
        expect(supervisorMenuItems("drive", vi.fn()).map((i) => i.checked)).toEqual([false, false, true]);
    });
});

describe("SupervisorBucket", () => {
    const history: SupervisorState["history"] = [
        { at_ms: 1, action: "mode_set", reason: "Set to Drive." },
        { at_ms: 2, action: "continued", verdict: "continue", reason: "It asked to proceed." },
        { at_ms: 3, action: "paused", verdict: "needs_user", reason: "It needs a password." },
    ];

    it("lists the decisions newest first, only while a mode is set", () => {
        const meta = { "supervisor:mode": "drive", "supervisor:state": state({ history }) };
        expect(supervisorRows(meta).map((r) => r.action)).toEqual(["paused", "continued", "mode_set"]);
        expect(supervisorRows({ "supervisor:state": state({ history }) })).toEqual([]);
    });

    it("shows each decision's action and reason", () => {
        const { container } = render(() => <SupervisorBucket rows={supervisorRows({ "supervisor:mode": "drive", "supervisor:state": state({ history }) })} />);
        const rows = container.querySelectorAll(".swarm-supervisor-row");
        expect(rows.length).toBe(3);
        expect(rows[0].querySelector(".swarm-supervisor-action")!.textContent).toBe("paused");
        expect(rows[0].querySelector(".swarm-supervisor-reason")!.textContent).toBe("It needs a password.");
        expect(rows[0].classList.contains("swarm-supervisor-row--paused")).toBe(true);
    });

    it("renders nothing with no decisions", () => {
        const { container } = render(() => <SupervisorBucket rows={[]} />);
        expect(container.querySelector(".swarm-bucket--supervisor")).toBeNull();
    });
});
