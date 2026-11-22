// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Swarm Supervisor on an agent row: the chip that shows and sets the mode,
 * and the bucket of its recent decisions.
 * docs/specs/SPEC_SWARM_SUPERVISOR_2026_10_16.md §3.8.
 */

import clsx from "clsx";
import { createMemo, createSignal, For, Show, type JSX } from "solid-js";
import { Button, openMenuAt, type UiTone } from "@/app/element/ui";
import { settingsAtom } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import {
    MODE_DESCRIPTIONS,
    MODE_LABELS,
    readSupervisorMode,
    readSupervisorState,
    supervisorChipDetail,
    supervisorEnabled,
    supervisorSupported,
    SUPERVISOR_MODES,
    SUPERVISOR_OFF_REASON,
    SUPERVISOR_SETTINGS_LABEL,
} from "@/app/store/supervisor";
import { t } from "@/app/i18n/i18n";
import { openSettingsAt } from "@/app/view/settings/open-settings";
import type { SupervisorDecision } from "@/types/rpc/SupervisorDecision";
import type { SupervisorMode } from "@/types/rpc/SupervisorMode";
import type { SupervisorState } from "@/types/rpc/SupervisorState";
import { tip } from "@/app/element/tip";

type Meta = Record<string, unknown> | undefined;

/**
 * The mode menu: a row per mode (its name, then what it does on a muted line below), a divider, and "Supervisor settings…",
 * which opens the Settings pane at the Supervisor section and is always offered (it is the way to turn the Supervisor
 * on). `current` checks the agent's own mode; null is a menu for a selection, where none is checked. While the
 * Supervisor is switched off, Steady and Drive can't be chosen (srv refuses them too; Off is always offered).
 */
export function supervisorMenuItems(
    current: SupervisorMode | null,
    pick: (mode: SupervisorMode) => void,
    enabled = true
): ContextMenuItem[] {
    return [
        ...SUPERVISOR_MODES.map((mode) => ({
            label: MODE_LABELS[mode],
            description: MODE_DESCRIPTIONS[mode],
            ...(current === null ? {} : { type: "radio" as const, checked: mode === current }),
            enabled: enabled || mode === "off",
            click: () => pick(mode),
        })),
        { type: "separator" as const },
        { label: SUPERVISOR_SETTINGS_LABEL, icon: "gear", click: () => void openSettingsAt("supervisor") },
    ];
}

/** The lines above the mode menu's rows: what it sets, and once (not per row) why some rows are off. */
export function supervisorMenuHeader(enabled: boolean, forWhom = ""): string[] {
    const title = forWhom ? t("swarm.supervisor.menuTitleFor", { who: forWhom }) : t("swarm.supervisor.menuTitle");
    return enabled ? [title] : [title, SUPERVISOR_OFF_REASON];
}

/**
 * The chip's tooltip: the mode, what it does, and the last decision. Switched off: that, first, and for an agent set to
 * Steady or Drive what it is set to, which applies again when the Supervisor is turned on.
 */
export function supervisorChipTitle(mode: SupervisorMode, state: SupervisorState | null, enabled = true): string {
    if (!enabled) {
        const lines = [t("swarm.supervisor.chipOff")];
        if (mode !== "off") lines.push(t("swarm.supervisor.chipOffMode", { mode: MODE_LABELS[mode] }));
        lines.push(t("swarm.supervisor.chipMenu"));
        return lines.join("\n");
    }
    const lines = [`Supervisor: ${MODE_LABELS[mode]}`, MODE_DESCRIPTIONS[mode]];
    if (mode !== "off" && state?.reason) lines.push(state.reason);
    lines.push("Click to change.");
    return lines.join("\n");
}

/**
 * The chip on a Claude agent's row. Off shows only the icon, muted; a mode
 * shows its name and what the Supervisor is doing ("judging", "3/15",
 * "paused"). Clicking opens the mode menu; a refusal shows as the tooltip.
 */
