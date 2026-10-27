// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Fleet control toolbar + confirm modal + results panel for the Swarm pane.
// See docs/specs/SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md.

import { createMemo, createSignal, For, Show, type JSX } from "solid-js";
import { ConfirmModal } from "@/app/element/confirm-modal";
import { ContextMenuModel } from "@/app/store/contextmenu";
import { settingsAtom } from "@/app/store/global";
import { MODE_DESCRIPTIONS, MODE_LABELS, SUPERVISOR_MODES, SUPERVISOR_OFF_REASON, supervisorEnabled } from "@/app/store/supervisor";
import { localTarget, partitionTargets, remoteFleetTargets, unavailableReason, type FleetAction } from "./swarm-fleet-targets";
import type { FleetResultEntry, SwarmViewModel } from "./swarm-model";
import { remoteSections } from "./swarm-remote";
import { Button, IconButton } from "@/app/element/ui";

// Staged rollout only offered once a selection is large enough that
// blast-radius capping is actually meaningful (spec §5.3) — for a
// handful of targets the whole point of staging (canary-first,
// abort-on-bad-batch) doesn't apply.
const STAGING_ELIGIBLE_AT = 5;
const DEFAULT_BATCH_SIZE = 3;
const DEFAULT_MAX_FAIL_PERCENTAGE = 50;

/** Shown on Stop when every selected agent is on another machine: a broadcast
 *  reaches a LAN peer's agents, a stop doesn't yet. */
const NOTHING_REACHABLE = "Stopping agents on another machine isn't supported yet";

/** Shown when no selected agent is this AgentMux's own: each AgentMux
 *  supervises its own agents. */
const NOTHING_SUPERVISABLE = "The Supervisor runs in each AgentMux for its own agents; select agents of this one";

