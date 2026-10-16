// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Sysinfo's one control row, the same in the simple and the Advanced views: a
 * button per view to switch in one click, the window (1 to 15 minutes), and on
 * the right a button that shows or hides those two sets (hidden until chosen),
 * pause, and a table of the figures instead of the charts. The right-hand
 * buttons sit flush in the pane's top-right corner, in their own grid column, so
 * they stay on the first line however narrow the pane gets. As the pane
 * narrows, the view and window buttons wrap onto further lines inside their
 * column, and a set of buttons wider than the column wraps within itself.
 */

import type { JSX } from "solid-js";
import { Show } from "solid-js";

import { Button, IconButton, SegmentedControl } from "@/app/element/ui";

import { windowChoices } from "./sysinfo-overview-data";

export type SysinfoBarProps = {
    /** The views this mode offers, the one shown, and a way to change it. */
    views: readonly { value: string; label: string }[];
    view: string;
    onView: (view: string) => void;
    /** The window shown now, in seconds, and a way to change it. */
    windowSecs: number;
    intervalSecs: number;
    onWindow: (secs: number) => void;
    paused: boolean;
    onPause: (paused: boolean) => void;
    /** Whether the view and window buttons are shown, and a way to change that. */
    controls: boolean;
    onControls: (shown: boolean) => void;
    /** The figures as a table instead of the charts. */
    table: boolean;
    onTable: (table: boolean) => void;
};

export function SysinfoBar(props: SysinfoBarProps): JSX.Element {
    return (
        <div class="sysinfo-bar">
            <Show when={props.controls}>
                <div class="sysinfo-bar-selectors">
                    <SegmentedControl
                        ariaLabel="View"
                        density="compact"
                        options={props.views.map((v) => ({ value: v.value, label: v.label }))}
                        value={props.view}
                        onChange={(v) => props.onView(v)}
                    />
                    <SegmentedControl
                        ariaLabel="Time window"
                        density="compact"
                        options={windowChoices(props.intervalSecs, props.windowSecs).map((c) => ({
                            value: String(c.secs),
                            label: c.label,
                        }))}
                        value={String(props.windowSecs)}
                        onChange={(v) => props.onWindow(Number(v))}
                    />
                </div>
            </Show>
            <div class="sysinfo-bar-actions">
                <Show when={props.paused}>
                    <span class="sysinfo-bar-paused">Paused</span>
                </Show>
                <Button
                    icon="sliders"
                    density="compact"
                    pressed={props.controls}
                    aria-label={props.controls ? "Hide view buttons" : "Show view buttons"}
                    onClick={() => props.onControls(!props.controls)}
                >
                    Views
                </Button>
                <IconButton
                    icon={props.paused ? "play" : "pause"}
                    density="compact"
                    label={props.paused ? "Resume" : "Pause"}
                    pressed={props.paused}
                    onClick={() => props.onPause(!props.paused)}
                />
                <IconButton
                    icon="table"
                    density="compact"
                    label={props.table ? "Show charts" : "Show as a table"}
                    pressed={props.table}
                    onClick={() => props.onTable(!props.table)}
                />
            </div>
        </div>
    );
}
