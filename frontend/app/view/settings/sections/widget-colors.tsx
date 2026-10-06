// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Settings → Appearance → Widget colors: the color of every pane of each widget
 * type, the `pane:colors` setting.
 * docs/specs/SPEC_WIDGET_DEFAULT_PANE_COLORS_2026_10_05.md §3.5.
 */

import { For, Show, type JSX } from "solid-js";

import { setWidgetHue } from "@/app/block/pane-color-menu";
import { paneRoleColor } from "@/app/block/pane-color-scheme";
import { isLightThemeActive, widgetHueFor } from "@/app/block/pane-identity";
import { listPaneTabs, type PaneTabManifest } from "@/app/block/pane-tab-registry";
import { Button } from "@/app/element/ui";
import { getSettingsKeyAtom } from "@/app/store/block-atom-cache";
import { makeIconClass } from "@/util/util";
import { HueSwatchRow, set, SettingRow } from "../settings-controls";

/** One row per registered widget, so a widget added later gets one too. */
export function WidgetColorsSettings(p: { id: string; label: string; description: string }): JSX.Element {
    const chosen = getSettingsKeyAtom("pane:colors");
    const widgets = () => listPaneTabs().filter((m) => !m.legacyOf);
    return (
        <>
            <SettingRow
                id={p.id}
                label={p.label}
                description={p.description}
                control={
                    <Show when={Object.keys(chosen() ?? {}).length > 0}>
                        <Button onClick={() => set("pane:colors", null)}>Reset all</Button>
                    </Show>
                }
            />
            <For each={widgets()}>{(m) => <WidgetColorRow manifest={m} />}</For>
        </>
    );
}

function WidgetColorRow(p: { manifest: PaneTabManifest }): JSX.Element {
    const view = () => p.manifest.view;
    const builtIn = () => p.manifest.defaultHue ?? null;
    const stored = () => getSettingsKeyAtom("pane:colors")()?.[view()];
    // The swatch shown as selected: the user's color, else the built-in one.
    const selected = () => (stored() === undefined ? builtIn() : stored());
    const preview = (role: "pill" | "identity") => paneRoleColor(widgetHueFor(view()), undefined, isLightThemeActive(), role);
    // Picking the built-in color stores nothing, so the widget keeps following it.
    const pick = (hue: number | null) => setWidgetHue(view(), hue === builtIn() ? undefined : hue);
    return (
        <SettingRow
            indent
            label={p.manifest.label}
            control={
                <div class="setting-widget-color">
                    <span
                        class="setting-widget-color-preview"
                        style={{
                            "background-color": preview("pill"),
                            "border-bottom-color": preview("identity"),
                        }}
                    >
                        <i class={makeIconClass(p.manifest.icon, true)} />
                        {p.manifest.label}
                    </span>
                    <HueSwatchRow label={`${p.manifest.label} color`} value={selected()} onChange={pick} />
                    <Button
                        // Kept in the layout when hidden, so the swatches don't shift.
                        style={{ visibility: stored() === undefined ? "hidden" : "visible" }}
                        onClick={() => setWidgetHue(view(), undefined)}
                    >
                        Reset
                    </Button>
                </div>
            }
        />
    );
}
