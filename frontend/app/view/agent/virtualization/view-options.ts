// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The agent view's choices as segmented-control options, for the two places that offer them: the
 * View popover (components/AgentViewPopover.tsx) and Settings → Agents → Agent view.
 */

import type { SegmentedOption } from "@/app/element/ui";
import { PRESET_LABEL, PRESET_NAMES, SWITCH_VALUES, type PresetName, type SwitchKey } from "./verbosity";

const FOLD_LABEL: Record<string, string> = {
    auto: "Auto",
    expanded: "Expanded",
    collapsed: "Collapsed",
    hidden: "Hidden",
    shown: "Shown",
};

export const PRESET_OPTIONS: SegmentedOption<PresetName | "custom">[] = [
    ...PRESET_NAMES.map((name) => ({ value: name, label: PRESET_LABEL[name] })),
    // Lit when the switches match no preset; not a choice, so it cannot be picked.
    { value: "custom", label: PRESET_LABEL.custom, disabled: true },
];

export function switchOptions(key: SwitchKey): SegmentedOption<string>[] {
    return SWITCH_VALUES[key].map((value) => ({ value, label: FOLD_LABEL[value] }));
}
