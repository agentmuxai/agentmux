// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Supervisor's settings, at the top of the Swarm's Supervisor section: one line with the master
// switch (the Supervisor is off until the operator turns it on), and a panel with its limits.
// docs/specs/SPEC_SWARM_SECTIONS_SUPERVISOR_SETTINGS_AND_KANBAN_2026_10_24.md §6.2.

import { createEffect, createSignal, For, on as onChange, onMount, Show, type JSX } from "solid-js";
import { Button, Switch } from "@/app/element/ui";
import { settingsAtom } from "@/app/store/global";
import { supervisorEnabled } from "@/app/store/supervisor";
import { supervisorSettingsRequests, takeSupervisorSettingsRequest } from "@/app/store/supervisor-settings-request";
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
    {
        key: "supervisor:cooldown",
        label: "Wait before continuing (seconds)",
        description:
            "A continuation is sent this long after its turn ended. A turn that starts in that time (your message, a jekt, a job) or a Stop cancels it. 0 sends at once. 0 to 600.",
        min: 0,
        max: 600,
        fallback: 30,
    },
    {
        key: "supervisor:hourly",
        label: "Fleet budget (continuations per hour)",
        description:
            "The most continuations across all panes in a rolling hour. At the cap a pane stops, with the reason. 0 means no cap. 0 to 1000.",
        min: 0,
        max: 1000,
        fallback: 30,
    },
];

export interface NoticeSetting {
    key: string;
    label: string;
    description: string;
}

/** Which stops send an OS notice; the rows and the history show every stop whatever these say. */
export const SUPERVISOR_NOTICES: NoticeSetting[] = [
    {
        key: "supervisor:notify:needsyou",
        label: "Notify when it needs you",
        description: "A question, \"needs you\" or \"blocked\".",
    },
    {
        key: "supervisor:notify:limit",
        label: "Notify when it hits a limit",
        description: "Continuations, time, no progress, the same reply twice, or the fleet budget.",
    },
    {
        key: "supervisor:notify:failed",
        label: "Notify when it couldn't continue",
        description: "The message couldn't be delivered, or the agent can't run.",
    },
    {
        key: "supervisor:notify:done",
        label: "Notify when it finds the work done",
        description: "The agent's work looks finished.",
    },
];

/** A notice setting's value; every notice is on until turned off. */
export function noticeOn(settings: Record<string, unknown> | null | undefined, item: NoticeSetting): boolean {
    return settings?.[item.key] !== false;
}

/** A setting's value, or its default when unset. */
export function limitValue(settings: Record<string, unknown> | null | undefined, item: LimitSetting): number {
    const v = settings?.[item.key];
    return typeof v === "number" ? v : item.fallback;
}

/** `blockId`: this Swarm pane, which the Settings search names when it asks for the panel to open. */
export function SupervisorSettingsBar(props: { blockId?: string }): JSX.Element {
    const settings = () => settingsAtom() as Record<string, unknown> | null | undefined;
    const on = () => supervisorEnabled(settings());
    const [open, setOpen] = createSignal(false);
    // Asked to open from the Settings pane's search: at once if this is showing, else on mount.
    const answerRequest = () => {
        if (takeSupervisorSettingsRequest(props.blockId)) setOpen(true);
    };
    onMount(answerRequest);
    createEffect(onChange(supervisorSettingsRequests, answerRequest, { defer: true }));
    return (
        <div class="swarm-supervisor-settings" classList={{ "swarm-supervisor-settings--off": !on() }}>
            <div class="swarm-supervisor-settings-line">
                <Switch ariaLabel="Supervisor" checked={on()} onChange={(v) => set("supervisor:enabled", v)} />
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
                    <For each={SUPERVISOR_NOTICES}>
                        {(item) => (
                            <SettingRow
                                id={`supervisor.${item.key.slice("supervisor:".length).replaceAll(":", "_")}`}
                                label={item.label}
                                description={item.description}
                                control={
                                    <ToggleControl
                                        checked={noticeOn(settings(), item)}
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
