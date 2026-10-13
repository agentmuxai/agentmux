// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Settings › Widgets › Widget bar: rows in bar order, the three states, and that the list
 *  follows a config change with no other action (docs/specs/SPEC_WIDGET_BAR_VISIBILITY_SETTINGS_2026_10_12.md). */

import { cleanup, fireEvent, render, screen, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const setConfig = vi.fn(async () => {});
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { SetConfigCommand: (...a: unknown[]) => setConfig(...(a as [])) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/store/global", async () => {
    const signals = await import("@/app/store/config-signals");
    return { atoms: { fullConfigAtom: signals.fullConfigAtom }, createBlock: vi.fn(), openOrFocusPaneByView: vi.fn() };
});

import { setFullConfigAtom } from "@/app/store/config-signals";
import { WidgetBarSettings } from "./widget-bar-settings";

function w(label: string, order: number, extra: Partial<WidgetConfigType> = {}): WidgetConfigType {
    return { label, icon: "circle", "display:order": order, blockdef: { meta: { view: label.toLowerCase() } }, ...extra } as WidgetConfigType;
}

const widgets = (): Record<string, WidgetConfigType> => ({
    "defwidget@agent": w("Agent", 1, { "display:pinned": true }),
    "defwidget@terminal": w("Terminal", 2, { "display:pinned": true }),
    "defwidget@drone": w("Drone", 12),
    "defwidget@settings": w("Settings", 99),
});

function config(settings: Record<string, unknown> = {}) {
    setFullConfigAtom({ settings, widgets: widgets() } as unknown as FullConfigType);
}

function rowLabels(): string[] {
    return [...document.querySelectorAll(".widget-bar-row")].map((r) => r.getAttribute("data-widget-key") ?? "");
}

function stateOf(key: string): string | null {
    const row = document.querySelector(`.widget-bar-row[data-widget-key="${key}"]`) as HTMLElement;
    return within(row).getByRole("radio", { checked: true }).textContent;
}

describe("WidgetBarSettings", () => {
    beforeEach(() => {
        setConfig.mockClear();
        config();
    });
    afterEach(() => cleanup());

    it("lists pinned first, then More, with each entry's state", () => {
        render(() => <WidgetBarSettings />);
        expect(rowLabels()).toEqual(["defwidget@agent", "defwidget@terminal", "defwidget@drone", "defwidget@settings"]);
        expect(stateOf("defwidget@agent")).toBe("Pinned");
        expect(stateOf("defwidget@drone")).toBe("More");
    });

    it("hiding writes widget:hidden and drops the pin in one call", () => {
        render(() => <WidgetBarSettings />);
        const row = document.querySelector('.widget-bar-row[data-widget-key="defwidget@terminal"]') as HTMLElement;
        fireEvent.click(within(row).getByRole("radio", { name: "Hidden" }));
        expect(setConfig).toHaveBeenCalledTimes(1);
        expect(setConfig.mock.calls[0][1]).toEqual({ "widget:hidden": ["defwidget@terminal"], "widget:pinned": ["agent"] });
    });

    it("follows a config change from elsewhere (the bar, a file edit) with no other action", () => {
        render(() => <WidgetBarSettings />);
        expect(stateOf("defwidget@drone")).toBe("More");
        config({ "widget:pinned": ["drone", "agent"], "widget:hidden": ["defwidget@terminal"] });
        expect(rowLabels()).toEqual(["defwidget@drone", "defwidget@agent", "defwidget@settings", "defwidget@terminal"]);
        expect(stateOf("defwidget@drone")).toBe("Pinned");
        expect(stateOf("defwidget@terminal")).toBe("Hidden");
    });

    it("can't hide Settings", () => {
        render(() => <WidgetBarSettings />);
        const row = document.querySelector('.widget-bar-row[data-widget-key="defwidget@settings"]') as HTMLElement;
        expect(within(row).getByRole("radio", { name: "Hidden" })).toBeDisabled();
        expect(within(row).getByText("always available")).toBeTruthy();
    });

    it("Reset clears both lists", () => {
        render(() => <WidgetBarSettings />);
        fireEvent.click(screen.getByRole("button", { name: /Reset/ }));
        expect(setConfig.mock.calls[0][1]).toEqual({ "widget:pinned": null, "widget:hidden": null });
    });
});
