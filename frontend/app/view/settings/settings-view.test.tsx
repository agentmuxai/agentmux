// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for the Settings rail (`SettingsView`) — no test coverage existed
 * for this view before; added alongside the dynamic-pane-title fix (see
 * docs/specs/SPEC_SECTIONED_PANE_DYNAMIC_TITLE_2026_08_12.md §3.2, §8) since
 * that fix touches this file and there was nothing to catch a regression.
 * Mirrors armory-view.test.tsx / warden-view.test.tsx's structure where it
 * applies; Settings has no per-pane zoom and no meta-backed section state
 * (SettingsViewModel has no blockAtom), so there's no zoom `describe` block
 * and section switches are asserted via `model.activeSection()` directly
 * rather than an RPC mock.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("./sections/appearance-section", () => ({
    AppearanceSection: () => <div data-testid="appearance-section" />,
}));
vi.mock("./sections/window-panes-section", () => ({
    WindowPanesSection: () => <div data-testid="window-section" />,
}));
vi.mock("./sections/terminal-section", () => ({
    TerminalSection: () => <div data-testid="terminal-section" />,
}));
vi.mock("./sections/sounds-section", () => ({
    SoundsSection: () => <div data-testid="sounds-section" />,
}));
vi.mock("./sections/recording-section", () => ({
    RecordingSection: () => <div data-testid="recording-section" />,
}));
vi.mock("./sections/advanced-section", () => ({
    AdvancedSection: () => <div data-testid="advanced-section" />,
}));

import { SettingsView } from "./settings-view";
import { SettingsViewModel } from "./settings-model";

describe("SettingsView rail", () => {
    afterEach(() => {
        cleanup();
    });

    function renderSettings() {
        const model = new SettingsViewModel("test-block", null as any);
        const result = render(() => (
            <SettingsView
                blockId="test-block"
                model={model}
                blockRef={{ current: null }}
                contentRef={{ current: null }}
            />
        ));
        return { ...result, model };
    }

    it("orders the rail as Appearance, Window & Panes, Terminal, Sounds, Recording, Advanced", () => {
        renderSettings();
        const rail = screen.getByLabelText("Settings section", { selector: "nav.settings-rail" });
        const labels = Array.from(rail.querySelectorAll("button span")).map((el) => el.textContent);
        expect(labels).toEqual(["Appearance", "Window & Panes", "Terminal", "Sounds", "Recording", "Advanced"]);
    });

    it("defaults to the Appearance section visible", () => {
        renderSettings();
        expect(screen.getByTestId("appearance-section")).toBeInTheDocument();
        expect(screen.queryByTestId("terminal-section")).not.toBeInTheDocument();
    });

    it("clicking a rail item switches the visible section", () => {
        const { model } = renderSettings();
        const rail = screen.getByLabelText("Settings section", { selector: "nav.settings-rail" });
        const terminalButton = Array.from(rail.querySelectorAll("button")).find(
            (b) => b.textContent?.includes("Terminal"),
        ) as HTMLButtonElement;
        terminalButton.click();
        expect(model.activeSection()).toBe("terminal");
        expect(screen.getByTestId("terminal-section")).toBeInTheDocument();
        expect(screen.queryByTestId("appearance-section")).not.toBeInTheDocument();
    });
});

describe("SettingsView pane title", () => {
    afterEach(() => {
        cleanup();
    });

    it("defaults viewName() to 'Appearance'", () => {
        const model = new SettingsViewModel("test-block", null as any);
        expect(model.viewName()).toBe("Appearance");
    });

    it("viewName() reflects the active section after setSection", () => {
        const model = new SettingsViewModel("test-block", null as any);
        model.setSection("sounds");
        expect(model.viewName()).toBe("Sounds");
    });

    it("clicking a rail item updates viewName() to match", () => {
        const model = new SettingsViewModel("test-block", null as any);
        render(() => (
            <SettingsView blockId="test-block" model={model} blockRef={{ current: null }} contentRef={{ current: null }} />
        ));
        const rail = screen.getByLabelText("Settings section", { selector: "nav.settings-rail" });
        const advancedButton = Array.from(rail.querySelectorAll("button")).find(
            (b) => b.textContent?.includes("Advanced"),
        ) as HTMLButtonElement;
        advancedButton.click();
        expect(model.viewName()).toBe("Advanced");
    });
});
