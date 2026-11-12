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
import { t } from "@/app/i18n/i18n";
import { motionTier, scaledMs } from "@/app/motion/motion";
import { DURATION } from "@/app/motion/primitives";
import { editAnimation, resetAnimation, setAnimationPreset } from "@/app/store/animation-config";
import { settingsAtom } from "@/app/store/global";
import { createSignal, Show, type JSX } from "solid-js";
import { NumberControl, SectionHeader, SettingRow, SliderControl, ToggleControl } from "../settings-controls";
import { settingEntry, type SettingsIndexEntry } from "../settings-model";
import { AnimationsPreview } from "./animations-preview";

// ── Search index — see appearance-section.tsx's header comment for the pattern. ──

export const ANIMATIONS_SETTINGS = {
    preset: settingEntry({
        id: "animations.preset",
        labelKey: "settings.animations.preset.label",
        descriptionKey: "settings.animations.preset.description",
        section: "animations",
        keywords: ["motion", "animation speed", "disable animations", "turn off animations", "reduce motion", "transitions", "animation:preset"],
    }),
    speed: settingEntry({
        id: "animations.speed",
        labelKey: "settings.animations.speed.label",
        descriptionKey: "settings.animations.speed.description",
        section: "animations",
        groupKey: "settings.animations.group.advanced",
        keywords: ["animation length", "speed", "slow motion", "faster animations", "animation:speed"],
    }),
    polish: settingEntry({
        id: "animations.polish",
        labelKey: "settings.animations.polish.label",
        descriptionKey: "settings.animations.polish.description",
        section: "animations",
        groupKey: "settings.animations.group.advanced",
        keywords: ["hover animation", "fade", "hover lift", "focus fade", "animation:polish"],
    }),
    reduceMovement: settingEntry({
        id: "animations.reduce_movement",
        labelKey: "settings.animations.reduceMovement.label",
        descriptionKey: "settings.animations.reduceMovement.description",
        section: "animations",
        groupKey: "settings.animations.group.advanced",
        keywords: ["reduced motion", "less motion", "vestibular", "no sliding", "no bounce", "animation:reducemovement"],
    }),
} satisfies Record<string, SettingsIndexEntry>;

const helpOf = (p: AnimationPreset): string =>
    ({
        system: t("settings.animations.help.system"),
        off: t("settings.animations.help.off"),
        quick: t("settings.animations.help.quick"),
        smooth: t("settings.animations.help.smooth"),
        slow: t("settings.animations.help.slow"),
        custom: t("settings.animations.help.custom"),
    })[p];

const labelOf = (p: AnimationPreset): string =>
    ({
        system: t("settings.animations.preset.system"),
        off: t("settings.animations.preset.off"),
        quick: t("settings.animations.preset.quick"),
        smooth: t("settings.animations.preset.smooth"),
        slow: t("settings.animations.preset.slow"),
        custom: t("settings.animations.preset.custom"),
    })[p];

/** The keys Custom reads: the button that returns to them shows when any is stored. */
const STORED_KEYS = ["animation:speed", "animation:polish", "animation:reducemovement"] as const;

/** How far from 0 an entry can be and still reach edit(), which clamps it and says so. */
const MAX_ENTRY = 1_000_000;

const CHOICES: AnimationPreset[] = ["system", "off", "quick", "smooth", "slow"];

// One object per option for good: the control keys its buttons by them, so a new object on each settings
// change would rebuild every button, the one the arrow key just focused included.
const BASE_OPTIONS: SegmentedOption<AnimationPreset>[] = CHOICES.map((p) => ({
    value: p,
    get label() {
        return labelOf(p);
    },
}));
// Custom shows only while it is the choice: it is what editing a number makes, not a thing to pick.
const OPTIONS_WITH_CUSTOM: SegmentedOption<AnimationPreset>[] = [...BASE_OPTIONS, {
    value: "custom",
    get label() {
        return labelOf("custom");
    },
}];

// ── Section: Animations ───────────────────────────────────────────────────────

