// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Settings > Animations (docs/specs/SPEC_ANIMATIONS_SETTINGS_PAGE_2026_11_01.md §4): a choice of
// Off, Quick, Smooth or Slow (or Follow system) over editable numbers. A change applies at once in
// every window; there is no Save.

import { Button, Notice, SegmentedControl, type SegmentedOption } from "@/app/element/ui";
import {
    type AnimationPreset,
    clampPercent,
    effectiveValues,
    presetOf,
    SPEED_MAX,
    SPEED_MIN,
    SPEED_STEP,
} from "@/app/motion/animation-settings";
import { motionTier, scaledMs } from "@/app/motion/motion";
import { DURATION } from "@/app/motion/primitives";
import { editAnimation, resetAnimation, setAnimationPreset } from "@/app/store/animation-config";
import { settingsAtom } from "@/app/store/global";
import { createSignal, Show, type JSX } from "solid-js";
import { NumberControl, SectionHeader, SettingRow, SliderControl, ToggleControl } from "../settings-controls";
import type { SettingsIndexEntry } from "../settings-model";
import { AnimationsPreview } from "./animations-preview";

// ── Search index — see appearance-section.tsx's header comment for the pattern. ──

export const ANIMATIONS_SETTINGS = {
    preset: {
        id: "animations.preset",
        label: "Animations",
        description: "Turn animations off, or make them quicker, smoother or slower. Applies at once in every window.",
        section: "animations",
        keywords: ["motion", "animation speed", "disable animations", "turn off animations", "reduce motion", "transitions", "animation:preset"],
    },
    speed: {
        id: "animations.speed",
        label: "Animation duration (%)",
        description: "How long animations take, as a percent of their designed timing. 0 is off; lower is quicker.",
        section: "animations",
        group: "Advanced",
        keywords: ["animation length", "speed", "slow motion", "faster animations", "animation:speed"],
    },
    polish: {
        id: "animations.polish",
        label: "Hover and fade polish (%)",
        description: "The length of hover and focus fades and hover lifts, as a percent of their designed timing. 0 is off.",
        section: "animations",
        group: "Advanced",
        keywords: ["hover animation", "fade", "hover lift", "focus fade", "animation:polish"],
    },
    reduceMovement: {
        id: "animations.reduce_movement",
        label: "Reduce movement",
        description: "Replace slides, zooms and bounces with fades.",
        section: "animations",
        group: "Advanced",
        keywords: ["reduced motion", "less motion", "vestibular", "no sliding", "no bounce", "animation:reducemovement"],
    },
} satisfies Record<string, SettingsIndexEntry>;

const HELP: Record<AnimationPreset, string> = {
    system: "Uses your computer's setting. Windows: Settings, Accessibility, Visual effects, Animation effects. macOS: Accessibility, Display, Reduce motion.",
    off: "Nothing moves. Changes happen instantly.",
    quick: "Short, snappy animations.",
    smooth: "The default.",
    slow: "Longer, gentler animations.",
    custom: "Your own numbers, below.",
};

const LABELS: Record<AnimationPreset, string> = {
    system: "Follow system",
    off: "Off",
    quick: "Quick",
    smooth: "Smooth",
    slow: "Slow",
    custom: "Custom",
};

const CHOICES: AnimationPreset[] = ["system", "off", "quick", "smooth", "slow"];

// ── Section: Animations ───────────────────────────────────────────────────────

