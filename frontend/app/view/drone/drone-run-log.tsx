// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { For, Show } from "solid-js";
import type { JSX } from "solid-js";
import type { DroneViewModel } from "./drone-model";
import { stateColor, stateIcon, relativeTime, durationMs } from "./drone-utils";

interface RunLogProps {
    model: DroneViewModel;
}

export function DroneRunLog(props: RunLogProps): JSX.Element {
    const drone = () => props.model.selectedDrone();
    const runs = () => props.model.runHistoryAtom();

    return (
        <Show when={props.model.runLogOpenAtom() && drone()}>
            <div class="drone-run-log">
                <div class="drone-run-log__header">
                    <span class="drone-run-log__title">
                        Run history — {drone()!.name}
                    </span>
                    <button class="drone-run-log__close" onClick={() => props.model.toggleRunLog()}>
                        ✕
                    </button>
                </div>
                <div class="drone-run-log__list">
                    <Show when={runs().length === 0}>
                        <div class="drone-run-log__empty">No runs yet</div>
                    </Show>
                    <For each={runs()}>
                        {(run) => (
                            <div class="drone-run-log__item">
                                <span
                                    class="drone-run-log__state"
                                    style={{ color: stateColor(run.state) }}
                                    title={run.state}
                                >
                                    {stateIcon(run.state)}
                                </span>
                                <div class="drone-run-log__info">
                                    <div class="drone-run-log__row">
                                        <span class="drone-run-log__run-num">Run #{run.runNumber}</span>
                                        <span class="drone-run-log__trigger">
                                            {run.triggerType === "manual" ? "manual" : run.triggerType}
                                        </span>
                                        <span class="drone-run-log__time">{relativeTime(run.startedAt)}</span>
                                        <Show when={run.endedAt}>
                                            <span class="drone-run-log__duration">
                                                {durationMs(run.startedAt, run.endedAt)}
                                            </span>
                                        </Show>
                                    </div>
                                    <Show when={run.attempt > 1}>
                                        <div class="drone-run-log__attempt">
                                            Attempt {run.attempt}/{run.maxAttempts}
                                        </div>
                                    </Show>
                                    <Show when={run.errorMsg}>
                                        <div class="drone-run-log__error">{run.errorMsg}</div>
                                    </Show>
                                </div>
                            </div>
                        )}
                    </For>
                </div>
            </div>
        </Show>
    );
}
