// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Fleet control toolbar + confirm modal + results panel for the Swarm pane.
// See docs/specs/SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md.

import { createMemo, createSignal, For, Show, type JSX } from "solid-js";
import { ConfirmModal } from "@/app/element/confirm-modal";
import { summarizeAmbientOutcomes } from "@/app/store/ambient-outcomes";
import { remoteFleetTargets, unavailableReason, type FleetAction } from "./swarm-fleet-targets";
import type { SwarmViewModel } from "./swarm-model";
import { remoteSections } from "./swarm-remote";

// Staged rollout only offered once a selection is large enough that
// blast-radius capping is actually meaningful (spec §5.3) — for a
// handful of targets the whole point of staging (canary-first,
// abort-on-bad-batch) doesn't apply.
const STAGING_ELIGIBLE_AT = 5;
const DEFAULT_BATCH_SIZE = 3;
const DEFAULT_MAX_FAIL_PERCENTAGE = 50;

/** Shown when every selected agent is on another machine, where nothing can
 *  act on it yet. */
const NOTHING_REACHABLE = "Agents on other machines can't be acted on yet: nothing there can verify the request is yours";

export function FleetToolbar({
    model,
    allBlockIds,
}: {
    model: SwarmViewModel;
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
    const reachable = (action: FleetAction) => {
        let n = 0;
        for (const key of selected()) {
            const t = remoteMap().get(key);
            if (!t || !unavailableReason(t, action)) n++;
        }
        return n;
    };
    const plural = (n: number) => `agent${n === 1 ? "" : "s"}`;

    const [broadcastOpen, setBroadcastOpen] = createSignal(false);
    const [broadcastText, setBroadcastText] = createSignal("");
    const [stopConfirmOpen, setStopConfirmOpen] = createSignal(false);
    const [useStaging, setUseStaging] = createSignal(false);
    const [batchSize, setBatchSize] = createSignal(DEFAULT_BATCH_SIZE);
    const [maxFailPercentage, setMaxFailPercentage] = createSignal(DEFAULT_MAX_FAIL_PERCENTAGE);

    const sendBroadcast = async (): Promise<void> => {
        const message = broadcastText().trim();
        if (!message) return;
        setBroadcastOpen(false);
        setBroadcastText("");
        await model.broadcastToSelection(message);
    };

    const confirmStop = async (): Promise<void> => {
        setStopConfirmOpen(false);
        const staged = useStaging()
            ? { batch_size: Math.max(1, batchSize()), max_fail_percentage: Math.min(100, Math.max(0, maxFailPercentage())) }
            : undefined;
        await model.bulkStopSelection({ staged });
    };

    // Shown whenever there's an agent to select, a selection to act on, or
    // stats to show: "Select all" and Stats must be reachable with nothing
    // selected yet.
    const stats = createMemo(() => summarizeAmbientOutcomes(model.ambientOutcomesAtom()));
    const showToolbar = () => allBlockIds().length > 0 || count() > 0 || stats() !== null;

    return (
        <Show when={showToolbar()}>
            <div class="swarm-fleet-toolbar">
                {/* Select all / none is reachable independent of the current
                    count — it's how a selection gets started OR cleared in
                    one click, not an action that requires one first. */}
                <Show when={allBlockIds().length > 0}>
                    <button
                        type="button"
                        class="swarm-fleet-btn"
                        onClick={() => (allSelected() ? model.clearSelection() : model.selectAll(allBlockIds()))}
                    >
                        <i class={allSelected() ? "fa-solid fa-square-check" : "fa-sharp fa-regular fa-square"} />{" "}
                        {allSelected() ? "Select none" : "Select all"}
                    </button>
                </Show>

                {/* Never "act on selected" without stating the concrete count
                    first (spec §3: hidden scope is the top accidental-broadcast
                    cause). Selection-dependent actions below are gated on
                    count() > 0 individually — only Stats (and this bar's own
                    visibility) doesn't require a selection. */}
                <Show when={count() > 0}>
                    <span class="swarm-fleet-toolbar-count">
                        {count()} selected
                        <Show when={otherMachineN() > 0}> · {otherMachineN()} on other machines</Show>
                    </span>

                    <Show
                        when={!broadcastOpen()}
                        fallback={
                            <div class="swarm-fleet-broadcast-inline">
                                <input
                                    type="text"
                                    class="swarm-fleet-broadcast-input"
                                    placeholder={`Message to send to ${reachable("broadcast")} ${plural(reachable("broadcast"))}…`}
                                    value={broadcastText()}
                                    onInput={(e) => setBroadcastText(e.currentTarget.value)}
                                    onKeyDown={(e) => {
                                        if (e.key === "Enter") void sendBroadcast();
                                        if (e.key === "Escape") { setBroadcastOpen(false); setBroadcastText(""); }
                                    }}
                                    autofocus
                                />
                                <button
                                    type="button"
                                    class="swarm-fleet-btn swarm-fleet-btn--primary"
                                    disabled={!broadcastText().trim() || model.fleetActionInFlightAtom()}
                                    onClick={() => void sendBroadcast()}
                                >
                                    Send
                                </button>
                                <button type="button" class="swarm-fleet-btn" onClick={() => { setBroadcastOpen(false); setBroadcastText(""); }}>
                                    Cancel
                                </button>
                            </div>
                        }
                    >
                        <button
                            type="button"
                            class="swarm-fleet-btn"
                            disabled={reachable("broadcast") === 0}
                            title={reachable("broadcast") === 0 ? NOTHING_REACHABLE : undefined}
                            onClick={() => setBroadcastOpen(true)}
                        >
                            <i class="fa-solid fa-tower-broadcast" /> Broadcast
                        </button>
                    </Show>

                    {/* Hidden while the broadcast composer is open — two
                        destructive-adjacent actions competing for the same
                        row invites mis-clicks, and Stop's own confirm modal
                        already covers the "changed my mind" path once this
                        reappears (Cancel closes the composer, not Stop). */}
                    <Show when={!broadcastOpen()}>
                        <button
                            type="button"
                            class="swarm-fleet-btn swarm-fleet-btn--destructive"
                            disabled={model.fleetActionInFlightAtom() || reachable("stop") === 0}
                            title={reachable("stop") === 0 ? NOTHING_REACHABLE : undefined}
                            onClick={() => setStopConfirmOpen(true)}
                        >
                            <i class="fa-solid fa-stop" /> Stop {reachable("stop")}
                        </button>
                    </Show>
                </Show>

                <Show when={stats()}>
                    {(st) => (
                        <button
                            type="button"
                            classList={{ "swarm-fleet-btn": true, "swarm-fleet-btn--active": model.statsOpenAtom() }}
                            aria-expanded={model.statsOpenAtom()}
                            title="Model requests AgentMux makes on its own (session titles, names, prompt suggestions), since its server started"
                            onClick={() => model.toggleStats()}
                        >
                            <i class="fa-solid fa-chart-simple" /> Stats
                            <Show when={st().unhealthyCount > 0}>
                                <span class="swarm-stats-failing">{st().unhealthyCount} failing</span>
                            </Show>
                        </button>
                    )}
                </Show>

                <Show when={count() > 0}>
                    <button type="button" class="swarm-fleet-btn swarm-fleet-toolbar-clear" onClick={() => model.clearSelection()}>
                        Clear
                    </button>
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

export function FleetResultPanel({ model }: { model: SwarmViewModel }): JSX.Element {
    const entry = () => model.lastFleetResultAtom();

    return (
        <Show when={entry()}>
            {(e) => (
                <div class="swarm-fleet-result-panel">
                    <div class="swarm-fleet-result-header">
                        <span>
                            {e().action === "broadcast" ? "Broadcast" : "Bulk stop"} — {e().result.succeeded.length} succeeded,{" "}
                            {e().result.failed.length} failed
                            <Show when={e().result.aborted_early}> — staged rollout aborted early</Show>
                        </span>
                        <button type="button" class="swarm-fleet-result-dismiss" onClick={() => model.dismissFleetResult()}>
                            <i class="fa-solid fa-xmark" />
                        </button>
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