export function AnimationsSection(): JSX.Element {
    const s = () => settingsAtom() ?? {};
    const preset = () => presetOf(s());
    const values = () => effectiveValues(s());
    const options = (): SegmentedOption<AnimationPreset>[] =>
        // Custom shows only while it is the choice: it is what editing a number makes, not a thing to pick.
        (preset() === "custom" ? [...CHOICES, "custom" as const] : CHOICES).map((p) => ({ value: p, label: LABELS[p] }));
    const [snapped, setSnapped] = createSignal<string | null>(null);

    const menuMs = () => Math.round(scaledMs(DURATION.base));
    const readOnly = () => preset() === "system";

    /** A number typed or dragged: keep the clamped value and say so when it moved. */
    const edit = (key: "speed" | "polish", raw: number) => {
        const next = clampPercent(raw, values()[key]);
        setSnapped(next !== raw ? `${raw} is out of range: using ${next}. Use 0, or ${SPEED_MIN} to ${SPEED_MAX} in steps of ${SPEED_STEP}.` : null);
        editAnimation(s(), { [key]: next });
    };

    const reset = () => {
        setSnapped(null);
        resetAnimation();
    };

    return (
        <div class="settings-section-body animations-section">
            <SettingRow
                id={ANIMATIONS_SETTINGS.preset.id}
                label={ANIMATIONS_SETTINGS.preset.label}
                description={ANIMATIONS_SETTINGS.preset.description}
                stacked
                control={
                    <div class="animations-choice">
                        <SegmentedControl
                            options={options()}
                            value={preset()}
                            onChange={setAnimationPreset}
                            ariaLabel="Animations"
                        />
                        <p class="animations-help" data-testid="animations-help">
                            {HELP[preset()]}
                        </p>
                        <p class="animations-readout" data-testid="animations-readout">
                            {menuMs() === 0 ? "Menus open instantly" : `Menus open in ${menuMs()} ms`}
                        </p>
                    </div>
                }
            />

            <Show when={motionTier() === "reduced" && preset() !== "system"}>
                <Notice
                    icon="circle-info"
                    actions={
                        <Button onClick={() => setAnimationPreset("system")} data-testid="animations-follow-system">
                            Follow system
                        </Button>
                    }
                >
                    Your system asks for reduced motion. AgentMux is not following it because you chose {LABELS[preset()]}.
                </Notice>
            </Show>

            <SectionHeader label="Preview" />
            <AnimationsPreview />

            <SectionHeader label="Advanced" />
            <Show when={readOnly()}>
                <p class="animations-help">The numbers below are what Follow system uses now (from your system). Choose another option to set your own.</p>
            </Show>
            <SettingRow
                id={ANIMATIONS_SETTINGS.speed.id}
                label={ANIMATIONS_SETTINGS.speed.label}
                description={ANIMATIONS_SETTINGS.speed.description}
                control={
                    <Show when={!readOnly()} fallback={<span class="animations-fixed">{values().speed}%</span>}>
                        <div class="animations-number">
                            <SliderControl min={0} max={SPEED_MAX} step={SPEED_STEP} value={values().speed} onChange={(v) => edit("speed", v)} />
                            <NumberControl min={0} max={1000} step={SPEED_STEP} parse="int" value={values().speed} onChange={(v) => edit("speed", v)} />
                        </div>
                    </Show>
                }
            />
            <SettingRow
                id={ANIMATIONS_SETTINGS.polish.id}
                label={ANIMATIONS_SETTINGS.polish.label}
                description={ANIMATIONS_SETTINGS.polish.description}
                control={
                    <Show when={!readOnly()} fallback={<span class="animations-fixed">{values().polish}%</span>}>
                        <div class="animations-number">
                            <SliderControl min={0} max={SPEED_MAX} step={SPEED_STEP} value={values().polish} onChange={(v) => edit("polish", v)} />
                            <NumberControl min={0} max={1000} step={SPEED_STEP} parse="int" value={values().polish} onChange={(v) => edit("polish", v)} />
                        </div>
                    </Show>
                }
            />
            <SettingRow
                id={ANIMATIONS_SETTINGS.reduceMovement.id}
                label={ANIMATIONS_SETTINGS.reduceMovement.label}
                description={ANIMATIONS_SETTINGS.reduceMovement.description}
                control={
                    <Show when={!readOnly()} fallback={<span class="animations-fixed">{values().reduceMovement ? "On" : "Off"}</span>}>
                        <ToggleControl checked={values().reduceMovement} onChange={(v) => editAnimation(s(), { reduceMovement: v })} />
                    </Show>
                }
            />
            <Show when={snapped()}>
                <p class="animations-snap" role="status" data-testid="animations-snap">
                    {snapped()}
                </p>
            </Show>
            <div class="animations-actions">
                <Show when={preset() !== "custom" && !readOnly() && s()["animation:speed"] !== undefined}>
                    <Button onClick={() => setAnimationPreset("custom")} data-testid="animations-use-custom">
                        Use my custom values
                    </Button>
                </Show>
                <Button onClick={reset} data-testid="animations-reset">
                    Reset to default
                </Button>
            </div>
        </div>
    );
}