export function AnimationsSection(): JSX.Element {
    const s = () => settingsAtom() ?? {};
    const preset = () => presetOf(s());
    const values = () => effectiveValues(s());
    const options = () => (preset() === "custom" ? OPTIONS_WITH_CUSTOM : BASE_OPTIONS);
    const [snapped, setSnapped] = createSignal<string | null>(null);
    // Bumped when a typed or dragged value was corrected: a field only shows a new value when the stored one
    // changes, so a correction back to the stored number would leave the rejected text on screen. The number
    // boxes take it as `resyncKey` and replace their text in place, so the box keeps its focus.
    const [corrected, setCorrected] = createSignal(0);

    const menuMs = () => Math.round(scaledMs(DURATION.base));
    const readOnly = () => preset() === "system";

    /** A number typed or dragged: keep the clamped value and say so when it moved. */
    const edit = (key: "speed" | "polish", raw: number) => {
        const next = clampPercent(raw, values()[key]);
        const moved = next !== raw;
        setSnapped(moved ? t("settings.animations.snapped", { raw: String(raw), next: String(next), min: String(SPEED_MIN), max: String(SPEED_MAX), step: String(SPEED_STEP) }) : null);
        if (moved) setCorrected((n) => n + 1);
        editAnimation(s(), { [key]: next });
    };

    const reset = () => {
        setSnapped(null);
        setCorrected((n) => n + 1);
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
                            ariaLabel={t("settings.animations.aria.group")}
                        />
                        <p class="animations-help" data-testid="animations-help">
                            {helpOf(preset())}
                        </p>
                        <p class="animations-readout" data-testid="animations-readout">
                            {menuMs() === 0 ? t("settings.animations.readout.instant") : t("settings.animations.readout.ms", { ms: String(menuMs()) })}
                        </p>
                    </div>
                }
            />

            <Show when={motionTier() === "reduced" && preset() !== "system"}>
                <Notice
                    icon="circle-info"
                    actions={
                        <Button onClick={() => setAnimationPreset("system")} data-testid="animations-follow-system">
                            {labelOf("system")}
                        </Button>
                    }
                >
                    {t("settings.animations.reducedNotice", { choice: labelOf(preset()) })}
                </Notice>
            </Show>

            <SectionHeader label={t("settings.animations.group.preview")} />
            <AnimationsPreview />

            <SectionHeader label={t("settings.animations.group.advanced")} />
            <Show when={readOnly()}>
                <p class="animations-help">{t("settings.animations.advancedHelp")}</p>
            </Show>
            <SettingRow
                id={ANIMATIONS_SETTINGS.speed.id}
                label={ANIMATIONS_SETTINGS.speed.label}
                description={ANIMATIONS_SETTINGS.speed.description}
                control={
                    <Show when={!readOnly()} fallback={<span class="animations-fixed">{values().speed}%</span>}>
                        <div class="animations-number">
                            <SliderControl min={0} max={SPEED_MAX} step={SPEED_STEP} value={values().speed} resyncKey={corrected()} onChange={(v) => edit("speed", v)} />
                            <NumberControl id="animations-speed-number" ariaLabel={t("settings.animations.aria.speed")} min={-MAX_ENTRY} max={MAX_ENTRY} step={SPEED_STEP} parse="int" value={values().speed} resyncKey={corrected()} onChange={(v) => edit("speed", v)} />
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
                            <SliderControl min={0} max={SPEED_MAX} step={SPEED_STEP} value={values().polish} resyncKey={corrected()} onChange={(v) => edit("polish", v)} />
                            <NumberControl id="animations-polish-number" ariaLabel={t("settings.animations.aria.polish")} min={-MAX_ENTRY} max={MAX_ENTRY} step={SPEED_STEP} parse="int" value={values().polish} resyncKey={corrected()} onChange={(v) => edit("polish", v)} />
                        </div>
                    </Show>
                }
            />
            <SettingRow
                id={ANIMATIONS_SETTINGS.reduceMovement.id}
                label={ANIMATIONS_SETTINGS.reduceMovement.label}
                description={ANIMATIONS_SETTINGS.reduceMovement.description}
                control={
                    <Show when={!readOnly()} fallback={<span class="animations-fixed">{values().reduceMovement ? t("settings.animations.on") : t("settings.animations.preset.off")}</span>}>
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
                <Show when={preset() !== "custom" && !readOnly() && STORED_KEYS.some((k) => s()[k] !== undefined)}>
                    <Button onClick={() => setAnimationPreset("custom")} data-testid="animations-use-custom">
                        {t("settings.animations.useCustom")}
                    </Button>
                </Show>
                <Button onClick={reset} data-testid="animations-reset">
                    {t("settings.animations.reset")}
                </Button>
            </div>
        </div>
    );
}
