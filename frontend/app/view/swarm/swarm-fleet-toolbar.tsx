// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Fleet control toolbar + confirm modal + results panel for the Swarm pane.
// See docs/specs/SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md.

import { actionableKeys } from "./swarm-filter-state";
import { createMemo, createSignal, For, Show, type JSX } from "solid-js";
import { t } from "@/app/i18n/i18n";
import { ConfirmModal } from "@/app/element/confirm-modal";
import { settingsAtom } from "@/app/store/global";
import { supervisorEnabled } from "@/app/store/supervisor";
import { openSettingsAt } from "@/app/view/settings/open-settings";
import { supervisorMenuHeader, supervisorMenuItems } from "./swarm-supervisor";
import { localTarget, partitionTargets, remoteFleetTargets, unavailableReason, type FleetAction } from "./swarm-fleet-targets";
import type { FleetResultEntry, SwarmViewModel } from "./swarm-model";
import { remoteSections } from "./swarm-remote";
import { Button, CloseButton, openMenuAt, Tag, TextInput } from "@/app/element/ui";

// Staged rollout only offered once a selection is large enough that
// blast-radius capping is actually meaningful (spec §5.3) — for a
// handful of targets the whole point of staging (canary-first,
// abort-on-bad-batch) doesn't apply.
const STAGING_ELIGIBLE_AT = 5;
const DEFAULT_BATCH_SIZE = 3;
const DEFAULT_MAX_FAIL_PERCENTAGE = 50;

/** Shown on Stop when every selected agent is on another machine: a broadcast
 *  reaches a LAN peer's agents, a stop doesn't yet. */
const nothingReachable = () => t("swarm.fleet.nothingReachable");

/** Shown when no selected agent is this AgentMux's own: each AgentMux
 *  supervises its own agents. */
const nothingSupervisable = () => t("swarm.fleet.nothingSupervisable");

/** The one read-only line the Swarm keeps about the Supervisor, only while it is off: the switch is in Settings. */
function SupervisorOffLine(): JSX.Element {
    return (
        <Show when={!supervisorEnabled(settingsAtom())}>
            <span class="swarm-fleet-supervisor-off">
                {t("swarm.supervisor.offLine")}{" "}
                <Button variant="link" pending={false} onClick={() => void openSettingsAt("supervisor")}>
                    {t("swarm.supervisor.turnOn")}
                </Button>
            </span>
        </Show>
    );
}

