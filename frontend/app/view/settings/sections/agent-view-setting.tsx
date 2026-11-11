// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Settings → Agents → Agent view: the default for what an agent pane shows (`agent:view`,
// docs/specs/SPEC_AGENT_PANE_VERBOSITY_MODES_2026_10_29.md §4.5). A pane that follows the default
// shows this view; a pane with its own view (the View popover in the pane) is not touched.

import { createEffect, createSignal, For, onCleanup, type JSX } from "solid-js";

import { SegmentedControl } from "@/app/element/ui";
import { settingsAtom } from "@/app/store/global";
import { SettingRow, set } from "../settings-controls";
import type { SettingsIndexEntry } from "../settings-model";
import { presetOf, resolveView, SWITCH_KEYS, SWITCH_LABEL, VIEW_PRESETS, type AgentView, type PresetName } from "@/app/view/agent/virtualization/verbosity";
import { PRESET_OPTIONS, switchOptions } from "@/app/view/agent/virtualization/view-options";

export const AGENT_VIEW_SETTING = {
    id: "agents.agent_view",
    label: "What an agent pane shows",
    description:
        "The default for every agent pane that has not chosen its own: Full shows everything, Standard folds thinking, tool calls and jekts to one line and hides setup, Replies shows the conversation only. Errors and anything waiting for you always show. A pane's View menu (Alt+V cycles it) changes one pane.",
    section: "agents",
    group: "Agent view",
    keywords: ["agent view", "verbosity", "verbose", "full", "standard", "replies", "thinking", "tool calls", "jekts", "hide", "show", "collapse", "agent:view"],
} satisfies SettingsIndexEntry;

/** How long a choice stands in for the setting while its write travels, if the setting never says it. */
const WRITE_GRACE_MS = 3000;

export function AgentViewSetting(): JSX.Element {
    const saved = () => resolveView((settingsAtom() ?? {})["agent:view"], undefined).view;
    // What was just chosen, until the setting says so: the highlight moves at once, and a second
    // click before the first has come back builds on the first, not on the old setting.
    const [chosen, setChosen] = createSignal<AgentView | null>(null);
    const view = (): AgentView => chosen() ?? saved();
    let timer: ReturnType<typeof setTimeout> | undefined;
    onCleanup(() => clearTimeout(timer));
    createEffect(() => {
        const c = chosen();
        if (c && SWITCH_KEYS.every((k) => c[k] === saved()[k])) setChosen(null);
    });
    const choose = (next: AgentView) => {
        setChosen(next);
        clearTimeout(timer);
        timer = setTimeout(() => setChosen(null), WRITE_GRACE_MS);
        set("agent:view", next);
    };
    return (
        <>
            <SettingRow
                id={AGENT_VIEW_SETTING.id}
                label={AGENT_VIEW_SETTING.label}
                description={AGENT_VIEW_SETTING.description}
                control={
                    <SegmentedControl
                        ariaLabel="Default agent view"
                        density="compact"
                        options={PRESET_OPTIONS}
                        value={presetOf(view())}
                        onChange={(name) => name !== "custom" && choose({ ...VIEW_PRESETS[name as PresetName] })}
                    />
                }
            />
            <For each={SWITCH_KEYS}>
                {(key) => (
                    <SettingRow
                        indent
                        label={SWITCH_LABEL[key]}
                        control={
                            <SegmentedControl
                                ariaLabel={SWITCH_LABEL[key]}
                                density="compact"
                                options={switchOptions(key)}
                                value={view()[key]}
                                onChange={(value) => choose({ ...view(), [key]: value } as AgentView)}
                            />
                        }
                    />
                )}
            </For>
        </>
    );
}
