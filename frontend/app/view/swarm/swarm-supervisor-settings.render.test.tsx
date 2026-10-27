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

describe("SupervisorSettingsBar", () => {
    it("says the Supervisor is off, and turning it on writes the master switch", () => {
        withSettings({});
        const { container, getByRole } = render(() => <SupervisorSettingsBar />);
        expect(container.textContent).toContain("Supervisor: off");
        expect(container.textContent).toContain("Turn it on to set modes");
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
        expect(inputs.map((i) => Number(i.value))).toEqual([5, 0, 15, 45, 2]);
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
        ]);
    });
});
