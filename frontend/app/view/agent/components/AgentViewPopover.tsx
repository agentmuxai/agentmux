// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The View popover: what this agent pane shows. The three presets (Custom lights when the five
 * switches match none), one segmented control per switch, Follow default, and Make this the default
 * (docs/specs/SPEC_AGENT_PANE_VERBOSITY_MODES_2026_10_29.md §4.5). Opened from the eye in the composer's
 * control row and the palette; every change applies at once and the panel stays open.
 */

import { For, Show, type JSX } from "solid-js";
import { AnchoredPopover } from "@/app/element/anchored-popover";
import { tip } from "@/app/element/tip";
import { Button, SegmentedControl } from "@/app/element/ui";
import type { AgentViewControls } from "../agent-view-controls";
import { presetOf, SWITCH_KEYS, SWITCH_LABEL, type AgentView } from "../virtualization/verbosity";
import {
    FOLLOW_DEFAULT_TIP,
    MAKE_DEFAULT_DONE_TIP,
    MAKE_DEFAULT_TIP,
    PRESET_OPTIONS,
    switchOptions,
} from "../virtualization/view-options";
import "./AgentViewPopover.scss";

export interface AgentViewPopoverProps {
    controls: AgentViewControls;
    /** The element the panel hangs from: the eye in the composer's control row, or the strip itself when the eye is not drawn. */
    anchor: Element | undefined;
    /** The agent pane: the panel stays inside it, narrowing to it when the pane is narrower than the panel. */
    pane?: HTMLElement | undefined;
}

export function AgentViewPopover(props: AgentViewPopoverProps): JSX.Element {
    const { controls } = props;
    const view = (): AgentView => controls.resolved().view;
    const followsDefault = () => controls.resolved().source === "default";
    // The pane already says what the default says, whether it follows it or happens to match it.
    const matchesDefault = () => SWITCH_KEYS.every((k) => view()[k] === controls.defaultView()[k]);
    const nothingToMake = () => followsDefault() && matchesDefault();

    return (
        <AnchoredPopover
            anchor={props.anchor}
            placement="top-end"
            containWithin={() => props.pane}
            fitWidth
            fitHeight
            shellClass="anchored-popover--menu"
            class="menu agent-view-popover"
            role="dialog"
            aria-label="Agent view"
            // Its tips sit beside the panel, level with the control, so none covers another row (title-tooltip.ts).
            ref={(el) => el.setAttribute("data-hover-tip-beside", "")}
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
                    <Button variant="link" {...tip(FOLLOW_DEFAULT_TIP, { instant: true })} onClick={() => controls.followDefault()}>
                        Follow default
                    </Button>
                </Show>
                {/* Not `disabled`: a disabled button gets no hover or focus, and its tip says why there is nothing to do. */}
                <Button
                    variant="link"
                    aria-disabled={nothingToMake() ? "true" : undefined}
                    {...tip(nothingToMake() ? MAKE_DEFAULT_DONE_TIP : MAKE_DEFAULT_TIP, { instant: true })}
                    onClick={() => !nothingToMake() && controls.makeDefault()}
                >
                    Make this the default
                </Button>
            </div>
        </AnchoredPopover>
    );
}
