// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { For, Show, type JSX } from "solid-js";

import { settingsAtom } from "@/app/store/global";
import { THEME_OPTIONS } from "@/app/menu/base-menus";
import { SectionHeader, set, SettingRow, SliderControl, ToggleControl } from "../settings-controls";

// ── Section: Appearance ───────────────────────────────────────────────────────

export function AppearanceSection(): JSX.Element {
    const s = () => settingsAtom() ?? ({} as any);
    const transparent = () => !!(s()["window:transparent"] as boolean);

    return (
        <div class="settings-section-body">
            <SettingRow
                label="Theme"
                description="UI color theme for all windows"
                control={
                    <select
                        class="setting-select"
                        value={(s()["window:theme"] as string) ?? "default"}
                        onChange={(e) => set("window:theme", e.currentTarget.value)}
                    >
                        <For each={THEME_OPTIONS}>
                            {(t) => <option value={t.id}>{t.label}</option>}
                        </For>
                    </select>
                }
            />
            <SettingRow
                label="Window transparency"
                description="Enable background transparency and blur"
                control={
                    <ToggleControl
                        checked={transparent()}
                        onChange={(v) => set("window:transparent", v)}
                    />
                }
            />
            <Show when={transparent()}>
                <SettingRow
                    indent
                    label="Opacity"
                    description="Window background opacity (35–100%)"
                    control={
                        <SliderControl
                            min={0.35} max={1} step={0.05}
                            value={(s()["window:opacity"] as number) ?? 1}
                            onChange={(v) => set("window:opacity", v)}
                        />
                    }
                />
                <SettingRow
                    indent
                    label="Background blur"
                    description="Blur the content behind the window"
                    control={
                        <ToggleControl
                            checked={!!(s()["window:blur"] as boolean)}
                            onChange={(v) => set("window:blur", v)}
                        />
                    }
                />
            </Show>
            <SettingRow
                label="Pane gap size"
                description="Pixels between tiled panes (0–20)"
                control={
                    <input
                        class="setting-number"
                        type="number" min={0} max={20}
                        value={(s()["window:tilegapsize"] as number) ?? 4}
                        onBlur={(e) => {
                            const v = parseInt(e.currentTarget.value, 10);
                            if (!isNaN(v) && v >= 0 && v <= 20) set("window:tilegapsize", v);
                        }}
                    />
                }
            />
            <SettingRow
                label="Reduce motion"
                description="Disable CSS animations and transitions"
                control={
                    <ToggleControl
                        checked={!!(s()["window:reducedmotion"] as boolean)}
                        onChange={(v) => set("window:reducedmotion", v)}
                    />
                }
            />
            <SettingRow
                label="Background color"
                description="Custom background color override (hex). Leave blank to use the theme default."
                control={
                    <input
                        class="setting-text"
                        type="text"
                        value={(s()["window:bgcolor"] as string) ?? ""}
                        placeholder="#1a1a1a"
                        onBlur={(e) => set("window:bgcolor", e.currentTarget.value || null)}
                    />
                }
            />
            <SectionHeader label="Pane hover-magnify" />
            <SettingRow
                label="Magnified opacity"
                description="Background opacity of a pane while magnified (0–1)"
                control={
                    <SliderControl
                        min={0} max={1} step={0.05}
                        value={(s()["window:magnifiedblockopacity"] as number) ?? 1}
                        onChange={(v) => set("window:magnifiedblockopacity", v)}
                    />
                }
            />
            <SettingRow
                label="Magnified size"
                description="Scale factor applied to a pane while magnified"
                control={
                    <input
                        class="setting-number setting-number--wide"
                        type="number" min={1} step={0.1}
                        value={(s()["window:magnifiedblocksize"] as number) ?? 1.5}
                        onBlur={(e) => {
                            const v = parseFloat(e.currentTarget.value);
                            if (!isNaN(v) && v >= 1) set("window:magnifiedblocksize", v);
                        }}
                    />
                }
            />
            <SettingRow
                label="Magnified blur (primary)"
                description="Backdrop blur, in pixels, applied to the magnified pane itself"
                control={
                    <input
                        class="setting-number"
                        type="number" min={0}
                        value={(s()["window:magnifiedblockblurprimarypx"] as number) ?? 0}
                        onBlur={(e) => {
                            const v = parseInt(e.currentTarget.value, 10);
                            if (!isNaN(v) && v >= 0) set("window:magnifiedblockblurprimarypx", v);
                        }}
                    />
                }
            />
            <SettingRow
                label="Magnified blur (secondary)"
                description="Backdrop blur, in pixels, applied to the other panes behind it"
                control={
                    <input
                        class="setting-number"
                        type="number" min={0}
                        value={(s()["window:magnifiedblockblursecondarypx"] as number) ?? 0}
                        onBlur={(e) => {
                            const v = parseInt(e.currentTarget.value, 10);
                            if (!isNaN(v) && v >= 0) set("window:magnifiedblockblursecondarypx", v);
                        }}
                    />
                }
            />
        </div>
    );
}
