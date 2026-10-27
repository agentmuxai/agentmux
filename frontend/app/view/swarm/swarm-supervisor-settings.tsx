// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Supervisor's settings, at the top of the Swarm's Supervisor section: one line with the master
// switch (the Supervisor is off until the operator turns it on), and a panel with its limits.
// docs/specs/SPEC_SWARM_SECTIONS_SUPERVISOR_SETTINGS_AND_KANBAN_2026_10_24.md §6.2.

import { createSignal, For, Show, type JSX } from "solid-js";
import { Button } from "@/app/element/ui";
import { settingsAtom } from "@/app/store/global";
import { supervisorEnabled } from "@/app/store/supervisor";
import { NumberControl, set, SettingRow, ToggleControl } from "@/app/view/settings/settings-controls";

export interface LimitSetting {
    key: string;
    label: string;
    description: string;
    min: number;
    max: number;
    fallback: number;
}

/** The limits, with srv's defaults and bounds (`supervisor/settings.rs`). */
export const SUPERVISOR_LIMITS: LimitSetting[] = [
    {
        key: "supervisor:steady:continuations",
        label: "Steady: continuations in a row",
        description: "How many times Steady keeps an agent going before it stops for you. 1 to 100.",
        min: 1,
        max: 100,
        fallback: 5,
    },
    {
        key: "supervisor:steady:minutes",
        label: "Steady: time cap (minutes)",
        description: "Stop a Steady run after this long. 0 means no cap. 0 to 1440.",
        min: 0,
        max: 1440,
        fallback: 0,
    },
    {
        key: "supervisor:drive:continuations",
        label: "Drive: continuations in a row",
        description: "How many times Drive keeps an agent going before it stops for you. 1 to 100.",
        min: 1,
        max: 100,
        fallback: 15,
    },
    {
        key: "supervisor:drive:minutes",
        label: "Drive: time cap (minutes)",
        description: "Stop a Drive run after this long. 0 means no cap. 0 to 1440.",
        min: 0,
        max: 1440,
        fallback: 120,
    },
    {
        key: "supervisor:noprogress",
        label: "Stop after turns that used no tool",
        description: "Stop when this many continued turns in a row used no tool, in either mode. 1 to 10.",
        min: 1,
        max: 10,
        fallback: 2,
    },
];

/** A setting's value, or its default when unset. */
export function limitValue(settings: Record<string, unknown> | null | undefined, item: LimitSetting): number {
    const v = settings?.[item.key];
    return typeof v === "number" ? v : item.fallback;
}

export function SupervisorSettingsBar(): JSX.Element {
    const settings = () => settingsAtom() as Record<string, unknown> | null | undefined;
    const on = () => supervisorEnabled(settings());
    const [open, setOpen] = createSignal(false);
    return (
        <div class="swarm-supervisor-settings" classList={{ "swarm-supervisor-settings--off": !on() }}>
            <div class="swarm-supervisor-settings-line">
                <ToggleControl checked={on()} onChange={(v) => set("supervisor:enabled", v)} />
                <span class="swarm-supervisor-settings-state">{on() ? "Supervisor: on" : "Supervisor: off"}</span>
                <Show when={!on()}>
                    <span class="swarm-supervisor-settings-hint">Agents run until they stop. Turn it on to set modes.</span>
                </Show>
                <Button class="swarm-supervisor-settings-toggle" aria-expanded={open()} onClick={() => setOpen(!open())}>
                    <i class={open() ? "fa-solid fa-chevron-up" : "fa-solid fa-sliders"} /> Settings
                </Button>
            </div>
            <Show when={open()}>
                <div class="swarm-supervisor-settings-panel">
                    <For each={SUPERVISOR_LIMITS}>
                        {(item) => (
                            <SettingRow
                                id={`supervisor.${item.key.slice("supervisor:".length).replaceAll(":", "_")}`}
                                label={item.label}
                                description={`${item.description} Default ${item.fallback}.`}
                                control={
                                    <NumberControl
                                        class="setting-number"
                                        min={item.min}
                                        max={item.max}
                                        step={1}
                                        parse="int"
                                        value={limitValue(settings(), item)}
                                        onChange={(v) => set(item.key, v)}
                                    />
                                }
                            />
                        )}
                    </For>
                    <div class="swarm-supervisor-settings-note">
                        It judges each finished turn with Haiku 4.5 through the ambient gateway. Warden's Ambient section
                        counts the judgements and failures.
                    </div>
                </div>
            </Show>
        </div>
    );
}
