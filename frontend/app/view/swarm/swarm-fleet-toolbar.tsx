// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Fleet control toolbar + confirm modal + results panel for the Swarm pane.
// See docs/specs/SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md.

import { createSignal, For, Show, type JSX } from "solid-js";
import { ConfirmModal } from "@/app/element/confirm-modal";
import type { SwarmViewModel } from "./swarm-model";

// Staged rollout only offered once a selection is large enough that
// blast-radius capping is actually meaningful (spec §5.3) — for a
// handful of targets the whole point of staging (canary-first,
// abort-on-bad-batch) doesn't apply.
const STAGING_ELIGIBLE_AT = 5;
const DEFAULT_BATCH_SIZE = 3;
const DEFAULT_MAX_FAIL_PERCENTAGE = 50;

export function FleetToolbar({ model }: { model: SwarmViewModel }): JSX.Element {
    const selected = () => model.selectedBlockIdsAtom();
    const count = () => selected().size;

    const [broadcastOpen, setBroadcastOpen] = createSignal(false);
    const [broadcastText, setBroadcastText] = createSignal("");
    const [stopConfirmOpen, setStopConfirmOpen] = createSignal(false);
    const [useStaging, setUseStaging] = createSignal(false);
    const [batchSize, setBatchSize] = createSignal(DEFAULT_BATCH_SIZE);
    const [maxFailPercentage, setMaxFailPercentage] = createSignal(DEFAULT_MAX_FAIL_PERCENTAGE);
    const [groupPickerOpen, setGroupPickerOpen] = createSignal(false);
    const [savingGroupName, setSavingGroupName] = createSignal<string | null>(null);

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

    const submitSaveGroup = async (): Promise<void> => {
        const name = (savingGroupName() ?? "").trim();
        if (!name) return;
        setSavingGroupName(null);
        await model.saveSelectionAsGroup(name);
    };

    return (
        <Show when={count() > 0}>
            <div class="swarm-fleet-toolbar">
                {/* Never "act on selected" without stating the concrete count
                    first (spec §3: hidden scope is the top accidental-broadcast
                    cause). */}
                <span class="swarm-fleet-toolbar-count">{count()} selected</span>

                <Show
                    when={!broadcastOpen()}
                    fallback={
                        <div class="swarm-fleet-broadcast-inline">
                            <input
                                type="text"
                                class="swarm-fleet-broadcast-input"
                                placeholder={`Message to send to ${count()} agents…`}
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
                    <button type="button" class="swarm-fleet-btn" onClick={() => setBroadcastOpen(true)}>
                        <i class="fa-solid fa-tower-broadcast" /> Broadcast
                    </button>
                </Show>

                <button
                    type="button"
                    class="swarm-fleet-btn swarm-fleet-btn--destructive"
                    disabled={model.fleetActionInFlightAtom()}
                    onClick={() => setStopConfirmOpen(true)}
                >
                    <i class="fa-solid fa-stop" /> Stop {count()}
                </button>

                <div class="swarm-fleet-group-picker">
                    <button type="button" class="swarm-fleet-btn" onClick={() => setGroupPickerOpen((v) => !v)}>
                        Groups <i class="fa-solid fa-chevron-down" />
                    </button>
                    <Show when={groupPickerOpen()}>
                        <div class="swarm-fleet-group-dropdown">
                            <Show
                                when={savingGroupName() !== null}
                                fallback={
                                    <button
                                        type="button"
                                        class="swarm-fleet-group-dropdown-item swarm-fleet-group-dropdown-item--action"
                                        onClick={() => setSavingGroupName("")}
                                    >
                                        Save selection as group…
                                    </button>
                                }
                            >
                                <div class="swarm-fleet-group-save-inline">
                                    <input
                                        type="text"
                                        placeholder="Group name"
                                        value={savingGroupName() ?? ""}
                                        onInput={(e) => setSavingGroupName(e.currentTarget.value)}
                                        onKeyDown={(e) => {
                                            if (e.key === "Enter") void submitSaveGroup();
                                            if (e.key === "Escape") setSavingGroupName(null);
                                        }}
                                        autofocus
                                    />
                                    <button type="button" class="swarm-fleet-btn swarm-fleet-btn--primary" onClick={() => void submitSaveGroup()}>
                                        Save
                                    </button>
                                </div>
                            </Show>
                            <Show when={model.fleetGroupsAtom().length > 0} fallback={<div class="swarm-fleet-group-dropdown-empty">No saved groups yet</div>}>
                                <For each={model.fleetGroupsAtom()}>
                                    {(group) => (
                                        <div class="swarm-fleet-group-dropdown-item">
                                            <button
                                                type="button"
                                                class="swarm-fleet-group-dropdown-item-apply"
                                                onClick={() => { model.applyGroupAsSelection(group); setGroupPickerOpen(false); }}
                                                title={`Select this group's ${group.member_ids.length} agent(s)`}
                                            >
                                                {group.name} <span class="swarm-fleet-group-dropdown-item-count">({group.member_ids.length})</span>
                                            </button>
                                            <button
                                                type="button"
                                                class="swarm-fleet-group-dropdown-item-delete"
                                                title="Delete group"
                                                onClick={() => void model.deleteFleetGroup(group.id)}
                                            >
                                                <i class="fa-solid fa-xmark" />
                                            </button>
                                        </div>
                                    )}
                                </For>
                            </Show>
                        </div>
                    </Show>
                </div>

                <button type="button" class="swarm-fleet-btn swarm-fleet-toolbar-clear" onClick={() => model.clearSelection()}>
                    Clear
                </button>
            </div>

            <ConfirmModal
                open={stopConfirmOpen()}
                title={`Stop ${count()} agent${count() === 1 ? "" : "s"}?`}
                description="This stops the selected agent panes. Each pane's own stop outcome is reported individually — a partial failure never shows as a single pass/fail."
                destructive
                confirmLabel={`Stop ${count()}`}
                onConfirm={confirmStop}
                onCancel={() => setStopConfirmOpen(false)}
            >
                <div class="swarm-fleet-confirm-target-list">
                    <For each={Array.from(selected())}>{(blockId) => <div class="swarm-fleet-confirm-target-row">{blockId}</div>}</For>
                </div>
                <Show when={count() >= STAGING_ELIGIBLE_AT}>
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
                                    <i class="fa-solid fa-check" /> {id}
                                </div>
                            )}
                        </For>
                        <For each={e().result.failed}>
                            {(f) => (
                                <div class="swarm-fleet-result-row swarm-fleet-result-row--fail">
                                    <i class="fa-solid fa-triangle-exclamation" /> {f.id} — {f.error}
                                </div>
                            )}
                        </For>
                    </div>
                </div>
            )}
        </Show>
    );
}
