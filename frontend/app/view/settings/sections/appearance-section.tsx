// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { For, Show, type JSX } from "solid-js";

import { settingsAtom } from "@/app/store/global";
import { THEME_OPTIONS } from "@/app/menu/base-menus";
import type { SettingsIndexEntry } from "../settings-model";
import { NumberControl, SectionHeader, set, SettingRow, SliderControl, ToggleControl } from "../settings-controls";

// ── Search index — one entry per row below, named-key so re-ordering rows
// can't silently misalign an entry with the wrong row (see settings-model.ts's
// SettingsIndexEntry doc comment). label/description here are the single
// source of truth the JSX reads from — see each row's `label={...}` below. ──

export const APPEARANCE_SETTINGS = {
    theme: {
        id: "appearance.theme",
        label: "Theme",
        description: "UI color theme for all windows",
        section: "appearance",
        keywords: ["dark mode", "light mode", "color scheme", "appearance mode", "window:theme"],
    },
    splash: {
        id: "appearance.splash",
        label: "Startup splash screen",
        description: "Show the AgentMux splash while the app starts. Applies at the next launch — the launcher reads this before the app itself is running.",
        section: "appearance",
        keywords: ["splash screen", "boot screen", "loading screen", "disable splash", "splash:disabled"],
    },
    transparency: {
        id: "appearance.transparency",
        label: "Window transparency",
        description: "Enable background transparency and blur",
        section: "appearance",
        keywords: ["transparent window", "glass effect", "translucent", "window:transparent"],
    },
    opacity: {
        id: "appearance.opacity",
        label: "Opacity",
        description: "Window background opacity (35–100%)",
        section: "appearance",
        keywords: ["window opacity", "transparency level", "glass opacity", "window:opacity"],
    },
    blur: {
        id: "appearance.blur",
        label: "Background blur",
        description: "Blur the content behind the window",
        section: "appearance",
        keywords: ["blur effect", "frosted glass", "backdrop blur", "window:blur"],
    },
    paneGap: {
        id: "appearance.pane_gap",
        label: "Pane gap size",
        description: "Pixels between tiled panes (0–20)",
        section: "appearance",
        keywords: ["pane spacing", "tile gap", "split gap", "pane margin", "window:tilegapsize"],
    },
    reduceMotion: {
        id: "appearance.reduce_motion",
        label: "Reduce motion",
        description: "Disable CSS animations and transitions",
        section: "appearance",
        keywords: ["disable animations", "accessibility", "motion sickness", "animation speed", "window:reducedmotion"],
    },
    bgColor: {
        id: "appearance.bg_color",
        label: "Background color",
        description: "Custom background color override (hex). Leave blank to use the theme default.",
        section: "appearance",
        keywords: ["custom background", "hex color", "window color", "window:bgcolor"],
    },
    magnifiedOpacity: {
        id: "appearance.magnified_opacity",
        label: "Magnified opacity",
        description: "Background opacity of a pane while magnified (0–1)",
        section: "appearance",
        keywords: ["zoom opacity", "focus mode opacity", "pane zoom", "window:magnifiedblockopacity"],
    },
    magnifiedSize: {
        id: "appearance.magnified_size",
        label: "Magnified size",
        description: "Scale factor applied to a pane while magnified",
        section: "appearance",
        keywords: ["zoom scale", "pane zoom factor", "magnify scale", "window:magnifiedblocksize"],
    },
    magnifiedBlurPrimary: {
        id: "appearance.magnified_blur_primary",
        label: "Magnified blur (primary)",
        description: "Backdrop blur, in pixels, applied to the magnified pane itself",
        section: "appearance",
        keywords: ["focused pane blur", "zoom blur", "window:magnifiedblockblurprimarypx"],
    },
    magnifiedBlurSecondary: {
        id: "appearance.magnified_blur_secondary",
        label: "Magnified blur (secondary)",
        description: "Backdrop blur, in pixels, applied to the other panes behind it",
        section: "appearance",
        keywords: ["background pane blur", "other panes blur", "window:magnifiedblockblursecondarypx"],
    },
} satisfies Record<string, SettingsIndexEntry>;

// ── Section: Appearance ───────────────────────────────────────────────────────

