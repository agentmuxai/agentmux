// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Settings → Agents → Desktop driving: the switch for `desktop:enabled`. */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const setConfig = vi.fn();
let settings: Record<string, unknown> = {};

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { SetConfigCommand: (...args: unknown[]) => setConfig(...args) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/global", () => ({ settingsAtom: () => settings }));

import { AGENTS_SETTINGS, AgentsSection } from "./agents-section";
import { DESKTOP_DRIVING_SETTING } from "./desktop-driving";

const desktopSwitch = () => document.getElementById(`setting-${DESKTOP_DRIVING_SETTING.id}`)?.querySelector('[role="switch"]') ?? null;

afterEach(() => {
    cleanup();
    setConfig.mockReset();
    settings = {};
});

describe("Desktop driving in Settings", () => {
    it("is off until the person turns it on, and turning it on writes desktop:enabled", () => {
        render(() => <AgentsSection />);
        expect(screen.getByText("Desktop driving")).toBeTruthy();
        expect(desktopSwitch()?.getAttribute("aria-checked")).toBe("false");
        fireEvent.click(desktopSwitch()!);
        expect(setConfig).toHaveBeenCalledWith({}, { "desktop:enabled": true });
    });

    it("shows on when the setting is on, and turning it off writes false", () => {
        settings = { "desktop:enabled": true };
        render(() => <AgentsSection />);
        expect(desktopSwitch()?.getAttribute("aria-checked")).toBe("true");
        fireEvent.click(desktopSwitch()!);
        expect(setConfig).toHaveBeenCalledWith({}, { "desktop:enabled": false });
    });

    it("says what turning it on allows, and is found by search under Agents", () => {
        expect(DESKTOP_DRIVING_SETTING.description).toMatch(/click and type/);
        expect(DESKTOP_DRIVING_SETTING.description).toMatch(/allow that app/);
        expect(DESKTOP_DRIVING_SETTING.description).toMatch(/stop an agent/);
        expect(AGENTS_SETTINGS.desktopDriving.section).toBe("agents");
        expect(DESKTOP_DRIVING_SETTING.keywords).toContain("desktop:enabled");
    });
});