export function SuperviseActions(props: {
    model: SwarmViewModel;
    /** Every currently-listed agent's blockId. Passed down rather than read via
     *  `model.buildTree()` here — SwarmView already computes this from the same
     *  memo the row list renders from, so the toolbar and the rows never
     *  disagree about what's selectable. */
    allBlockIds: () => string[];
}): JSX.Element {
    const { model, allBlockIds } = props;
    // What an action reaches is the selected agents the filter shows; a selected agent it hides
    // stays selected, is counted, and is not included (swarm-filter-state.ts).
    const selected = () => new Set(actionableKeys(model, model.selectedBlockIdsAtom()));
    const count = () => model.selectedBlockIdsAtom().size;
    // Agents on other instances in the selection, and how many of the selected
    // an action can reach. Cheap: only the other instances' list is read, not
    // this instance's tree (docs/specs/SPEC_SWARM_REMOTE_AGENTS_PLATFORM_TAG_AND_SELECTION_2026_10_03.md §5).
    const remoteMap = createMemo(() => remoteFleetTargets(remoteSections(model.otherInstancesAtom())));
    // Every listed target: the remote ones, and this instance's by key. A
    // selected key in neither counts as unreachable, as the action treats it.
    const known = createMemo(() => {
        const map = new Map(remoteMap());
        for (const key of allBlockIds()) if (!map.has(key)) map.set(key, localTarget(key));
        return map;
    });
    const split = (action: FleetAction) => partitionTargets([...selected()], action, known());
    const reachable = (action: FleetAction) => split(action).reachable.length;
    const agents = (n: number) => t("swarm.fleet.agentCount", { count: n });

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

    // Stop and the mode menu, on the selection's row, there while something is selected: they say how many
    // agents they reach and are off when it is none. How many are selected is beside Select all / none (swarm-select-toggle.tsx).
    return (
        <>
            <div class="swarm-fleet-toolbar">
                <Button
                    tone="danger"
                    class="swarm-fleet-btn swarm-fleet-btn--destructive"
                    disabled={model.fleetActionInFlightAtom() || reachable("stop") === 0}
                    title={count() === 0 ? t("swarm.fleet.selectToStop") : reachable("stop") === 0 ? nothingReachable() : undefined}
                    onClick={() => setStopConfirmOpen(true)}
                >
                    <i class="fa-solid fa-stop" /> {t("swarm.fleet.stopN", { n: reachable("stop") })}
                </Button>
                {/* Sets the Supervisor's mode on the selection, from a menu of the three modes. Not
                    destructive, so no confirm, like Broadcast; the result panel reports each agent
                    (docs/specs/SPEC_SWARM_SUPERVISOR_2026_10_16.md §3.8). */}
                <Button
                    class="swarm-fleet-btn"
                    disabled={model.fleetActionInFlightAtom() || reachable("supervise") === 0}
                    title={count() === 0 ? t("swarm.fleet.selectToSupervise") : reachable("supervise") === 0 ? nothingSupervisable() : undefined}
                    onClick={(e: MouseEvent) => {
                        // The same menu as an agent's chip, for the selection: switched off, srv refuses Steady and Drive.
                        const on = supervisorEnabled(settingsAtom());
                        openMenuAt(
                            supervisorMenuItems(null, (mode) => void model.superviseSelection(mode), on),
                            e,
                            { header: supervisorMenuHeader(on, agents(reachable("supervise"))) }
                        );
                    }}
                >
                    <i class="fa-solid fa-user-shield" /> {t("swarm.fleet.mode")}
                </Button>
                <SupervisorOffLine />
            </div>

            <ConfirmModal
                open={stopConfirmOpen()} scope="pane"
                title={t("swarm.fleet.stopTitle", { count: reachable("stop") })}
                description={t("swarm.fleet.stopDescription")}
                destructive
                confirmLabel={t("swarm.fleet.stopN", { n: reachable("stop") })}
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
                        {t("swarm.fleet.staged")}
                    </label>
                    <Show when={useStaging()}>
                        <div class="swarm-fleet-staging-fields">
                            <label>
                                {t("swarm.fleet.batchSize")}
                                <TextInput
                                    variant="plain"
                                    type="number"
                                    min="1"
                                    value={batchSize()}
                                    onInput={(e) => setBatchSize(Number(e.currentTarget.value) || DEFAULT_BATCH_SIZE)}
                                />
                            </label>
                            <label>
                                {t("swarm.fleet.abortRate")}
                                <TextInput
                                    variant="plain"
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
        </>
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
        return actionableKeys(props.model, props.model.selectedBlockIdsAtom()).map((key) => {
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
                        <Tag size="xs">{row.platform}</Tag>
                    </Show>
                    <Show when={row.badge}>
                        <Tag size="xs">{row.badge}</Tag>
                    </Show>
                    <Show when={row.reason}>
                        <span class="swarm-fleet-target-reason">{row.reason}</span>
                    </Show>
                </div>
            )}
        </For>
    );
}

/** The result panel's title for an action; read where it is drawn, so it follows the language. */
function fleetActionTitle(action: FleetResultEntry["action"]): string {
    switch (action) {
        case "broadcast":
            return t("swarm.fleet.titleBroadcast");
        case "bulk-stop":
            return t("swarm.fleet.titleBulkStop");
        default:
            return t("swarm.fleet.titleSupervisor");
    }
}

/** `quietWhenClean`: a broadcast's result shows only when something failed or was kept, since the Broadcast
 *  input says "Sent to N" itself. Another action's result always shows. */
export function FleetResultPanel(props: { model: SwarmViewModel; quietWhenClean?: boolean }): JSX.Element {
    const model = props.model;
    // `quietWhenClean` follows the section while this stays mounted, so it is read through `props`.
    const entry = () => {
        const e = model.lastFleetResultAtom();
        if (!e || !props.quietWhenClean || e.action !== "broadcast") return e;
        return e.result.failed.length > 0 || (e.result.held ?? []).length > 0 || e.result.aborted_early ? e : null;
    };

    return (
        <Show when={entry()}>
            {(e) => (
                <div class="swarm-fleet-result-panel">
                    <div class="swarm-fleet-result-header">
                        <span>
                            {t("swarm.fleet.resultSummary", {
                                title: fleetActionTitle(e().action),
                                succeeded: e().result.succeeded.length,
                                failed: e().result.failed.length,
                            })}
                            <Show when={(e().result.held ?? []).length > 0}>{t("swarm.broadcast.kept", { count: (e().result.held ?? []).length })}</Show>
                            <Show when={e().result.aborted_early}> — {t("swarm.fleet.abortedEarly")}</Show>
                        </span>
                        <CloseButton size="sm" label={t("swarm.fleet.dismiss")} onClick={() => model.dismissFleetResult()} />
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
                        <For each={e().result.held ?? []}>
                            {(h) => (
                                <div class="swarm-fleet-result-row swarm-fleet-result-row--held">
                                    <i class="fa-solid fa-hourglass-half" /> {e().labels[h.id] ?? h.id} — {h.note}
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
