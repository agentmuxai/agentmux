// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Settings pane's Supervisor section: the master switch first with its explanation in both states, then every
 * limit and notice the Swarm's panel used to hold, with the same keys, defaults, bounds and descriptions.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ setConfig: vi.fn(), settings: { current: (() => ({})) as () => Record<string, unknown> } }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { SetConfigCommand: h.setConfig } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/global", () => ({ settingsAtom: () => h.settings.current() }));

import { SETTINGS_INDEX, searchSettings } from "../settings-index";
import { SUPERVISOR_LIMITS, SUPERVISOR_NOTICES, SUPERVISOR_SETTINGS, SupervisorSection } from "./supervisor-section";

afterEach(() => {
    cleanup();
    h.setConfig.mockClear();
});

function withSettings(initial: Record<string, unknown>) {
    const [settings, setSettings] = createSignal(initial);
    h.settings.current = settings;
    return setSettings;
}

const rowIds = (c: HTMLElement) => [...c.querySelectorAll<HTMLElement>(".setting-row")].map((r) => r.id);

describe("the Supervisor section", () => {
    it("has the master switch first, then every limit and notice, in the Swarm panel's order", () => {
        withSettings({});
        const { container } = render(() => <SupervisorSection />);
        expect(rowIds(container)).toEqual([
            "setting-supervisor.enabled",
            "setting-supervisor.steady_continuations",
            "setting-supervisor.steady_minutes",
            "setting-supervisor.drive_continuations",
            "setting-supervisor.drive_minutes",
            "setting-supervisor.noprogress",
            "setting-supervisor.cooldown",
            "setting-supervisor.hourly",
            "setting-supervisor.notify_needsyou",
            "setting-supervisor.notify_limit",
            "setting-supervisor.notify_failed",
            "setting-supervisor.notify_done",
        ]);
        expect(container.textContent).toContain("It judges each finished turn with Haiku 5.5 through the ambient gateway.");
    });

    it("explains the switch in both states: what it allows, and that each agent needs its own mode", () => {
        for (const initial of [{}, { "supervisor:enabled": true }]) {
            withSettings(initial);
            const { container, unmount } = render(() => <SupervisorSection />);
            const text = container.querySelector("#setting-supervisor\\.enabled")!.textContent!;
            expect(text).toContain("keep agents going when a turn ends");
            expect(text).toContain("Each agent still needs its own mode");
            unmount();
        }
    });

    it("turning the switch on or off writes the master switch", () => {
        withSettings({});
        const off = render(() => <SupervisorSection />);
        fireEvent.click(off.container.querySelector("#setting-supervisor\\.enabled [role=switch]")!);
        expect(h.setConfig).toHaveBeenCalledWith({}, { "supervisor:enabled": true });
        off.unmount();
        withSettings({ "supervisor:enabled": true });
        const on = render(() => <SupervisorSection />);
        const sw = on.container.querySelector("#setting-supervisor\\.enabled [role=switch]")!;
        expect(sw.getAttribute("aria-checked")).toBe("true");
        fireEvent.click(sw);
        expect(h.setConfig).toHaveBeenCalledWith({}, { "supervisor:enabled": false });
    });

    it("shows each limit at its default with its description, and a changed one writes its key", () => {
        withSettings({ "supervisor:drive:minutes": 45 });
        const { container } = render(() => <SupervisorSection />);
        const inputs = [...container.querySelectorAll<HTMLInputElement>(".setting-row input")];
        expect(inputs.map((i) => Number(i.value))).toEqual([5, 0, 15, 45, 2, 30, 30]);
        expect(container.textContent).toContain("Stop a Drive run after this long. 0 means no cap. 0 to 1440. Default 120.");
        fireEvent.input(inputs[2], { target: { value: "8" } });
        fireEvent.blur(inputs[2]);
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
        expect(SUPERVISOR_NOTICES.map((n) => n.key)).toEqual([
            "supervisor:notify:needsyou",
            "supervisor:notify:limit",
            "supervisor:notify:failed",
            "supervisor:notify:done",
        ]);
    });

    it("each notice is on until turned off, and a toggle writes its key", () => {
        withSettings({ "supervisor:notify:limit": false });
        const { container } = render(() => <SupervisorSection />);
        const toggles = [...container.querySelectorAll<HTMLElement>("[id^='setting-supervisor.notify_'] [role=switch]")];
        expect(toggles.map((t) => t.getAttribute("aria-checked"))).toEqual(["true", "false", "true", "true"]);
        fireEvent.click(toggles[1]);
        expect(h.setConfig).toHaveBeenCalledWith({}, { "supervisor:notify:limit": true });
        fireEvent.click(toggles[3]);
        expect(h.setConfig).toHaveBeenCalledWith({}, { "supervisor:notify:done": false });
    });
});

describe("the Supervisor settings in the search", () => {
    it("has one entry per row, all in the Supervisor section, and finds them by their words", () => {
        const entries = Object.values(SUPERVISOR_SETTINGS);
        expect(entries).toHaveLength(12);
        expect(entries.every((e) => e.section === "supervisor" && !e.opens)).toBe(true);
        for (const e of entries) expect(SETTINGS_INDEX).toContain(e);
        for (const [q, id] of [
            ["supervisor", "supervisor.enabled"],
            ["fleet budget", "supervisor.hourly"],
            ["wait before continuing", "supervisor.cooldown"],
            ["supervisor:notify:done", "supervisor.notify_done"],
        ] as const) {
            expect(searchSettings(SETTINGS_INDEX, q).map((e) => e.id), q).toContain(id);
        }
    });
});
