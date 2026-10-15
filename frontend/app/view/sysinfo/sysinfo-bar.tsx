// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Sysinfo's one control row, the same in the simple and the Advanced views: a
 * button per view to switch in one click, the window (1 to 15 minutes), and on
 * the right pause and a table of the figures instead of the charts. As the pane
 * narrows, the controls break onto the next line one at a time from the right
 * (pause and table, then the window), and a set of buttons wider than the pane
 * wraps within itself.
 */

import type { JSX } from "solid-js";
import { Show } from "solid-js";

import { IconButton, SegmentedControl } from "@/app/element/ui";

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
    /** The figures as a table instead of the charts. */
    table: boolean;
    onTable: (table: boolean) => void;
};

export function SysinfoBar(props: SysinfoBarProps): JSX.Element {
    return (
        <div class="sysinfo-bar">
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
            <div class="sysinfo-bar-actions">
                <Show when={props.paused}>
                    <span class="sysinfo-bar-paused">Paused</span>
                </Show>
                <IconButton
                    icon={props.paused ? "play" : "pause"}
                    label={props.paused ? "Resume" : "Pause"}
                    pressed={props.paused}
                    onClick={() => props.onPause(!props.paused)}
                />
                <IconButton
                    icon="table"
                    label={props.table ? "Show charts" : "Show as a table"}
                    pressed={props.table}
                    onClick={() => props.onTable(!props.table)}
                />
            </div>
        </div>
    );
}