export function SupervisorChip(props: {
    meta: Meta;
    live: boolean;
    onSet: (mode: SupervisorMode) => Promise<string | null>;
}): JSX.Element {
    const mode = createMemo(() => readSupervisorMode(props.meta));
    const state = createMemo(() => readSupervisorState(props.meta));
    const detail = createMemo(() => supervisorChipDetail(state(), mode(), props.live));
    const [error, setError] = createSignal<string | null>(null);
    // Off stays quiet; a mode shows in the accent; a pause asks for attention.
    const tone = (): UiTone =>
        error() ? "danger" : !on() ? "quiet" : detail() === "paused" ? "attention" : mode() === "off" ? "quiet" : "accent";
    // The master switch is the person's, in Settings: while it is off every chip says so, whatever the agent's own mode.
    const on = () => supervisorEnabled(settingsAtom());
    const open = (e: MouseEvent) => {
        e.stopPropagation();
        e.preventDefault();
        openMenuAt(
            supervisorMenuItems(
                mode(),
                (next) => {
                    setError(null);
                    void props.onSet(next).then(setError);
                },
                on()
            ),
            e,
            { header: supervisorMenuHeader(on()) }
        );
    };
    return (
        <Show when={supervisorSupported(props.meta)}>
            <Button
                tone={tone()}
                density="compact"
                // Switched off: a different shape (the person with a slash), not only a duller tone.
                icon={on() ? "user-shield" : "user-slash"}
                class={clsx(
                    "swarm-supervisor-chip",
                    `swarm-supervisor-chip--${mode()}`,
                    !on() && "swarm-supervisor-chip--switched-off",
                    detail() === "paused" && "swarm-supervisor-chip--paused",
                    !!error() && "swarm-supervisor-chip--error"
                )}
                title={error() ?? supervisorChipTitle(mode(), state(), on())}
                aria-label={on() ? `Supervisor: ${MODE_LABELS[mode()]}` : t("swarm.supervisor.chipOffLabel", { mode: MODE_LABELS[mode()] })}
                onClick={open}
                onContextMenu={open}
            >
                <Show when={mode() !== "off" && on()}>
                    <span class="swarm-supervisor-mode">{MODE_LABELS[mode()]}</span>
                </Show>
                <Show when={detail() && on()}>
                    <span class="swarm-supervisor-detail">{detail()}</span>
                </Show>
            </Button>
        </Show>
    );
}

/** The decisions shown in the row's Supervisor bucket, newest first. Only
 *  while a mode is set: an Off pane's history stays in meta, out of the way. */
export function supervisorRows(meta: Meta): SupervisorDecision[] {
    if (readSupervisorMode(meta) === "off") return [];
    return [...(readSupervisorState(meta)?.history ?? [])].reverse();
}

const ACTION_LABELS: Record<SupervisorDecision["action"], string> = {
    continued: "continued",
    paused: "paused",
    done: "done",
    waited: "waiting",
    no_verdict: "no verdict",
    mode_set: "set",
    stopped: "stopped by you",
};

function timeOf(ms: number): string {
    return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

export function SupervisorBucket(props: { rows: SupervisorDecision[] }): JSX.Element {
    return (
        <Show when={props.rows.length > 0}>
            <div class="swarm-bucket swarm-bucket--supervisor">
                <div class="swarm-bucket-header">
                    <span class="swarm-bucket-label">Supervisor</span>
                    <span class="swarm-bucket-count">{props.rows.length}</span>
                </div>
                <For each={props.rows}>
                    {(row) => (
                        <div
                            classList={{
                                "swarm-supervisor-row": true,
                                [`swarm-supervisor-row--${row.action}`]: true,
                            }}
                        >
                            <span class="swarm-supervisor-time">{timeOf(row.at_ms)}</span>
                            <span class="swarm-supervisor-action">{ACTION_LABELS[row.action]}</span>
                            <span class="swarm-supervisor-reason" {...tip(row.reason ?? "")}>
                                {row.reason ?? ""}
                            </span>
                        </div>
                    )}
                </For>
            </div>
        </Show>
    );
}

/** Sets one agent's Supervisor mode, from its row; a refusal (not a Claude
 *  agent) comes back for the chip to show. */
export async function setSupervisorMode(blockId: string, mode: SupervisorMode): Promise<string | null> {
    try {
        const result = await RpcApi.SupervisorSetModeCommand(TabRpcClient, { targets: [blockId], mode });
        return result.failed[0]?.error ?? null;
    } catch (e) {
        return e instanceof Error ? e.message : String(e);
    }
}
