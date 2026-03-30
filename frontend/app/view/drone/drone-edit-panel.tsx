// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal, For, Show } from "solid-js";
import type { JSX } from "solid-js";
import type { DroneDefinition, DroneTrigger, CronTrigger, EventTrigger, DependencyTrigger } from "./drone-types";
import type { DroneViewModel } from "./drone-model";
import { cronToHuman } from "./drone-utils";

interface EditPanelProps {
    model: DroneViewModel;
}

export function DroneEditPanel(props: EditPanelProps): JSX.Element {
    const drone = () => props.model.editingDroneAtom();

    function update(patch: Partial<DroneDefinition>) {
        const cur = drone();
        if (!cur) return;
        props.model["setEditingDrone"]({ ...cur, ...patch });
    }

    function updatePolicy(patch: Partial<DroneDefinition["runPolicy"]>) {
        const cur = drone();
        if (!cur) return;
        update({ runPolicy: { ...cur.runPolicy, ...patch } });
    }

    function addTrigger() {
        const cur = drone();
        if (!cur) return;
        update({ triggers: [...cur.triggers, { type: "manual" } as DroneTrigger] });
    }

    function removeTrigger(idx: number) {
        const cur = drone();
        if (!cur) return;
        const triggers = cur.triggers.filter((_, i) => i !== idx);
        update({ triggers });
    }

    function updateTrigger(idx: number, patch: Partial<DroneTrigger>) {
        const cur = drone();
        if (!cur) return;
        const triggers = cur.triggers.map((t, i) => {
            if (i !== idx) return t;
            // If the type is changing, reset to a clean trigger of the new type
            // to avoid stale type-specific fields (e.g. `expr` after switching from cron)
            if (patch.type && patch.type !== t.type) {
                return makeTriggerOfType(patch.type as DroneTrigger["type"]);
            }
            return { ...t, ...patch } as DroneTrigger;
        });
        update({ triggers });
    }

    function makeTriggerOfType(type: DroneTrigger["type"]): DroneTrigger {
        switch (type) {
            case "cron":       return { type: "cron", expr: "0 9 * * 1-5" };
            case "event":      return { type: "event", eventName: "" };
            case "dependency": return { type: "dependency", droneId: "", on: "success" };
            case "manual":     return { type: "manual" };
        }
    }

    return (
        <Show when={drone()}>
            <div class="drone-edit-panel">
                <div class="drone-edit-panel__header">
                    <span class="drone-edit-panel__title">
                        {drone()!.name || "New Drone"}
                    </span>
                    <button class="drone-edit-panel__close" onClick={() => props.model.closeEditPanel()}>
                        ✕
                    </button>
                </div>

                <div class="drone-edit-panel__body">
                    {/* Identity */}
                    <section class="drone-edit-section">
                        <h3 class="drone-edit-section__title">Identity</h3>
                        <label class="drone-edit-label">
                            Name
                            <input
                                class="drone-edit-input"
                                type="text"
                                value={drone()!.name}
                                onInput={(e) => update({ name: e.currentTarget.value })}
                                placeholder="Drone name"
                            />
                        </label>
                        <label class="drone-edit-label">
                            Description
                            <input
                                class="drone-edit-input"
                                type="text"
                                value={drone()!.description ?? ""}
                                onInput={(e) => update({ description: e.currentTarget.value })}
                                placeholder="Optional description"
                            />
                        </label>
                    </section>

                    {/* Task */}
                    <section class="drone-edit-section">
                        <h3 class="drone-edit-section__title">Task</h3>
                        <p class="drone-edit-hint">
                            The prompt sent to the agent on each run.
                            Variables: <code>{"{{date}}"}</code>, <code>{"{{trigger_type}}"}</code>, <code>{"{{run_number}}"}</code>
                        </p>
                        <textarea
                            class="drone-edit-textarea"
                            value={drone()!.task}
                            onInput={(e) => update({ task: e.currentTarget.value })}
                            rows={4}
                            placeholder="Describe the automated task..."
                        />
                    </section>

                    {/* Triggers */}
                    <section class="drone-edit-section">
                        <h3 class="drone-edit-section__title">Triggers</h3>
                        <For each={drone()!.triggers}>
                            {(trigger, idx) => (
                                <TriggerRow
                                    trigger={trigger}
                                    onUpdate={(p) => updateTrigger(idx(), p)}
                                    onRemove={() => removeTrigger(idx())}
                                />
                            )}
                        </For>
                        <button class="drone-edit-add-btn" onClick={addTrigger}>
                            + Add trigger
                        </button>
                    </section>

                    {/* Run policy */}
                    <section class="drone-edit-section">
                        <h3 class="drone-edit-section__title">Run Policy</h3>
                        <div class="drone-edit-row">
                            <label class="drone-edit-label drone-edit-label--short">
                                Retries
                                <input
                                    class="drone-edit-input drone-edit-input--num"
                                    type="number"
                                    min="0"
                                    max="10"
                                    value={drone()!.runPolicy.retryCount}
                                    onInput={(e) => updatePolicy({ retryCount: parseInt(e.currentTarget.value) || 0 })}
                                />
                            </label>
                            <label class="drone-edit-label drone-edit-label--short">
                                Retry delay (s)
                                <input
                                    class="drone-edit-input drone-edit-input--num"
                                    type="number"
                                    min="0"
                                    value={drone()!.runPolicy.retryDelaySecs}
                                    onInput={(e) => updatePolicy({ retryDelaySecs: parseInt(e.currentTarget.value) || 30 })}
                                />
                            </label>
                            <label class="drone-edit-label drone-edit-label--short">
                                Timeout (min)
                                <input
                                    class="drone-edit-input drone-edit-input--num"
                                    type="number"
                                    min="0"
                                    value={drone()!.runPolicy.timeoutMins}
                                    onInput={(e) => updatePolicy({ timeoutMins: parseInt(e.currentTarget.value) || 0 })}
                                />
                            </label>
                        </div>
                    </section>
                </div>

                {/* Footer actions */}
                <div class="drone-edit-panel__footer">
                    <Show when={drone()!.id && props.model.dronesAtom().find((d) => d.id === drone()!.id)}>
                        <button
                            class="drone-edit-btn drone-edit-btn--danger"
                            onClick={() => { props.model.deleteDrone(drone()!.id); }}
                        >
                            Delete
                        </button>
                    </Show>
                    <div style={{ flex: 1 }} />
                    <button class="drone-edit-btn" onClick={() => props.model.closeEditPanel()}>
                        Cancel
                    </button>
                    <button
                        class="drone-edit-btn drone-edit-btn--primary"
                        onClick={() => props.model.saveDrone(drone()!)}
                    >
                        Save
                    </button>
                </div>
            </div>
        </Show>
    );
}

