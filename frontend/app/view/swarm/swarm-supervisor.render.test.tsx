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
vi.mock("@/app/element/ui/open-menu-at", async (original) => ({ ...(await original<typeof import("@/app/element/ui/open-menu-at")>()), openMenuAt: menu.show }));
// The master switch, which the person turns on in Settings: the chip reads it.
const master = vi.hoisted(() => ({ on: true }));
vi.mock("@/app/store/global", async (original) => ({
    ...(await original<typeof import("@/app/store/global")>()),
    settingsAtom: () => ({ "supervisor:enabled": master.on }),
}));
const settings = vi.hoisted(() => ({ openSettingsAt: vi.fn(async () => {}) }));
vi.mock("@/app/view/settings/open-settings", () => ({ openSettingsAt: settings.openSettingsAt }));

import { SupervisorBucket, SupervisorChip, supervisorChipTitle, supervisorMenuHeader, supervisorMenuItems, supervisorRows } from "./swarm-supervisor";

afterEach(() => {
    cleanup();
    menu.show.mockClear();
    settings.openSettingsAt.mockClear();
    master.on = true;
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
        expect(items.filter((i) => i.type === "radio").map((i) => [i.label, i.checked])).toEqual([
            ["Off", true],
            ["Steady", false],
            ["Drive", false],
        ]);
        expect(menu.show.mock.calls[0][2]).toEqual({ header: ["Supervisor mode"] });
        items[1].click!();
        expect(onSet).toHaveBeenCalledWith("steady");
        await Promise.resolve();
        await Promise.resolve();
        expect(chip.classList.contains("swarm-supervisor-chip--error")).toBe(true);
        expect(chip.getAttribute("title")).toBe("the Supervisor works with Claude agents only for now");
    });
});

describe("supervisorMenuItems", () => {
    it("has a row per mode with its name and its explanation as separate fields, then a divider and the settings entry", () => {
        const items = supervisorMenuItems("steady", vi.fn());
        expect(items.map((i) => i.label ?? "—")).toEqual(["Off", "Steady", "Drive", "—", "Supervisor settings…"]);
        expect(items.slice(0, 3).map((i) => i.description)).toEqual([
            "Leave the agent to stop when it stops.",
            "Keep it going when it stops only to check in. Pause for real questions.",
            "Keep it going through open questions on its own best judgement. Pause for what only you can provide.",
        ]);
        // The name carries no explanation, and the explanation is not a right-hand sublabel (which is cut short).
        expect(items.slice(0, 3).every((i) => i.sublabel === undefined && !i.label!.includes("."))).toBe(true);
        expect(items[3].type).toBe("separator");
    });

    it("checks the current mode, and none for a selection", () => {
        expect(supervisorMenuItems("drive", vi.fn()).slice(0, 3).map((i) => i.checked)).toEqual([false, false, true]);
        expect(supervisorMenuItems(null, vi.fn()).slice(0, 3).map((i) => i.checked)).toEqual([undefined, undefined, undefined]);
    });

    it("offers every mode when it is on, and the settings entry opens the Settings pane at the Supervisor section", () => {
        const items = supervisorMenuItems("off", vi.fn());
        expect(items.map((i) => i.enabled)).toEqual([true, true, true, undefined, undefined]);
        items[4].click!();
        expect(settings.openSettingsAt).toHaveBeenCalledWith("supervisor");
    });

    it("switched off: Steady and Drive are disabled, Off and the settings entry are not", () => {
        const items = supervisorMenuItems("off", vi.fn(), false);
        expect(items.map((i) => i.enabled)).toEqual([true, false, false, undefined, undefined]);
        // The reason is said once, above the rows, not on each row.
        expect(items.every((i) => !i.sublabel && !JSON.stringify(i.description ?? "").includes("switched off"))).toBe(true);
        items[4].click!();
        expect(settings.openSettingsAt).toHaveBeenCalledWith("supervisor");
        expect(supervisorMenuHeader(false)).toEqual(["Supervisor mode", "The Supervisor is switched off in Settings, so Steady and Drive can't be chosen."]);
        expect(supervisorMenuHeader(true, "3 agents")).toEqual(["Supervisor mode for 3 agents"]);
    });
});

describe("the chip while the Supervisor is switched off", () => {
    it("shows the off state in every mode: another shape, no mode or detail, and a tooltip that says why", () => {
        master.on = false;
        for (const mode of ["off", "steady", "drive"] as const) {
            const meta = claude({ "supervisor:mode": mode, "supervisor:state": state({ mode: mode === "off" ? "steady" : mode, status: "continued", continuations: 3 }) });
            const { container, unmount } = render(() => <SupervisorChip meta={meta} live={true} onSet={vi.fn()} />);
            const chip = container.querySelector(".swarm-supervisor-chip")!;
            expect(chip.classList.contains("swarm-supervisor-chip--switched-off")).toBe(true);
            expect(chip.querySelector(".fa-user-slash")).not.toBeNull();
            expect(chip.querySelector(".fa-user-shield")).toBeNull();
            expect(chip.textContent).toBe("");
            expect(chip.getAttribute("title")).toContain("switched off in Settings");
            expect(chip.getAttribute("aria-label")).toContain("switched off");
            unmount();
        }
    });

    it("tells an agent set to Steady or Drive what it is set to, and an Off agent nothing about a mode", () => {
        expect(supervisorChipTitle("steady", null, false)).toContain("This agent is set to Steady; that applies again when it is turned on.");
        expect(supervisorChipTitle("drive", null, false)).toContain("set to Drive");
        expect(supervisorChipTitle("off", null, false)).not.toContain("set to");
        expect(supervisorChipTitle("off", null, true)).toBe("Supervisor: Off\nLeave the agent to stop when it stops.\nClick to change.");
    });

    it("is the ordinary chip again when the switch is on, and opens the menu with the reason once", () => {
        master.on = false;
        const { container } = render(() => <SupervisorChip meta={claude()} live={false} onSet={vi.fn()} />);
        fireEvent.click(container.querySelector(".swarm-supervisor-chip")!);
        expect(menu.show.mock.calls[0][2].header).toHaveLength(2);
        cleanup();
        master.on = true;
        const on = render(() => <SupervisorChip meta={claude()} live={false} onSet={vi.fn()} />);
        expect(on.container.querySelector(".swarm-supervisor-chip--switched-off")).toBeNull();
        expect(on.container.querySelector(".fa-user-shield")).not.toBeNull();
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
