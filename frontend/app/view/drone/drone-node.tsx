// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal, Show, For } from "solid-js";
import type { JSX } from "solid-js";
import type { DroneDefinition, DroneRun, DroneRunState } from "./drone-types";
import type { DroneViewModel } from "./drone-model";
import { stateColor, stateIcon, stateLabel, triggerIcon, triggerLabel, relativeTime, durationMs } from "./drone-utils";

interface DroneNodeProps {
    drone: DroneDefinition;
    latestRun?: DroneRun;
    model: DroneViewModel;
    selected: boolean;
    onDragStart: (e: MouseEvent) => void;
}

export function DroneNode(props: DroneNodeProps): JSX.Element {
    const [expanded, setExpanded] = createSignal(false);

    const state = (): DroneRunState | "disabled" | null => {
        if (!props.drone.enabled) return "disabled";
        return props.latestRun?.state ?? null;
    };

    const statusColor = () => stateColor(state());
    const statusIcon = () => stateIcon(state());

    const lastRunLabel = () => {
        const run = props.latestRun;
        if (!run) return "never run";
        return relativeTime(run.startedAt);
    };

    const runtimeLabel = () => {
        const run = props.latestRun;
        if (!run || run.state === "queued") return "";
        return durationMs(run.startedAt, run.endedAt);
    };

    const runSparkline = () => {
        // Phase 1: show last run state as simple icon sequence (placeholder)
        const r = props.latestRun;
        if (!r) return "";
        const icons = [r.state === "success" ? "✓" : r.state === "failed" ? "✗" : "●"];
        return icons.join(" ");
    };

    return (
        <div
            class={`drone-node${props.selected ? " drone-node--selected" : ""}${!props.drone.enabled ? " drone-node--disabled" : ""}`}
            style={{ "--status-color": statusColor() }}
            onMouseDown={props.onDragStart}
            onClick={(e) => {
                e.stopPropagation();
                props.model.selectDrone(props.drone.id);
            }}
            onDblClick={(e) => {
                e.stopPropagation();
                props.model.openEditPanel(props.drone.id);
            }}
        >
            {/* Header row */}
            <div class="drone-node__header">
                <span class="drone-node__status" title={stateLabel(state() === "disabled" ? null : state())}>
                    {statusIcon()}
                </span>
                <span class="drone-node__name" title={props.drone.name}>
                    {props.drone.name}
                </span>
                <div class="drone-node__actions">
                    <button
                        class="drone-node__btn"
                        title="Run now"
                        disabled={!props.drone.enabled}
                        onClick={(e) => { e.stopPropagation(); props.model.triggerDrone(props.drone.id); }}
                    >
                        ▶
                    </button>
                    <button
                        class="drone-node__btn"
                        title="Edit"
                        onClick={(e) => { e.stopPropagation(); props.model.openEditPanel(props.drone.id); }}
                    >
                        ⋯
                    </button>
                </div>
            </div>

            {/* Trigger + last run row */}
            <div class="drone-node__meta">
                <span class="drone-node__trigger" title={props.drone.triggers.map(triggerLabel).join(", ")}>
                    <For each={props.drone.triggers.slice(0, 2)}>
                        {(t) => <span>{triggerIcon(t)}</span>}
                    </For>
                    {" "}
                    {props.drone.triggers.length > 0 ? triggerLabel(props.drone.triggers[0]) : "no trigger"}
                </span>
                <span class="drone-node__lastrun">{lastRunLabel()}</span>
            </div>

            {/* Expanded details */}
            <Show when={expanded()}>
                <div class="drone-node__details">
                    <Show when={props.latestRun?.state === "running" || props.latestRun?.state === "retrying"}>
                        <div class="drone-node__detail-row">
                            <span class="drone-node__detail-label">Runtime</span>
                            <span class="drone-node__detail-value">{runtimeLabel()}</span>
                        </div>
                        <Show when={props.latestRun?.attempt > 1}>
                            <div class="drone-node__detail-row">
                                <span class="drone-node__detail-label">Attempt</span>
                                <span class="drone-node__detail-value">
                                    {props.latestRun?.attempt}/{props.latestRun?.maxAttempts}
                                </span>
                            </div>
                        </Show>
                    </Show>
                    <div class="drone-node__sparkline" title="Recent runs">
                        {runSparkline()}
                    </div>
                    <div class="drone-node__detail-btns">
                        <button
                            class="drone-node__detail-btn"
                            onClick={(e) => { e.stopPropagation(); props.model.selectDrone(props.drone.id); props.model.toggleRunLog(); }}
                        >
                            View logs
                        </button>
                        <button
                            class="drone-node__detail-btn"
                            onClick={(e) => { e.stopPropagation(); props.model.openEditPanel(props.drone.id); }}
                        >
                            Edit
                        </button>
                        <button
                            class="drone-node__detail-btn drone-node__detail-btn--danger"
                            onClick={(e) => { e.stopPropagation(); props.model.toggleEnabled(props.drone.id); }}
                        >
                            {props.drone.enabled ? "Disable" : "Enable"}
                        </button>
                    </div>
                </div>
            </Show>

            {/* Expand toggle */}
            <button
                class="drone-node__expand"
                title={expanded() ? "Collapse" : "Expand"}
                onClick={(e) => { e.stopPropagation(); setExpanded(!expanded()); }}
            >
                {expanded() ? "▲" : "▼"}
            </button>
        </div>
    );
}
