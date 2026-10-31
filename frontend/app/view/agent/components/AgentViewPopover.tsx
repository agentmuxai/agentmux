// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The View popover: what this agent pane shows. The three presets (Custom lights when the five
 * switches match none), one segmented control per switch, Follow default, and Make this the default
 * (docs/specs/SPEC_AGENT_PANE_VERBOSITY_MODES_2026_10_29.md §4.5). Opened from the composer chip, the
 * pane's right-click menu and the palette; every change applies at once and the panel stays open.
 */

import { For, Show, type JSX } from "solid-js";
import { AnchoredPopover } from "@/app/element/anchored-popover";
import { Button, SegmentedControl, type SegmentedOption } from "@/app/element/ui";
import type { AgentViewControls } from "../agent-view-controls";
import {
    PRESET_LABEL,
    PRESET_NAMES,
    presetOf,
    SWITCH_KEYS,
    SWITCH_LABEL,
    SWITCH_VALUES,
    type AgentView,
    type PresetName,
    type SwitchKey,
} from "../virtualization/verbosity";
import "./AgentViewPopover.scss";

const FOLD_LABEL: Record<string, string> = {
    auto: "Auto",
    expanded: "Expanded",
    collapsed: "Collapsed",
    hidden: "Hidden",
    shown: "Shown",
};

const PRESET_OPTIONS: SegmentedOption<PresetName | "custom">[] = [
    ...PRESET_NAMES.map((name) => ({ value: name, label: PRESET_LABEL[name] })),
    // Lit when the switches match no preset; not a choice, so it cannot be picked.
    { value: "custom", label: PRESET_LABEL.custom, disabled: true },
];

function switchOptions(key: SwitchKey): SegmentedOption<string>[] {
    return SWITCH_VALUES[key].map((value) => ({ value, label: FOLD_LABEL[value] }));
}

export interface AgentViewPopoverProps {
    controls: AgentViewControls;
    /** The element the panel hangs from: the composer chip, or the strip when the chip is hidden. */
    anchor: Element | undefined;
}

export function AgentViewPopover(props: AgentViewPopoverProps): JSX.Element {
    const { controls } = props;
    const view = (): AgentView => controls.resolved().view;
    const followsDefault = () => controls.resolved().source === "default";
    // The pane already says what the default says, whether it follows it or happens to match it.
    const matchesDefault = () => SWITCH_KEYS.every((k) => view()[k] === controls.defaultView()[k]);

    return (
        <AnchoredPopover
            anchor={props.anchor}
            placement="top-end"
            shellClass="anchored-popover--menu"
            class="menu agent-view-popover"
            role="dialog"
            aria-label="Agent view"
            onDismiss={() => controls.setPopoverOpen(false)}
        >
            <div class="agent-view-popover-title">View</div>
            <SegmentedControl
                class="agent-view-popover-presets"
                ariaLabel="Preset"
                density="compact"
                options={PRESET_OPTIONS}
                value={presetOf(view())}
                onChange={(name) => name !== "custom" && controls.setPreset(name)}
            />
            <For each={SWITCH_KEYS}>
                {(key) => (
                    <div class="agent-view-popover-row">
                        <span class="agent-view-popover-label">{SWITCH_LABEL[key]}</span>
                        <SegmentedControl
                            density="compact"
                            ariaLabel={SWITCH_LABEL[key]}
                            options={switchOptions(key)}
                            value={view()[key]}
                            onChange={(value) => controls.setSwitch(key, value as never)}
                        />
                    </div>
                )}
            </For>
            <div class="agent-view-popover-foot">
                <Show
                    when={!followsDefault()}
                    fallback={<span class="agent-view-popover-note">Follows the default</span>}
                >
                    <Button variant="link" onClick={() => controls.followDefault()}>
                        Follow default
                    </Button>
                </Show>
                <Button
                    variant="link"
                    disabled={followsDefault() && matchesDefault()}
                    title="Every pane that follows the default shows this view"
                    onClick={() => controls.makeDefault()}
                >
                    Make this the default
                </Button>
            </div>
        </AnchoredPopover>
    );
}
