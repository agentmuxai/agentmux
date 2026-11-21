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

// What each button does, in a sentence that says the effect and not the label again. They are the buttons' tooltips
// (shown at once) and their accessible descriptions. Each says what the choice does to this pane's list: the rows and
// the history stay complete whatever is chosen, a view only changes what the list shows.

export const PRESET_TIP: Record<PresetName | "custom", string> = {
    full: "Shows everything as it comes: no row is folded or hidden by the view.",
    standard: "Folds thinking, tool calls and jekts to one line each, hides setup rows, and keeps notes.",
    replies: "Shows only the conversation: hides thinking, tool calls, jekts, notes and setup rows.",
    custom: "Lit when the switches below match no preset. Change a switch to make your own mix; this cannot be picked.",
};

/** The rows a switch governs, as it names them in a sentence. */
const ROWS: Record<SwitchKey, string> = {
    thinking: "thinking",
    tools: "tool calls",
    jekts: "jekts and agent messages",
    notes: "AgentMux notes",
    context: "startup context and setup rows",
};

/** What a row waits on the person for, or says is never hidden: a switch never hides these. */
const NEVER_HIDDEN: Partial<Record<SwitchKey, string>> = {
    tools: " (one waiting for your answer stays)",
    jekts: " (a sensitive or unverified one stays)",
    notes: " (a note about something lost stays)",
};

const SWITCH_TIP: { [K in SwitchKey]: Record<string, string> } = {
    thinking: {
        auto: "Thinking shows in full, as it is written; only thinking that was cancelled starts folded.",
    },
    tools: {
        auto: "A tool call is open while it runs, then folds once it finishes and scrolls off.",
    },
    jekts: {
        auto: "Agent messages stay open; a jekt opens when it arrives and folds as it scrolls off.",
    },
    notes: {
        shown: "AgentMux's own notes appear in the conversation where they happened.",
    },
    context: {
        auto: "Startup context and setup rows show as they do by default, folded to a title.",
    },
};

/** The sentence for choosing `value` on switch `key`. */
export function switchTip(key: SwitchKey, value: string): string {
    const own = SWITCH_TIP[key][value];
    if (own) return own;
    const rows = ROWS[key];
    if (value === "expanded") return `Opens every one of the ${rows}; you can still fold one by clicking it.`;
    if (value === "collapsed") return `Folds every one of the ${rows} to one line; click one to open it.`;
    return `Leaves the ${rows} out of the list${NEVER_HIDDEN[key] ?? ""}, counted in a line you can open.`;
}

/** The two links under the switches. */
export const FOLLOW_DEFAULT_TIP =
    "Drops this pane's own view, so it shows whatever the default view is, now and when the default changes.";
export const MAKE_DEFAULT_TIP = "Makes this view the default for every pane that follows the default, this pane included.";
/** The same link when there is nothing to change (the pane follows the default and shows it). */
export const MAKE_DEFAULT_DONE_TIP = "Nothing to change: this pane already follows the default and shows it.";

export const PRESET_OPTIONS: SegmentedOption<PresetName | "custom">[] = [
    ...PRESET_NAMES.map((name) => ({ value: name, label: PRESET_LABEL[name], tip: PRESET_TIP[name] })),
    // Lit when the switches match no preset; not a choice, so it cannot be picked.
    { value: "custom", label: PRESET_LABEL.custom, disabled: true, tip: PRESET_TIP.custom },
];

export function switchOptions(key: SwitchKey): SegmentedOption<string>[] {
    return SWITCH_VALUES[key].map((value) => ({ value, label: FOLD_LABEL[value], tip: switchTip(key, value) }));
}
