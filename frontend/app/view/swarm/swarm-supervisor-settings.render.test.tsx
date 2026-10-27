// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Supervisor section's settings line: the master switch and the limits.
 * docs/specs/SPEC_SWARM_SECTIONS_SUPERVISOR_SETTINGS_AND_KANBAN_2026_10_24.md §6.2.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ setConfig: vi.fn(), settings: { current: (() => ({})) as () => Record<string, unknown> } }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { SetConfigCommand: h.setConfig } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/global", () => ({ settingsAtom: () => h.settings.current() }));

import { requestSupervisorSettingsOpen, resetSupervisorSettingsRequest } from "@/app/store/supervisor-settings-request";
import { SUPERVISOR_LIMITS, SupervisorSettingsBar } from "./swarm-supervisor-settings";

afterEach(() => {
    cleanup();
    h.setConfig.mockClear();
});

function withSettings(initial: Record<string, unknown>) {
    const [settings, setSettings] = createSignal(initial);
    h.settings.current = settings;
    return setSettings;
}

describe("SupervisorSettingsBar: asked to open from the Settings search", () => {
    it("opens its panel on mount when the ask came first", () => {
        resetSupervisorSettingsRequest();
        withSettings({});
        requestSupervisorSettingsOpen("pane-1");
        const { container } = render(() => <SupervisorSettingsBar blockId="pane-1" />);
        expect(container.querySelector(".swarm-supervisor-settings-panel")).not.toBeNull();
    });

    it("opens a mounted panel at once, and one ask opens one panel", () => {
        resetSupervisorSettingsRequest();
        withSettings({});
        const first = render(() => <SupervisorSettingsBar blockId="pane-1" />);
        expect(first.container.querySelector(".swarm-supervisor-settings-panel")).toBeNull();
        requestSupervisorSettingsOpen("pane-1");
        expect(first.container.querySelector(".swarm-supervisor-settings-panel")).not.toBeNull();
        // The ask is spent: a bar that mounts later stays closed.
        const second = render(() => <SupervisorSettingsBar blockId="pane-1" />);
        expect(second.container.querySelector(".swarm-supervisor-settings-panel")).toBeNull();
    });

    it("opens only the pane that was asked, not a Swarm mounted in another tab", () => {
        resetSupervisorSettingsRequest();
        withSettings({});
        const other = render(() => <SupervisorSettingsBar blockId="other-tab" />);
        // The target mounts first and the ask comes after, as for a Swarm the search just created.
        const target = render(() => <SupervisorSettingsBar blockId="new-pane" />);
        requestSupervisorSettingsOpen("new-pane");
        expect(target.container.querySelector(".swarm-supervisor-settings-panel")).not.toBeNull();
        expect(other.container.querySelector(".swarm-supervisor-settings-panel")).toBeNull();
        // Nor does it wait for the other one to open later.
        const later = render(() => <SupervisorSettingsBar blockId="other-tab" />);
        expect(later.container.querySelector(".swarm-supervisor-settings-panel")).toBeNull();
    });

    it("stays closed when nobody asked", () => {
        resetSupervisorSettingsRequest();
        withSettings({});
        const { container } = render(() => <SupervisorSettingsBar blockId="pane-1" />);
        expect(container.querySelector(".swarm-supervisor-settings-panel")).toBeNull();
    });
});

describe("SupervisorSettingsBar", () => {
    it("says the Supervisor is off, and turning it on writes the master switch", () => {
        withSettings({});
        const { container, getByRole } = render(() => <SupervisorSettingsBar />);
        expect(container.textContent).toContain("Supervisor: off");
        expect(container.textContent).toContain("Turn it on to set modes");
        expect(getByRole("switch").getAttribute("aria-label")).toBe("Supervisor");
        fireEvent.click(getByRole("switch"));
        expect(h.setConfig).toHaveBeenCalledWith({}, { "supervisor:enabled": true });
    });

    it("on: says so and offers turning it off", () => {
        withSettings({ "supervisor:enabled": true });
        const { container, getByRole } = render(() => <SupervisorSettingsBar />);
        expect(container.textContent).toContain("Supervisor: on");
        expect(container.textContent).not.toContain("Turn it on to set modes");
        fireEvent.click(getByRole("switch"));
        expect(h.setConfig).toHaveBeenCalledWith({}, { "supervisor:enabled": false });
    });

    it("the panel is closed until Settings is pressed, and shows each limit at its default", () => {
        withSettings({ "supervisor:drive:minutes": 45 });
        const { container, getByText } = render(() => <SupervisorSettingsBar />);
        expect(container.querySelector(".swarm-supervisor-settings-panel")).toBeNull();
        fireEvent.click(getByText("Settings"));
        const inputs = [...container.querySelectorAll<HTMLInputElement>(".swarm-supervisor-settings-panel input")];
        expect(inputs.map((i) => Number(i.value))).toEqual([5, 0, 15, 45, 2, 30, 30]);
        expect(container.textContent).toContain("Default 120");
    });

    it("a limit changed in the panel writes its key", () => {
        withSettings({});
        const { container, getByText } = render(() => <SupervisorSettingsBar />);
        fireEvent.click(getByText("Settings"));
        const drive = container.querySelectorAll<HTMLInputElement>(".swarm-supervisor-settings-panel input")[2];
        fireEvent.input(drive, { target: { value: "8" } });
        fireEvent.blur(drive);
        expect(h.setConfig).toHaveBeenCalledWith({}, { "supervisor:drive:continuations": 8 });
    });

    it("the limits match srv's keys, defaults and bounds", () => {
        expect(SUPERVISOR_LIMITS.map((l) => [l.key, l.fallback, l.min, l.max])).toEqual([
            ["supervisor:steady:continuations", 5, 1, 100],
            ["supervisor:steady:minutes", 0, 0, 1440],
            ["supervisor:drive:continuations", 15, 1, 100],
            ["supervisor:drive:minutes", 120, 0, 1440],
            ["supervisor:noprogress", 2, 1, 10],
            ["supervisor:cooldown", 30, 0, 600],
            ["supervisor:hourly", 30, 0, 1000],
        ]);
    });

    it("each notice is on until turned off, and a toggle writes its key", () => {
        withSettings({ "supervisor:notify:limit": false });
        const { container, getByText } = render(() => <SupervisorSettingsBar />);
        fireEvent.click(getByText("Settings"));
        const toggles = [...container.querySelectorAll<HTMLElement>(".swarm-supervisor-settings-panel [role=switch]")];
        expect(toggles.map((t) => t.getAttribute("aria-checked"))).toEqual(["true", "false", "true", "true"]);
        fireEvent.click(toggles[1]);
        expect(h.setConfig).toHaveBeenCalledWith({}, { "supervisor:notify:limit": true });
        fireEvent.click(toggles[3]);
        expect(h.setConfig).toHaveBeenCalledWith({}, { "supervisor:notify:done": false });
    });
});
