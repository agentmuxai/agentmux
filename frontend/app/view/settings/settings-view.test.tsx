// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for the Settings section tabs (`SettingsView`) — no test coverage existed
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
vi.mock("./sections/devices-section", async (importOriginal) => ({
    ...(await importOriginal<object>()),
    DevicesSection: () => <div data-testid="devices-section" />,
}));
vi.mock("./sections/widgets-section", async (importOriginal) => ({
    ...(await importOriginal<object>()),
    WidgetsSection: () => <div data-testid="widgets-section" />,
}));
vi.mock("./sections/advanced-section", async (importOriginal) => ({
    ...(await importOriginal<object>()),
    AdvancedSection: () => <div data-testid="advanced-section" />,
}));

import { createRoot } from "solid-js";
import { settingsPaneTab } from "./settings";
import { SettingsView } from "./settings-view";
import { SettingsViewModel } from "./settings-model";

describe("SettingsView section tabs", () => {
    afterEach(() => {
        cleanup();
    });

    function renderSettings() {
        const model = new SettingsViewModel();
        const result = render(() => <SettingsView model={model} />);
        return { ...result, model };
    }

    it("orders the tabs as Appearance, Window & Panes, Browser, Terminal, Sounds, Notifications & Tray, Recording, Paired devices, Widgets, Advanced", () => {
        renderSettings();
        const tabs = screen.getByRole("tablist", { name: "Settings section" });
        const labels = Array.from(tabs.querySelectorAll('[role="tab"]')).map((el) => el.textContent);
        expect(labels).toEqual(["Appearance", "Window & Panes", "Browser", "Terminal", "Sounds", "Notifications & Tray", "Recording", "Paired devices", "Widgets", "Advanced"]);
    });

    it("defaults to the Appearance section visible", () => {
        renderSettings();
        expect(screen.getByTestId("appearance-section")).toBeInTheDocument();
        expect(screen.queryByTestId("terminal-section")).not.toBeInTheDocument();
    });

    // One tablist, along the top at every width
    // (SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md §5.4), before the
    // content it controls.
    it("renders one tablist, before the content it controls", () => {
        renderSettings();
        expect(screen.getAllByRole("tablist")).toHaveLength(1);
        const tablist = screen.getByRole("tablist", { name: "Settings section" });
        const panel = screen.getByRole("tabpanel");
        expect(tablist.compareDocumentPosition(panel) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
        expect(panel.getAttribute("aria-labelledby")).toBe(screen.getByRole("tab", { name: "Appearance" }).id);
    });

    it("clicking a tab switches the visible section", () => {
        const { model } = renderSettings();
        screen.getByRole("tab", { name: "Terminal" }).click();
        expect(model.activeSection()).toBe("terminal");
        expect(screen.getByTestId("terminal-section")).toBeInTheDocument();
        expect(screen.queryByTestId("appearance-section")).not.toBeInTheDocument();
    });

    // REPORT_FOCUS_ON_OPEN_AUDIT_2026_10_08.md: switching sections with the
    // mouse used to leave the caret on the tab, so typing didn't search.
    it("a tab picked with the mouse hands the caret back to the search, its text selected", async () => {
        const { model } = renderSettings();
        model.setQuery("font");
        const tab = screen.getByRole("tab", { name: "Terminal" });
        fireEvent.pointerDown(tab);
        tab.focus();
        tab.click();
        await Promise.resolve();
        await Promise.resolve();
        const search = document.querySelector<HTMLInputElement>(".settings-search-input")!;
        expect(document.activeElement).toBe(search);
        expect([search.selectionStart, search.selectionEnd]).toEqual([0, search.value.length]);
    });

    it("arrow keys in the tab list keep the caret on the tabs", async () => {
        renderSettings();
        const tab = screen.getByRole("tab", { name: "Appearance" });
        tab.focus();
        fireEvent.keyDown(tab, { key: "ArrowRight" });
        await Promise.resolve();
        await Promise.resolve();
        expect(document.activeElement).toBe(screen.getByRole("tab", { name: "Window & Panes" }));
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

    it("clicking a tab updates viewName() to match", () => {
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

describe("Settings pane: the section to open at", () => {
    // The host passes meta as an accessor; reading it as an object silently
    // opened every Settings pane at Appearance (a browser pane's Manage
    // profiles… asks for Browser).
    function openWith(meta: Record<string, unknown>): string {
        return createRoot((dispose) => {
            const inst = settingsPaneTab.create({
                blockId: "b1",
                meta: () => meta as MetaType,
                setMeta: async () => {},
                isFocused: () => true,
                visibility: () => "active",
            });
            const title = inst.liveTitle!().text;
            dispose();
            return title;
        });
    }

    it("opens at the section its block asks for", () => {
        expect(openWith({ view: "settings", "settings:section": "browser" })).toBe("Browser");
    });

    it("opens at Appearance when none, or an unknown one, is asked for", () => {
        expect(openWith({ view: "settings" })).toBe("Appearance");
        expect(openWith({ view: "settings", "settings:section": "nope" })).toBe("Appearance");
    });
});
