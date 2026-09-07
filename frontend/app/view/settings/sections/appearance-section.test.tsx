// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for the startup-splash toggle in the Appearance section.
 *
 * This row is the one control in Settings whose stored value is INVERTED
 * relative to its label: the key is `splash:disabled` (false by default)
 * because `agentmux-launcher` reads settings.json directly, before the
 * frontend exists, and "disabled" is the form that fail-safe read wants —
 * an unreadable or absent value must mean "show the splash"
 * (`agentmux-launcher/src/splash_config.rs`). A polarity slip here would
 * silently hide the splash for every user who ever opened Settings, so the
 * mapping is pinned in both directions.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const setConfig = vi.fn();
let settings: Record<string, unknown> = {};

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        SetConfigCommand: (...args: unknown[]) => setConfig(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/global", () => ({
    settingsAtom: () => settings,
}));
vi.mock("@/app/menu/base-menus", () => ({ THEME_OPTIONS: [] }));

import { AppearanceSection } from "./appearance-section";

describe("Appearance — startup splash toggle", () => {
    beforeEach(() => {
        setConfig.mockReset();
        settings = {};
    });
    afterEach(() => {
        cleanup();
    });

    function splashToggle(): HTMLElement {
        render(() => <AppearanceSection />);
        // The row's label is positive ("show the splash"), so find the switch
        // by that row rather than by index — adding a row above must not
        // silently retarget this test.
        const row = screen.getByText("Startup splash screen").closest(".setting-row");
        expect(row).not.toBeNull();
        const toggle = row!.querySelector('[role="switch"]');
        expect(toggle).not.toBeNull();
        return toggle as HTMLElement;
    }

    it("shows as ON when the setting is absent, because the splash defaults to visible", () => {
        expect(splashToggle().getAttribute("aria-checked")).toBe("true");
    });

    it("shows as OFF when splash:disabled is true", () => {
        settings = { "splash:disabled": true };
        expect(splashToggle().getAttribute("aria-checked")).toBe("false");
    });

    it("writes splash:disabled=true when the user turns the splash OFF", () => {
        fireEvent.click(splashToggle());
        expect(setConfig).toHaveBeenCalledTimes(1);
        expect(setConfig.mock.calls[0][1]).toEqual({ "splash:disabled": true });
    });

    it("writes splash:disabled=false when the user turns the splash back ON", () => {
        settings = { "splash:disabled": true };
        fireEvent.click(splashToggle());
        expect(setConfig.mock.calls[0][1]).toEqual({ "splash:disabled": false });
    });
});
