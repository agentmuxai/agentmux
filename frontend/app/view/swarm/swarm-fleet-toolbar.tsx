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
    // Set once, synchronously, by the button's own `ref` callback during
    // FleetToolbar's initial render — already valid by the time a user
    // could possibly click it open, so plain top-level state (not a signal)
    // is enough here.
    let groupPickerButtonRef: HTMLButtonElement | undefined;

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
                        onClick={() => setGroupPickerOpen((v) => !v)}
                    >
                        Groups <i class="fa-solid fa-chevron-down" />
                    </button>
                    {/* A fresh GroupsDropdown instance mounts each time this
                        flips true (Solid's <Show> unmounts/remounts its child
                        on a boolean transition) — required so its own
                        usePaneOverlay/onMount re-registers on every open,
                        matching FlyoutMenu/PopoverMenu's pattern. Calling
                        usePaneOverlay directly in FleetToolbar's body instead
                        would only ever mount once for the whole Swarm pane's
                        lifetime, while groupsMenuEl was still null — reagent
                        P1 on this PR's previous revision. */}
                    <Show when={groupPickerOpen()}>
                        <GroupsDropdown
                            triggerEl={groupPickerButtonRef!}
                            model={model}
                            hasSelection={count() > 0}
                            savingGroupName={savingGroupName}
                            setSavingGroupName={setSavingGroupName}
                            submitSaveGroup={submitSaveGroup}
                            onClose={() => setGroupPickerOpen(false)}
                        />
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

/**
 * Saved-groups flyout for the fleet toolbar — a separate component (not
 * inline JSX in `FleetToolbar`) specifically so it mounts fresh every time
 * the picker opens. `usePaneOverlay`'s registration happens in `onMount`,
 * which in Solid fires once per component instance — a `<Show>` toggling a
 * boolean unmounts/remounts its child on each transition, so a genuinely
 * separate component here gets a fresh `onMount` (and the working pane-clip
 * registration that depends on it) every open. Calling `usePaneOverlay`
 * directly in `FleetToolbar`'s own body would only ever run once, for the
 * whole Swarm pane's lifetime — see the call site's comment.
 *
 * Portaled to `document.body` and positioned via `computeMenuPosition`
 * (floating-ui, kept live via `autoUpdate`), the same primitive
 * `flyoutmenu.tsx`/`popover-menu.tsx` use — escapes `.swarm-view`'s
 * `overflow: hidden` entirely, so it can't be clipped regardless of where
 * the trigger button lands after the toolbar wraps.
 */
function GroupsDropdown(props: {
    triggerEl: HTMLButtonElement;
    model: SwarmViewModel;
    hasSelection: boolean;
    savingGroupName: () => string | null;
    setSavingGroupName: (v: string | null) => void;
    submitSaveGroup: () => Promise<void>;
    onClose: () => void;
}): JSX.Element {
    const [menuEl, setMenuEl] = createSignal<HTMLDivElement | null>(null);
    const [menuStyle, setMenuStyle] = createSignal<JSX.CSSProperties>({
        position: "fixed",
        left: "0px",
        top: "0px",
        visibility: "hidden",
    });
    usePaneOverlay(menuEl);
    let cleanupAutoUpdate: (() => void) | null = null;

    const updatePosition = async (): Promise<void> => {
        const menu = menuEl();
        if (!menu) return;
        const pos = await computeMenuPosition({ anchor: props.triggerEl, placement: "bottom-start" }, menu);
        setMenuStyle({
            ...pos.style,
            "max-height": `${pos.maxHeight}px`,
            "max-width": `${pos.maxWidth}px`,
            "overflow-y": "auto",
        });
    };

    const registerMenu = (el: HTMLDivElement): void => {
        setMenuEl(el);
        requestAnimationFrame(() => {
            if (!(el instanceof Element)) return;
            cleanupAutoUpdate = autoUpdate(props.triggerEl, el, updatePosition);
            assertMenuInPaintableArea(el, "swarm-fleet-group-dropdown");
        });
    };

    const handleOutsideClick = (e: MouseEvent): void => {
        const t = e.target as Node;
        if (props.triggerEl.contains(t) || menuEl()?.contains(t)) return;
        props.onClose();
    };
    const handleEscape = (e: KeyboardEvent): void => {
        if (e.key === "Escape") props.onClose();
    };

    onMount(() => {
        document.addEventListener("mousedown", handleOutsideClick, true);
        document.addEventListener("keydown", handleEscape);
    });
    onCleanup(() => {
        document.removeEventListener("mousedown", handleOutsideClick, true);
        document.removeEventListener("keydown", handleEscape);
        cleanupAutoUpdate?.();
    });

    return (
        <Portal mount={document.body}>
            <div ref={registerMenu} class="swarm-fleet-group-dropdown" data-pane-overlay style={menuStyle()}>
                {/* Saving only makes sense against a non-empty selection —
                    applying/deleting an EXISTING group below never requires one. */}
                <Show when={props.hasSelection}>
                    <Show
                        when={props.savingGroupName() !== null}
                        fallback={
                            <button
                                type="button"
                                class="swarm-fleet-group-dropdown-item swarm-fleet-group-dropdown-item--action"
                                onClick={() => props.setSavingGroupName("")}
                            >
                                Save selection as group…
                            </button>
                        }
                    >
                        <div class="swarm-fleet-group-save-inline">
                            <input
                                type="text"
                                placeholder="Group name"
                                value={props.savingGroupName() ?? ""}
                                onInput={(e) => props.setSavingGroupName(e.currentTarget.value)}
                                onKeyDown={(e) => {
                                    if (e.key === "Enter") void props.submitSaveGroup();
                                    if (e.key === "Escape") props.setSavingGroupName(null);
                                }}
                                autofocus
                            />
                            <button type="button" class="swarm-fleet-btn swarm-fleet-btn--primary" onClick={() => void props.submitSaveGroup()}>
                                Save
                            </button>
                        </div>
                    </Show>
                </Show>
                <Show when={props.model.fleetGroupsAtom().length > 0} fallback={<div class="swarm-fleet-group-dropdown-empty">No saved groups yet</div>}>
                    <For each={props.model.fleetGroupsAtom()}>
                        {(group) => (
                            <div class="swarm-fleet-group-dropdown-item">
                                <button
                                    type="button"
                                    class="swarm-fleet-group-dropdown-item-apply"
                                    onClick={() => { props.model.applyGroupAsSelection(group); props.onClose(); }}
                                    title={`Select this group's ${group.member_ids.length} agent(s)`}
                                >
                                    {group.name} <span class="swarm-fleet-group-dropdown-item-count">({group.member_ids.length})</span>
                                </button>
                                <button
                                    type="button"
                                    class="swarm-fleet-group-dropdown-item-delete"
                                    title="Delete group"
                                    onClick={() => void props.model.deleteFleetGroup(group.id)}
                                >
                                    <i class="fa-solid fa-xmark" />
                                </button>
                            </div>
                        )}
                    </For>
                </Show>
            </div>
        </Portal>
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
