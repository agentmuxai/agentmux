// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Fleet control toolbar + confirm modal + results panel for the Swarm pane.
// See docs/specs/SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md.

import { autoUpdate } from "@floating-ui/dom";
import { createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { Portal } from "solid-js/web";
import { ConfirmModal } from "@/app/element/confirm-modal";
import { usePaneOverlay } from "@/app/platform/pane-overlay";
import { assertMenuInPaintableArea, computeMenuPosition } from "@/app/util/menu-position";
import type { SwarmViewModel } from "./swarm-model";

// Staged rollout only offered once a selection is large enough that
// blast-radius capping is actually meaningful (spec §5.3) — for a
// handful of targets the whole point of staging (canary-first,
// abort-on-bad-batch) doesn't apply.
const STAGING_ELIGIBLE_AT = 5;
const DEFAULT_BATCH_SIZE = 3;
const DEFAULT_MAX_FAIL_PERCENTAGE = 50;

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

    const [broadcastOpen, setBroadcastOpen] = createSignal(false);
    const [broadcastText, setBroadcastText] = createSignal("");
    const [stopConfirmOpen, setStopConfirmOpen] = createSignal(false);
    const [useStaging, setUseStaging] = createSignal(false);
    const [batchSize, setBatchSize] = createSignal(DEFAULT_BATCH_SIZE);
    const [maxFailPercentage, setMaxFailPercentage] = createSignal(DEFAULT_MAX_FAIL_PERCENTAGE);
    const [groupPickerOpen, setGroupPickerOpen] = createSignal(false);
    const [savingGroupName, setSavingGroupName] = createSignal<string | null>(null);

    // The groups dropdown is portaled + floating-ui-positioned rather than a
    // plain `position: absolute` child of the toolbar (reagent/Codex on this
    // PR's review: a 200px-wide flyout anchored to a button inside a narrow,
    // `overflow: hidden` Swarm pane can render past the visible edge and get
    // invisibly clipped — no fixed CSS anchor side fixes this in general,
    // since the trigger button's position varies with toolbar wrapping).
    // Mirrors FlyoutMenu/PopoverMenu's established pattern (see
    // frontend/app/element/flyoutmenu.tsx, scripts/check-menu-positioning.sh).
    let groupPickerButtonRef: HTMLButtonElement | undefined;
    const [groupsMenuEl, setGroupsMenuEl] = createSignal<HTMLDivElement | null>(null);
    const [groupsMenuStyle, setGroupsMenuStyle] = createSignal<JSX.CSSProperties>({
        position: "fixed",
        left: "0px",
        top: "0px",
        visibility: "hidden",
    });
    usePaneOverlay(groupsMenuEl);
    let cleanupGroupsMenuAutoUpdate: (() => void) | null = null;

    const updateGroupsMenuPosition = async (): Promise<void> => {
        const btn = groupPickerButtonRef;
        const menu = groupsMenuEl();
        if (!btn || !menu) return;
        const pos = await computeMenuPosition({ anchor: btn, placement: "bottom-start" }, menu);
        setGroupsMenuStyle({
            ...pos.style,
            "max-height": `${pos.maxHeight}px`,
            "max-width": `${pos.maxWidth}px`,
            "overflow-y": "auto",
        });
    };

    const registerGroupsMenu = (el: HTMLDivElement): void => {
        setGroupsMenuEl(el);
        requestAnimationFrame(() => {
            if (!groupPickerButtonRef || !(el instanceof Element)) return;
            cleanupGroupsMenuAutoUpdate?.();
            cleanupGroupsMenuAutoUpdate = autoUpdate(groupPickerButtonRef, el, updateGroupsMenuPosition);
            assertMenuInPaintableArea(el, "swarm-fleet-group-dropdown");
        });
    };

    const closeGroupPicker = (): void => {
        setGroupPickerOpen(false);
        cleanupGroupsMenuAutoUpdate?.();
        cleanupGroupsMenuAutoUpdate = null;
    };

    const handleGroupPickerOutsideClick = (e: MouseEvent): void => {
        if (!groupPickerOpen()) return;
        const t = e.target as Node;
        if (groupPickerButtonRef?.contains(t) || groupsMenuEl()?.contains(t)) return;
        closeGroupPicker();
    };
    const handleGroupPickerEscape = (e: KeyboardEvent): void => {
        if (e.key === "Escape" && groupPickerOpen()) closeGroupPicker();
    };

    onMount(() => {
        document.addEventListener("mousedown", handleGroupPickerOutsideClick, true);
        document.addEventListener("keydown", handleGroupPickerEscape);
    });
    onCleanup(() => {
        document.removeEventListener("mousedown", handleGroupPickerOutsideClick, true);
        document.removeEventListener("keydown", handleGroupPickerEscape);
        cleanupGroupsMenuAutoUpdate?.();
    });

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

    // Shown whenever there's at least one agent to select, a live selection
    // to act on, OR a saved group to offer — a saved group must stay
    // reachable as a one-click way to RESTORE a selection even when nothing
    // is currently checked (Codex P2, PR #2687 review: the whole toolbar,
    // including the group picker, used to be gated on count() > 0, so a
    // group could never be the FIRST thing a user reached for). "Select all"
    // needs the same always-reachable treatment — it's the other way to
    // start a selection from zero.
    const showToolbar = () => allBlockIds().length > 0 || count() > 0 || model.fleetGroupsAtom().length > 0;

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
                    count() > 0 individually — only the group picker (and this
                    bar's own visibility) doesn't require a selection. */}
                <Show when={count() > 0}>
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

                    {/* Hidden while the broadcast composer is open — two
                        destructive-adjacent actions competing for the same
                        row invites mis-clicks, and Stop's own confirm modal
                        already covers the "changed my mind" path once this
                        reappears (Cancel closes the composer, not Stop). */}
                    <Show when={!broadcastOpen()}>
                        <button
                            type="button"
                            class="swarm-fleet-btn swarm-fleet-btn--destructive"
                            disabled={model.fleetActionInFlightAtom()}
                            onClick={() => setStopConfirmOpen(true)}
                        >
                            <i class="fa-solid fa-stop" /> Stop {count()}
                        </button>
                    </Show>
                </Show>

                <div class="swarm-fleet-group-picker">
                    <button
                        type="button"
                        class="swarm-fleet-btn"
                        ref={(el) => (groupPickerButtonRef = el)}
                        onClick={() => (groupPickerOpen() ? closeGroupPicker() : setGroupPickerOpen(true))}
                    >
                        Groups <i class="fa-solid fa-chevron-down" />
                    </button>
                    <Show when={groupPickerOpen()}>
                        {/* Portaled + floating-ui-positioned (see the state block
                            above) instead of a plain absolutely-positioned child —
                            escapes `.swarm-view`'s `overflow: hidden` so a narrow,
                            wrapped toolbar can never invisibly clip this dropdown
                            off the visible pane, regardless of where the trigger
                            button lands. */}
                        <Portal mount={document.body}>
                            <div
                                ref={registerGroupsMenu}
                                class="swarm-fleet-group-dropdown"
                                data-pane-overlay
                                style={groupsMenuStyle()}
                            >
                                {/* Saving only makes sense against a non-empty
                                    selection — applying/deleting an EXISTING
                                    group below never requires one. */}
                                <Show when={count() > 0}>
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
                                </Show>
                                <Show when={model.fleetGroupsAtom().length > 0} fallback={<div class="swarm-fleet-group-dropdown-empty">No saved groups yet</div>}>
                                    <For each={model.fleetGroupsAtom()}>
                                        {(group) => (
                                            <div class="swarm-fleet-group-dropdown-item">
                                                <button
                                                    type="button"
                                                    class="swarm-fleet-group-dropdown-item-apply"
                                                    onClick={() => { model.applyGroupAsSelection(group); closeGroupPicker(); }}
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
                        </Portal>
                    </Show>
                </div>

                <Show when={count() > 0}>
                    <button type="button" class="swarm-fleet-btn swarm-fleet-toolbar-clear" onClick={() => model.clearSelection()}>
                        Clear
                    </button>
                </Show>
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