// ── Trigger row ───────────────────────────────────────────────────────────────

interface TriggerRowProps {
    trigger: DroneTrigger;
    onUpdate: (patch: Partial<DroneTrigger>) => void;
    onRemove: () => void;
}

function TriggerRow(props: TriggerRowProps): JSX.Element {
    return (
        <div class="drone-trigger-row">
            <select
                class="drone-edit-select"
                value={props.trigger.type}
                onChange={(e) => props.onUpdate({ type: e.currentTarget.value as DroneTrigger["type"] })}
            >
                <option value="manual">Manual</option>
                <option value="cron">Cron schedule</option>
                <option value="event">Event</option>
                <option value="dependency">Dependency</option>
            </select>

            <Show when={props.trigger.type === "cron"}>
                <div class="drone-trigger-cron">
                    <input
                        class="drone-edit-input drone-edit-input--cron"
                        type="text"
                        value={(props.trigger as CronTrigger).expr ?? ""}
                        onInput={(e) => props.onUpdate({ expr: e.currentTarget.value } as any)}
                        placeholder="0 7 * * 1-5"
                        spellcheck={false}
                    />
                    <span class="drone-trigger-cron-preview">
                        {cronToHuman((props.trigger as CronTrigger).expr ?? "")}
                    </span>
                </div>
            </Show>

            <Show when={props.trigger.type === "event"}>
                <input
                    class="drone-edit-input"
                    type="text"
                    value={(props.trigger as EventTrigger).eventName ?? ""}
                    onInput={(e) => props.onUpdate({ eventName: e.currentTarget.value } as any)}
                    placeholder="e.g. deploy.failed"
                />
            </Show>

            <Show when={props.trigger.type === "dependency"}>
                <select
                    class="drone-edit-select"
                    value={(props.trigger as DependencyTrigger).on ?? "success"}
                    onChange={(e) => props.onUpdate({ on: e.currentTarget.value as any })}
                >
                    <option value="success">on success</option>
                    <option value="failure">on failure</option>
                    <option value="any">on any completion</option>
                </select>
            </Show>

            <button class="drone-trigger-remove" onClick={props.onRemove} title="Remove trigger">
                ✕
            </button>
        </div>
    );
}