export function AppearanceSection(): JSX.Element {
    const s = () => settingsAtom() ?? ({} as any);
    const transparent = () => !!(s()["window:transparent"] as boolean);

    return (
        <div class="settings-section-body">
            <SettingRow
                id={APPEARANCE_SETTINGS.theme.id}
                label={APPEARANCE_SETTINGS.theme.label}
                description={APPEARANCE_SETTINGS.theme.description}
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
                id={APPEARANCE_SETTINGS.splash.id}
                label={APPEARANCE_SETTINGS.splash.label}
                description={APPEARANCE_SETTINGS.splash.description}
                control={
                    <ToggleControl
                        // Stored inverted (`splash:disabled`, false by default) because
                        // the launcher reads the raw file before any of this code runs;
                        // the label stays positive so the toggle reads the way the
                        // splash behaves.
                        checked={!(s()["splash:disabled"] as boolean)}
                        onChange={(v) => set("splash:disabled", !v)}
                    />
                }
            />
            <SettingRow
                id={APPEARANCE_SETTINGS.transparency.id}
                label={APPEARANCE_SETTINGS.transparency.label}
                description={APPEARANCE_SETTINGS.transparency.description}
                control={
                    <ToggleControl
                        checked={transparent()}
                        onChange={(v) => set("window:transparent", v)}
                    />
                }
            />
            <Show when={transparent()}>
                <SettingRow
                    id={APPEARANCE_SETTINGS.opacity.id}
                    indent
                    label={APPEARANCE_SETTINGS.opacity.label}
                    description={APPEARANCE_SETTINGS.opacity.description}
                    control={
                        <SliderControl
                            min={0.35} max={1} step={0.05}
                            value={(s()["window:opacity"] as number) ?? 1}
                            onChange={(v) => set("window:opacity", v)}
                        />
                    }
                />
                <SettingRow
                    id={APPEARANCE_SETTINGS.blur.id}
                    indent
                    label={APPEARANCE_SETTINGS.blur.label}
                    description={APPEARANCE_SETTINGS.blur.description}
                    control={
                        <ToggleControl
                            checked={!!(s()["window:blur"] as boolean)}
                            onChange={(v) => set("window:blur", v)}
                        />
                    }
                />
            </Show>
            <SettingRow
                id={APPEARANCE_SETTINGS.paneGap.id}
                label={APPEARANCE_SETTINGS.paneGap.label}
                description={APPEARANCE_SETTINGS.paneGap.description}
                control={
                    <NumberControl
                        class="setting-number"
                        min={0} max={20} step={1} parse="int"
                        value={(s()["window:tilegapsize"] as number) ?? 4}
                        onChange={(v) => set("window:tilegapsize", v)}
                    />
                }
            />
            <SettingRow
                id={APPEARANCE_SETTINGS.reduceMotion.id}
                label={APPEARANCE_SETTINGS.reduceMotion.label}
                description={APPEARANCE_SETTINGS.reduceMotion.description}
                control={
                    <ToggleControl
                        checked={!!(s()["window:reducedmotion"] as boolean)}
                        onChange={(v) => set("window:reducedmotion", v)}
                    />
                }
            />
            <SettingRow
                id={APPEARANCE_SETTINGS.bgColor.id}
                label={APPEARANCE_SETTINGS.bgColor.label}
                description={APPEARANCE_SETTINGS.bgColor.description}
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
                id={APPEARANCE_SETTINGS.magnifiedOpacity.id}
                label={APPEARANCE_SETTINGS.magnifiedOpacity.label}
                description={APPEARANCE_SETTINGS.magnifiedOpacity.description}
                control={
                    <SliderControl
                        min={0} max={1} step={0.05}
                        value={(s()["window:magnifiedblockopacity"] as number) ?? 1}
                        onChange={(v) => set("window:magnifiedblockopacity", v)}
                    />
                }
            />
            <SettingRow
                id={APPEARANCE_SETTINGS.magnifiedSize.id}
                label={APPEARANCE_SETTINGS.magnifiedSize.label}
                description={APPEARANCE_SETTINGS.magnifiedSize.description}
                control={
                    <NumberControl
                        class="setting-number setting-number--wide"
                        min={1} step={0.1}
                        value={(s()["window:magnifiedblocksize"] as number) ?? 1.5}
                        onChange={(v) => set("window:magnifiedblocksize", v)}
                    />
                }
            />
            <SettingRow
                id={APPEARANCE_SETTINGS.magnifiedBlurPrimary.id}
                label={APPEARANCE_SETTINGS.magnifiedBlurPrimary.label}
                description={APPEARANCE_SETTINGS.magnifiedBlurPrimary.description}
                control={
                    <NumberControl
                        class="setting-number"
                        min={0} step={1} parse="int"
                        value={(s()["window:magnifiedblockblurprimarypx"] as number) ?? 0}
                        onChange={(v) => set("window:magnifiedblockblurprimarypx", v)}
                    />
                }
            />
            <SettingRow
                id={APPEARANCE_SETTINGS.magnifiedBlurSecondary.id}
                label={APPEARANCE_SETTINGS.magnifiedBlurSecondary.label}
                description={APPEARANCE_SETTINGS.magnifiedBlurSecondary.description}
                control={
                    <NumberControl
                        class="setting-number"
                        min={0} step={1} parse="int"
                        value={(s()["window:magnifiedblockblursecondarypx"] as number) ?? 0}
                        onChange={(v) => set("window:magnifiedblockblursecondarypx", v)}
                    />
                }
            />
        </div>
    );
}
