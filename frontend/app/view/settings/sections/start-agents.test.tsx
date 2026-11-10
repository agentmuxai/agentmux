// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Settings → Agents → Starting agents: the switch for `agent:maystartagents`. */

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
import { START_AGENTS_SETTING } from "./start-agents";

const startSwitch = () => document.getElementById(`setting-${START_AGENTS_SETTING.id}`)?.querySelector('[role="switch"]') ?? null;

afterEach(() => {
    cleanup();
    setConfig.mockReset();
    settings = {};
});

describe("Agents may start agents in Settings", () => {
    it("is off until the person turns it on, and turning it on writes agent:maystartagents", () => {
        render(() => <AgentsSection />);
        expect(screen.getByText("Agents may start agents")).toBeTruthy();
        expect(startSwitch()?.getAttribute("aria-checked")).toBe("false");
        fireEvent.click(startSwitch()!);
        expect(setConfig).toHaveBeenCalledWith({}, { "agent:maystartagents": true });
    });

    it("shows on when the setting is on, and turning it off writes false", () => {
        settings = { "agent:maystartagents": true };
        render(() => <AgentsSection />);
        expect(startSwitch()?.getAttribute("aria-checked")).toBe("true");
        fireEvent.click(startSwitch()!);
        expect(setConfig).toHaveBeenCalledWith({}, { "agent:maystartagents": false });
    });

    it("is found by searching for starting agents", () => {
        expect(AGENTS_SETTINGS.startAgents.keywords).toContain("start agents");
    });
});
