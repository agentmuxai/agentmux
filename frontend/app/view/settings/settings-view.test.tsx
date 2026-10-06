// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for the Settings rail (`SettingsView`) — no test coverage existed
 * for this view before; added alongside the dynamic-pane-title fix (see
 * docs/specs/SPEC_SECTIONED_PANE_DYNAMIC_TITLE_2026_08_12.md §3.2, §8) since
 * that fix touches this file and there was nothing to catch a regression.
 * Mirrors section-pane.test.tsx / warden-view.test.tsx's structure where it
 * applies; Settings has no per-pane zoom and no meta-backed section state
 * (SettingsViewModel has no blockAtom), so there's no zoom `describe` block
 * and section switches are asserted via `model.activeSection()` directly
 * rather than an RPC mock.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

// importOriginal, not a bare replacement object: each section file now also
// exports a *_SETTINGS search-index registry (SPEC_SETTINGS_PANE_SEARCH_2026_09_21.md
// §3.2) that settings-index.ts aggregates and SettingsSearchBar reads — a
// bare `{ AppearanceSection: ... }` mock would strip that export out from
// under settings-index.ts too, since it replaces the whole module.
vi.mock("./sections/appearance-section", async (importOriginal) => ({
    ...(await importOriginal<object>()),
    AppearanceSection: () => <div data-testid="appearance-section" />,
}));
vi.mock("./sections/window-panes-section", async (importOriginal) => ({
    ...(await importOriginal<object>()),
    WindowPanesSection: () => <div data-testid="window-section" />,
}));
vi.mock("./sections/terminal-section", async (importOriginal) => ({
    ...(await importOriginal<object>()),
    TerminalSection: () => <div data-testid="terminal-section" />,
}));
vi.mock("./sections/sounds-section", async (importOriginal) => ({
    ...(await importOriginal<object>()),
    SoundsSection: () => <div data-testid="sounds-section" />,
}));
vi.mock("./sections/notifications-section", async (importOriginal) => ({
    ...(await importOriginal<object>()),
    NotificationsSection: () => <div data-testid="notifications-section" />,
}));
vi.mock("./sections/recording-section", async (importOriginal) => ({
    ...(await importOriginal<object>()),
    RecordingSection: () => <div data-testid="recording-section" />,
}));
vi.mock("./sections/advanced-section", async (importOriginal) => ({
    ...(await importOriginal<object>()),
    AdvancedSection: () => <div data-testid="advanced-section" />,
}));

import { SettingsView } from "./settings-view";
import { SettingsViewModel } from "./settings-model";

describe("SettingsView rail", () => {
    afterEach(() => {
        cleanup();
    });

    function renderSettings() {
        const model = new SettingsViewModel();
        const result = render(() => <SettingsView model={model} />);
        return { ...result, model };
    }

    it("orders the rail as Appearance, Window & Panes, Terminal, Sounds, Notifications & Tray, Recording, Advanced", () => {
        renderSettings();
        const rail = screen.getByRole("tablist", { name: "Settings section" });
        const labels = Array.from(rail.querySelectorAll('[role="tab"]')).map((el) => el.textContent);
        expect(labels).toEqual(["Appearance", "Window & Panes", "Terminal", "Sounds", "Notifications & Tray", "Recording", "Advanced"]);
    });

    it("defaults to the Appearance section visible", () => {
        renderSettings();
        expect(screen.getByTestId("appearance-section")).toBeInTheDocument();
        expect(screen.queryByTestId("terminal-section")).not.toBeInTheDocument();
    });

    // One tablist that changes layout with width, instead of a rail plus a
    // separate top tab bar (SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md
    // §5.4). It still comes before the content, so at narrow widths it sits at
    // the top (SPEC_RESPONSIVE_TAB_BAR_TOP_POSITION_2026_08_24.md).
    it("renders one tablist, before the content it controls", () => {
        renderSettings();
        expect(screen.getAllByRole("tablist")).toHaveLength(1);
        const tablist = screen.getByRole("tablist", { name: "Settings section" });
        const panel = screen.getByRole("tabpanel");
        expect(tablist.compareDocumentPosition(panel) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
        expect(panel.getAttribute("aria-labelledby")).toBe(screen.getByRole("tab", { name: "Appearance" }).id);
    });

    it("clicking a rail item switches the visible section", () => {
        const { model } = renderSettings();
        screen.getByRole("tab", { name: "Terminal" }).click();
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
        const model = new SettingsViewModel();
        expect(model.viewName()).toBe("Appearance");
    });

    it("viewName() reflects the active section after setSection", () => {
        const model = new SettingsViewModel();
        model.setSection("sounds");
        expect(model.viewName()).toBe("Sounds");
    });

    it("clicking a rail item updates viewName() to match", () => {
        const model = new SettingsViewModel();
        render(() => (
            <SettingsView model={model} />
        ));
        screen.getByRole("tab", { name: "Advanced" }).click();
        expect(model.viewName()).toBe("Advanced");
    });
});

// SPEC_SETTINGS_PANE_SEARCH_2026_09_21.md §3.5/§6.
describe("SettingsView search bar", () => {
    afterEach(() => cleanup());

    function renderSettings() {
        const model = new SettingsViewModel();
        render(() => (
            <SettingsView model={model} />
        ));
        return { model };
    }

    it("shows no results dropdown when the query is empty", () => {
        renderSettings();
        expect(screen.queryByTestId("settings-search-results")).not.toBeInTheDocument();
    });

    it("finds a setting by a curated synonym, not just its literal label", () => {
        renderSettings();
        const input = screen.getByTestId("settings-search-input");
        fireEvent.input(input, { target: { value: "dark mode" } });
        const results = screen.getAllByTestId("settings-search-result");
        expect(results[0]).toHaveTextContent("Theme");
    });

    it("switches section and clears the query when a cross-section result is selected", () => {
        const { model } = renderSettings();
        expect(model.activeSection()).toBe("appearance");
        const input = screen.getByTestId("settings-search-input");
        // "clipboard" only matches a Terminal-section setting (Copy on
        // select) — not anything in the default Appearance section — so
        // selecting it must switch sections, not just filter within one.
        fireEvent.input(input, { target: { value: "clipboard" } });
        const result = screen.getAllByTestId("settings-search-result")[0];
        expect(result).toHaveTextContent("Copy on select");
        fireEvent.click(result);
        expect(model.activeSection()).toBe("terminal");
        expect(model.query()).toBe("");
        expect(screen.queryByTestId("settings-search-results")).not.toBeInTheDocument();
    });

    it("lists an exact setting name alone, not the settings that only look like it", () => {
        renderSettings();
        const input = screen.getByTestId("settings-search-input");
        fireEvent.input(input, { target: { value: "Message accepted" } });
        const results = screen.getAllByTestId("settings-search-result");
        expect(results).toHaveLength(1);
        expect(results[0]).toHaveTextContent("Message accepted");
    });

    it("still finds a setting through a typo", () => {
        renderSettings();
        const input = screen.getByTestId("settings-search-input");
        fireEvent.input(input, { target: { value: "Copy on selct" } });
        expect(screen.getAllByTestId("settings-search-result")[0]).toHaveTextContent("Copy on select");
    });

    it("shows an empty state for a query with no match", () => {
        renderSettings();
        const input = screen.getByTestId("settings-search-input");
        fireEvent.input(input, { target: { value: "xyzzy_nonexistent_setting" } });
        expect(screen.getByTestId("settings-search-empty")).toBeInTheDocument();
    });
});
