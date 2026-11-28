// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Settings → Agents → Browser windows: the switch for `agent:floatbrowsers`. */

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
import { FLOAT_BROWSERS_SETTING } from "./float-browsers";

const floatSwitch = () => document.getElementById(`setting-${FLOAT_BROWSERS_SETTING.id}`)?.querySelector('[role="switch"]') ?? null;

afterEach(() => {
    cleanup();
    setConfig.mockReset();
    settings = {};
});

describe("Agent browsers float in Settings", () => {
    it("is off until the person turns it on, and turning it on writes agent:floatbrowsers", () => {
        render(() => <AgentsSection />);
        expect(screen.getByText("Agent browsers float")).toBeTruthy();
        expect(floatSwitch()?.getAttribute("aria-checked")).toBe("false");
        fireEvent.click(floatSwitch()!);
        expect(setConfig).toHaveBeenCalledWith({}, { "agent:floatbrowsers": true });
    });

    it("shows on when the setting is on, and turning it off writes false", () => {
        settings = { "agent:floatbrowsers": true };
        render(() => <AgentsSection />);
        expect(floatSwitch()?.getAttribute("aria-checked")).toBe("true");
        fireEvent.click(floatSwitch()!);
        expect(setConfig).toHaveBeenCalledWith({}, { "agent:floatbrowsers": false });
    });

    it("is found by searching for a floating browser", () => {
        expect(AGENTS_SETTINGS.floatBrowsers.keywords).toContain("floating");
    });
});
