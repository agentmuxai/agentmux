// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Settings → Agents → Agent view: the default `agent:view`, a preset and one row per switch. */

import { cleanup, fireEvent, render, screen, within } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const setConfig = vi.fn();
let settings: Record<string, unknown> = {};
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { SetConfigCommand: (...args: unknown[]) => setConfig(...args) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/global", () => ({ settingsAtom: () => settings }));

import { VIEW_PRESETS } from "@/app/view/agent/virtualization/verbosity";
import { AGENTS_SETTINGS, AgentsSection } from "./agents-section";

const group = (name: string) => screen.getByRole("radiogroup", { name });
const selected = (name: string) => within(group(name)).getAllByRole("radio").find((r) => r.getAttribute("aria-checked") === "true")?.textContent;

afterEach(() => {
    cleanup();
    setConfig.mockReset();
    settings = {};
});

describe("the default agent view in Settings", () => {
    it("reads Full with every switch on Auto when nothing is set", () => {
        render(() => <AgentsSection />);
        expect(selected("Default agent view")).toBe("Full");
        expect(selected("Thinking")).toBe("Auto");
        expect(selected("AgentMux notes")).toBe("Shown");
    });

    it("picking a preset writes the whole view", () => {
        render(() => <AgentsSection />);
        fireEvent.click(within(group("Default agent view")).getByText("Standard"));
        expect(setConfig).toHaveBeenCalledWith({}, { "agent:view": VIEW_PRESETS.standard });
    });

    it("changing one switch writes the view with that switch changed, and reads Custom", () => {
        settings = { "agent:view": "standard" };
        render(() => <AgentsSection />);
        expect(selected("Default agent view")).toBe("Standard");
        expect(selected("Tool calls")).toBe("Collapsed");
        fireEvent.click(within(group("Tool calls")).getByText("Hidden"));
        expect(setConfig).toHaveBeenCalledWith({}, { "agent:view": { ...VIEW_PRESETS.standard, tools: "hidden" } });
    });

    it("moves the highlight at once and keeps a quick second change on top of the first", () => {
        render(() => <AgentsSection />);
        fireEvent.click(within(group("Tool calls")).getByText("Expanded"));
        expect(selected("Tool calls")).toBe("Expanded");
        // The setting has not come back yet: this click must build on the first, not on Auto.
        fireEvent.click(within(group("Jekts and agent messages")).getByText("Hidden"));
        expect(setConfig).toHaveBeenLastCalledWith({}, { "agent:view": { ...VIEW_PRESETS.full, tools: "expanded", jekts: "hidden" } });
        expect(selected("Tool calls")).toBe("Expanded");
    });

    it("shows Custom, which is not a choice, when the switches match no preset", () => {
        settings = { "agent:view": { ...VIEW_PRESETS.full, thinking: "hidden" } };
        render(() => <AgentsSection />);
        expect(selected("Default agent view")).toBe("Custom");
        fireEvent.click(within(group("Default agent view")).getByText("Custom"));
        expect(setConfig).not.toHaveBeenCalled();
    });

    it("is found by searching for verbosity", () => {
        expect(AGENTS_SETTINGS.agentView.keywords).toContain("verbosity");
    });
});
