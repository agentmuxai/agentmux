// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Shortcuts pane leaves out entries hidden in Settings → Widgets, and drops back to the top level
 *  when the group it is drilled into gets hidden (SPEC_WIDGET_BAR_VISIBILITY_SETTINGS_2026_10_12.md). */

import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/global", async () => {
    const signals = await import("@/app/store/config-signals");
    return { atoms: { fullConfigAtom: signals.fullConfigAtom }, replaceBlock: vi.fn(), createBlock: vi.fn(), openOrFocusPaneByView: vi.fn() };
});
vi.mock("@/store/global", async () => {
    const signals = await import("@/app/store/config-signals");
    return { atoms: { fullConfigAtom: signals.fullConfigAtom }, createBlock: vi.fn(), openOrFocusPaneByView: vi.fn() };
});

vi.mock("@/layout/index", () => ({ getLayoutModelForStaticTab: vi.fn() }));

import { setFullConfigAtom } from "@/app/store/config-signals";
import { ShortcutsViewModel } from "./shortcuts";

const w = (label: string, extra: Partial<WidgetConfigType> = {}): WidgetConfigType =>
    ({ label, icon: "circle", "display:order": 1, blockdef: { meta: { view: label.toLowerCase() } }, ...extra }) as WidgetConfigType;
const widgets = (): Record<string, WidgetConfigType> => ({
    "defwidget@agent": w("Agent"),
    "defwidget@drone": w("Drone"),
    "defwidget@messengers": w("Messengers", { children: ["discord"] }),
    "defwidget@discord": w("Discord", { "display:grouponly": true }),
});
const labels = (m: ShortcutsViewModel) => m.filteredWidgets().map((x) => x.label);

describe("Shortcuts and widget:hidden", () => {
    it("leaves hidden entries out of the top-level grid", () => {
        setFullConfigAtom({ settings: { "widget:hidden": ["defwidget@drone"] }, widgets: widgets() } as unknown as FullConfigType);
        const m = new ShortcutsViewModel("b1");
        expect(labels(m)).toEqual(["Agent", "Messengers"]);
    });

    it("drops back to the top level when the group it is in gets hidden", () => {
        setFullConfigAtom({ settings: {}, widgets: widgets() } as unknown as FullConfigType);
        const m = new ShortcutsViewModel("b1");
        m.drillInto(widgets()["defwidget@messengers"]);
        expect(labels(m)).toEqual(["Discord"]);
        setFullConfigAtom({ settings: { "widget:hidden": ["defwidget@messengers"] }, widgets: widgets() } as unknown as FullConfigType);
        expect(labels(m)).toEqual(["Agent", "Drone"]);
    });
});
