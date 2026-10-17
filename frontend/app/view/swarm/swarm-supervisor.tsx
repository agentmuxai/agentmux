// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Swarm Supervisor on an agent row: the chip that shows and sets the mode,
 * and the bucket of its recent decisions.
 * docs/specs/SPEC_SWARM_SUPERVISOR_2026_10_16.md §3.8.
 */

import clsx from "clsx";
import { createMemo, createSignal, For, Show, type JSX } from "solid-js";
import { Button, type UiTone } from "@/app/element/ui";
import { ContextMenuModel } from "@/app/store/contextmenu";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import {
    MODE_DESCRIPTIONS,
    MODE_LABELS,
    readSupervisorMode,
    readSupervisorState,
    supervisorChipDetail,
    supervisorSupported,
    SUPERVISOR_MODES,
} from "@/app/store/supervisor";
import type { SupervisorDecision } from "@/types/rpc/SupervisorDecision";
import type { SupervisorMode } from "@/types/rpc/SupervisorMode";
import type { SupervisorState } from "@/types/rpc/SupervisorState";

type Meta = Record<string, unknown> | undefined;

/** The mode menu: one radio item per mode, the current one checked. */
export function supervisorMenuItems(current: SupervisorMode, pick: (mode: SupervisorMode) => void): ContextMenuItem[] {
    return SUPERVISOR_MODES.map((mode) => ({
        label: `Supervisor: ${MODE_LABELS[mode]}`,
        sublabel: MODE_DESCRIPTIONS[mode],
        type: "radio" as const,
        checked: mode === current,
        click: () => pick(mode),
    }));
}

/** The chip's tooltip: the mode, what it does, and the last decision. */
export function supervisorChipTitle(mode: SupervisorMode, state: SupervisorState | null): string {
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
        error() ? "danger" : detail() === "paused" ? "attention" : mode() === "off" ? "quiet" : "accent";
    const open = (e: MouseEvent) => {
        e.stopPropagation();
        e.preventDefault();
        ContextMenuModel.showContextMenu(
            supervisorMenuItems(mode(), (next) => {
                setError(null);
                void props.onSet(next).then(setError);
            }),
            e
        );
    };
    return (
        <Show when={supervisorSupported(props.meta)}>
            <Button
                tone={tone()}
                density="compact"
                icon="user-shield"
                class={clsx(
                    "swarm-supervisor-chip",
                    `swarm-supervisor-chip--${mode()}`,
                    detail() === "paused" && "swarm-supervisor-chip--paused",
                    !!error() && "swarm-supervisor-chip--error"
                )}
                title={error() ?? supervisorChipTitle(mode(), state())}
                aria-label={`Supervisor: ${MODE_LABELS[mode()]}`}
                onClick={open}
                onContextMenu={open}
            >
                <Show when={mode() !== "off"}>
                    <span class="swarm-supervisor-mode">{MODE_LABELS[mode()]}</span>
                </Show>
                <Show when={detail()}>
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
                            <span class="swarm-supervisor-reason" title={row.reason ?? ""}>
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