export function FleetToolbar({
    model,
    allBlockIds,
    section = "supervisor",
}: {
    model: SwarmViewModel;
    /** The Supervisor shows Stop and the mode menu; Broadcast has the selection only (its composer is docked). */
    section?: "supervisor" | "broadcast";
    /** Every currently-listed agent's blockId, for "Select all". Passed down
     *  rather than read via `model.buildTree()` here — SwarmView already
     *  computes this from the same memo the row list renders from, so the
     *  toolbar and the rows never disagree about what's selectable. */
    allBlockIds: () => string[];
}): JSX.Element {
    const selected = () => model.selectedBlockIdsAtom();
    const count = () => selected().size;
    const allSelected = () => {
        const ids = allBlockIds();
        return ids.length > 0 && ids.every((id) => selected().has(id));
    };
    // Agents on other instances in the selection, and how many of the selected
    // an action can reach. Cheap: only the other instances' list is read, not
    // this instance's tree (docs/specs/SPEC_SWARM_REMOTE_AGENTS_PLATFORM_TAG_AND_SELECTION_2026_10_03.md §5).
    const remoteMap = createMemo(() => remoteFleetTargets(remoteSections(model.otherInstancesAtom())));
    // The count's "on other machines" leaves out this machine's other channels.
    const otherMachineN = () => {
        let n = 0;
        for (const key of selected()) if (remoteMap().get(key)?.otherMachine) n++;
        return n;
    };
    // Every listed target: the remote ones, and this instance's by key. A
    // selected key in neither counts as unreachable, as the action treats it.
    const known = createMemo(() => {
        const map = new Map(remoteMap());
        for (const key of allBlockIds()) if (!map.has(key)) map.set(key, localTarget(key));
        return map;
    });
    const split = (action: FleetAction) => partitionTargets([...selected()], action, known());
    const reachable = (action: FleetAction) => split(action).reachable.length;
    // Why "N selected" and "send to M" differ, when they do (spec §2.2).
    const plural = (n: number) => `agent${n === 1 ? "" : "s"}`;

    const [stopConfirmOpen, setStopConfirmOpen] = createSignal(false);
    const [useStaging, setUseStaging] = createSignal(false);
    const [batchSize, setBatchSize] = createSignal(DEFAULT_BATCH_SIZE);
    const [maxFailPercentage, setMaxFailPercentage] = createSignal(DEFAULT_MAX_FAIL_PERCENTAGE);

    const confirmStop = async (): Promise<void> => {
        setStopConfirmOpen(false);
        const staged = useStaging()
            ? { batch_size: Math.max(1, batchSize()), max_fail_percentage: Math.min(100, Math.max(0, maxFailPercentage())) }
            : undefined;
        await model.bulkStopSelection({ staged });
    };

    // Shown whenever there's an agent to select or a selection to act on:
    // "Select all" must be reachable with nothing selected yet.
    const showToolbar = () => allBlockIds().length > 0 || count() > 0;

    return (
        <Show when={showToolbar()}>
            <div class="swarm-fleet-toolbar">
                {/* Select all / none is reachable independent of the current
                    count — it's how a selection gets started OR cleared in
                    one click, not an action that requires one first. */}
                <Show when={allBlockIds().length > 0}>
                    <Button
                        class="swarm-fleet-btn"
                        onClick={() => (allSelected() ? model.clearSelection() : model.selectAll(allBlockIds()))}
                    >
                        <i class={allSelected() ? "fa-solid fa-square-check" : "fa-regular fa-square"} />{" "}
                        {allSelected() ? "Select none" : "Select all"}
                    </Button>
                </Show>

                {/* Never "act on selected" without stating the concrete count
                    first (spec §3: hidden scope is the top accidental-broadcast
                    cause). Selection-dependent actions below are gated on
                    count() > 0 individually; only this bar's own visibility
                    doesn't require a selection. */}
                <Show when={count() > 0}>
                    <span class="swarm-fleet-toolbar-count">
                        {count()} selected
                        <Show when={otherMachineN() > 0}> · {otherMachineN()} on other machines</Show>
                    </span>

                    <Show when={section !== "broadcast"}>
                        <Button
                            tone="danger"
                            class="swarm-fleet-btn swarm-fleet-btn--destructive"
                            disabled={model.fleetActionInFlightAtom() || reachable("stop") === 0}
                            title={reachable("stop") === 0 ? NOTHING_REACHABLE : undefined}
                            onClick={() => setStopConfirmOpen(true)}
                        >
                            <i class="fa-solid fa-stop" /> Stop {reachable("stop")}
                        </Button>
                        {/* Sets the Supervisor's mode on the selection, from a
                            menu of the three modes. Not destructive, so no
                            confirm, like Broadcast; the result panel reports
                            each agent (docs/specs/SPEC_SWARM_SUPERVISOR_2026_10_16.md §3.8). */}
                        <Button
                            class="swarm-fleet-btn"
                            disabled={model.fleetActionInFlightAtom() || reachable("supervise") === 0}
                            title={reachable("supervise") === 0 ? NOTHING_SUPERVISABLE : undefined}
                            onClick={(e: MouseEvent) =>
                                ContextMenuModel.showContextMenu(
                                    SUPERVISOR_MODES.map((mode) => {
                                        // Switched off: srv refuses Steady and Drive, so say so here (Off is always fine).
                                        const blocked = mode !== "off" && !supervisorEnabled(settingsAtom());
                                        return {
                                            label: `Supervisor: ${MODE_LABELS[mode]} for ${reachable("supervise")} ${plural(reachable("supervise"))}`,
                                            sublabel: blocked ? SUPERVISOR_OFF_REASON : MODE_DESCRIPTIONS[mode],
                                            enabled: !blocked,
                                            click: () => void model.superviseSelection(mode),
                                        };
                                    }),
                                    e
                                )
                            }
                        >
                            <i class="fa-solid fa-user-shield" /> Supervisor
                        </Button>
                    </Show>
                </Show>

                <Show when={count() > 0}>
                    <Button class="swarm-fleet-btn swarm-fleet-toolbar-clear" onClick={() => model.clearSelection()}>
                        Clear
                    </Button>
                </Show>
            </div>

            <ConfirmModal
                open={stopConfirmOpen()}
                title={`Stop ${reachable("stop")} ${plural(reachable("stop"))}?`}
                description="This stops the selected agent panes. Each pane's own stop outcome is reported individually — a partial failure never shows as a single pass/fail."
                destructive
                confirmLabel={`Stop ${reachable("stop")}`}
                onConfirm={confirmStop}
                onCancel={() => setStopConfirmOpen(false)}
            >
                <div class="swarm-fleet-confirm-target-list">
                    <Show when={stopConfirmOpen()}>
                        <StopTargetRows model={model} />
                    </Show>
                </div>
                <Show when={reachable("stop") >= STAGING_ELIGIBLE_AT}>
                    <label class="swarm-fleet-staging-toggle">
                        <input type="checkbox" checked={useStaging()} onChange={(e) => setUseStaging(e.currentTarget.checked)} />
                        Staged rollout — cap blast radius on a bad selection
                    </label>
                    <Show when={useStaging()}>
                        <div class="swarm-fleet-staging-fields">
                            <label>
                                Batch size
                                <input
                                    type="number"
                                    min="1"
                                    value={batchSize()}
                                    onInput={(e) => setBatchSize(Number(e.currentTarget.value) || DEFAULT_BATCH_SIZE)}
                                />
                            </label>
                            <label>
                                Abort if a batch's failure rate exceeds (%)
                                <input
                                    type="number"
                                    min="0"
                                    max="100"
                                    value={maxFailPercentage()}
                                    onInput={(e) => setMaxFailPercentage(Number(e.currentTarget.value) || 0)}
                                />
                            </label>
                        </div>
                    </Show>
                </Show>
            </ConfirmModal>
        </Show>
    );
}

/**
 * The stop confirmation's list: every selected agent by name, with the machine
 * and platform it is on, and for an agent the action can't reach, why. It is
 * built when the dialog opens, from the live lists, so it never shows a stale
 * agent or a block id.
 */
export function StopTargetRows(props: { model: SwarmViewModel }): JSX.Element {
    const rows = createMemo(() => {
        const known = props.model.fleetTargets();
        return Array.from(props.model.selectedBlockIdsAtom()).map((key) => {
            const t = known.get(key);
            return {
                key,
                name: t?.name ?? key,
                where: t?.where ?? null,
                platform: t?.platform ?? null,
                badge: t?.badge ?? null,
                reason: t ? unavailableReason(t, "stop") : null,
            };
        });
    });
    return (
        <For each={rows()}>
            {(row) => (
                <div classList={{ "swarm-fleet-confirm-target-row": true, "swarm-fleet-confirm-target-row--unavailable": !!row.reason }}>
                    <span class="swarm-fleet-target-name">{row.name}</span>
                    <Show when={row.where}>
                        <span class="swarm-fleet-target-where">{row.where}</span>
                    </Show>
                    <Show when={row.platform}>
                        <span class="swarm-fleet-tag">{row.platform}</span>
                    </Show>
                    <Show when={row.badge}>
                        <span class="swarm-fleet-tag">{row.badge}</span>
                    </Show>
                    <Show when={row.reason}>
                        <span class="swarm-fleet-target-reason">{row.reason}</span>
                    </Show>
                </div>
            )}
        </For>
    );
}

const FLEET_ACTION_TITLES: Record<FleetResultEntry["action"], string> = {
    broadcast: "Broadcast",
    "bulk-stop": "Bulk stop",
    supervisor: "Supervisor",
};

export function FleetResultPanel({ model }: { model: SwarmViewModel }): JSX.Element {
    const entry = () => model.lastFleetResultAtom();

    return (
        <Show when={entry()}>
            {(e) => (
                <div class="swarm-fleet-result-panel">
                    <div class="swarm-fleet-result-header">
                        <span>
                            {FLEET_ACTION_TITLES[e().action]} — {e().result.succeeded.length} succeeded,{" "}
                            {e().result.failed.length} failed
                            <Show when={e().result.aborted_early}> — staged rollout aborted early</Show>
                        </span>
                        <IconButton icon="xmark" label="Dismiss" density="compact" onClick={() => model.dismissFleetResult()} />
                    </div>
                    {/* Per-target rows, never a single aggregate line alone — see
                        this panel's own reason for existing (spec §3/§5.4). */}
                    <div class="swarm-fleet-result-rows">
                        <For each={e().result.succeeded}>
                            {(id) => (
                                <div class="swarm-fleet-result-row swarm-fleet-result-row--ok">
                                    <i class="fa-solid fa-check" /> {e().labels[id] ?? id}
                                </div>
                            )}
                        </For>
                        <For each={e().result.failed}>
                            {(f) => (
                                <div class="swarm-fleet-result-row swarm-fleet-result-row--fail">
                                    <i class="fa-solid fa-triangle-exclamation" /> {e().labels[f.id] ?? f.id} — {f.error}
                                </div>
                            )}
                        </For>
                    </div>
                </div>
            )}
        </Show>
    );
}
