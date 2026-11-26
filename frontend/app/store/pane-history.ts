// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// This window's trail of places, and Back / Forward through it: select pane B in tab 1, then pane G in tab 4;
// Back selects B again (its tab, its highlight, its caret), Forward selects G again. A place is a pane and,
// for a view that has joined (place-sources.ts), where the view stands inside it: follow a link to a task and
// the Kanban opens on it, and Back to it shows that task again. In memory, one per window.
// SPEC_MOUSE_SIDE_BUTTONS_PANE_AND_TAB_NAV_2026_10_19.md §4 and
// SPEC_BACK_FORWARD_FOLLOW_THE_WHOLE_TRAIL_2026_11_26.md.

import { createEffect, createRoot } from "solid-js";
import { atoms } from "@/app/store/global";
import { setActiveTab } from "@/app/store/tab-actions";
import { getObjectValue, makeORef } from "@/app/store/mos";
import { focusManager } from "@/app/store/focusManager";
import { whenTabContentSettled } from "@/app/tab/tab-content-settled";
import { emitPaneLocate } from "@/app/util/pane-locate";
import { getLayoutModelForTabById, setActiveBlockInStack } from "@/layout/index";
import { findNodeByBlockId } from "@/layout/lib/layoutNode";
import {
    dropEntry,
    emptyHistory,
    locatePane,
    moveTo,
    recordPane,
    replacePlace,
    samePlace,
    sameView,
    setViewAt,
    stepHistory,
    type PaneHistory,
    type PaneRef,
} from "./pane-history-core";
import { placeSourceOf, readPlace, whenPlaceSource } from "./place-sources";
import { installTrailRunners } from "./trail-hooks";

/** How long a step waits for the tab it switches to; it goes ahead after this. */
const TAB_SETTLE_CAP_MS = 1500;
/** How long a step or a gesture waits for a view to register its place source and take a request. */
const VIEW_WAIT_MS = 500;
/** Places recorded within this of each other are one: the last stands (rapid clicks, one gesture). */
const COALESCE_MS = 20;
/** A link's request that reaches a view this long after the gesture that made it still belongs to that step. */
const LATE_REQUEST_MS = 3000;
/** Presses queued while a tab switch is in flight; the rest are dropped. */
const MAX_PENDING = 3;

let history: PaneHistory = emptyHistory();
/** Above 0 while a step is moving the selection, which must not be recorded. */
let navigating = 0;
/** Above 0 while a gesture runs: it records one place when it ends, whatever moved meanwhile. */
let gestures = 0;
/** Above 0 while something that moves the selection for the person runs (an agent's or the host's): not recorded. */
let suppressed = 0;
/** Bumped by each place the user goes to, so steps queued before it are dropped. */
let epoch = 0;
let pending = 0;
let queue: Promise<void> = Promise.resolve();
let lastPush: { at: number; index: number } | null = null;
/** The last gesture's place: where it was recorded, and whether it made an entry of its own. */
let lastGesture: { blockId: string; at: number; index: number; pushed: boolean } | null = null;
let idleWaiters: Array<() => void> = [];

export function paneHistorySnapshot(): PaneHistory {
    return history;
}

export function resetPaneHistory(): void {
    history = emptyHistory();
    navigating = 0;
    gestures = 0;
    suppressed = 0;
    epoch = 0;
    pending = 0;
    queue = Promise.resolve();
    lastPush = null;
    lastGesture = null;
    idleWaiters = [];
}

const blocked = (): boolean => navigating > 0 || gestures > 0 || suppressed > 0;

/** `ref` with where its pane's view stands, when the view has joined. Reads nothing reactive. */
function withView(ref: PaneRef): PaneRef {
    const view = readPlace(ref.blockId);
    return view ? { ...ref, view } : ref;
}

/** Where the person is: the active tab, its focused pane, and that pane's view place. */
function currentPlace(): PaneRef | null {
    const tabId = atoms.activeTabId();
    const blockId = focusManager.blockFocusAtom();
    return tabId && blockId ? withView({ tabId, blockId }) : null;
}

/** The person went to `place`: push it, or let it stand for a push made a moment ago. */
function commit(place: PaneRef): boolean {
    const now = performance.now();
    const again = lastPush != null && history.index === lastPush.index && now - lastPush.at < COALESCE_MS;
    const next = again ? replacePlace(history, place) : recordPane(history, place);
    if (next === history) return false;
    history = next;
    epoch++;
    lastPush = { at: now, index: history.index };
    return true;
}

/** Where `ref`'s pane is now: its tab, the tab it was moved to in this window, or nowhere. */
function locate(ref: PaneRef): PaneRef | null {
    const ws = atoms.workspace();
    const tabIds = [...(ws?.pinnedtabids ?? []), ...(ws?.tabids ?? [])];
    // A tab whose object isn't loaded can't be checked here; selecting it settles the question.
    const tabs = tabIds.map((tabId) => {
        const tab = getObjectValue<Tab>(makeORef("tab", tabId));
        return { tabId, blockIds: tab == null ? null : (tab.blockids ?? []) };
    });
    return locatePane(ref, tabs);
}

function nextFrame(): Promise<void> {
    return new Promise((resolve) => (typeof requestAnimationFrame === "function" ? requestAnimationFrame(() => resolve()) : setTimeout(resolve, 16)));
}

/** Select `ref`'s pane and put the caret in it. False when the pane isn't there. */
async function selectPane(ref: PaneRef): Promise<boolean> {
    const here = atoms.activeTabId() === ref.tabId && focusManager.blockFocusAtom() === ref.blockId;
    if (atoms.activeTabId() !== ref.tabId) {
        await setActiveTab(ref.tabId);
        if (atoms.activeTabId() !== ref.tabId) return false;
        await whenTabContentSettled(ref.tabId, TAB_SETTLE_CAP_MS);
    }
    const model = getLayoutModelForTabById(ref.tabId);
    const root = model?.treeState?.rootNode;
    const node = root && findNodeByBlockId(root, ref.blockId);
    if (!node) return false;
    // The pane may hold the block as a dormant pane tab.
    if (node.data?.blockId !== ref.blockId) setActiveBlockInStack(model, node.id, ref.blockId);
    model.focusNode(node.id);
    // Where the person already is, the view is all that moves: no flash.
    if (!here) emitPaneLocate(ref.blockId);
    // A frame later, once the view is mounted, as the layout's own caret hand-off does.
    await nextFrame();
    focusManager.refocusNode();
    return true;
}

/**
 * Put the pane's view back at the entry's place. "none": the entry is pane-level, or the view has not joined
 * (the step stays pane-level); "ok": the view shows the place; "gone": the view cannot show it (the task was
 * deleted, the file moved).
 */
async function restoreView(ref: PaneRef): Promise<"none" | "ok" | "gone"> {
    if (!ref.view) return "none";
    const source = await whenPlaceSource(ref.blockId, VIEW_WAIT_MS);
    if (!source) return "none";
    if (sameView(readPlace(ref.blockId), ref.view)) return "ok";
    return (await source.restore(ref.view)) ? "ok" : "gone";
}

async function step(dir: -1 | 1, startEpoch: number): Promise<void> {
    await idle();
    // The user went somewhere since this press: it no longer applies.
    if (startEpoch !== epoch) return;
    const started = performance.now();
    const before = currentPlace();
    const tabBefore = atoms.activeTabId();
    let place = "pane";
    navigating++;
    try {
        for (let tries = 0; tries < 10; tries++) {
            // Only the first look asks whether the person is where the trail left them.
            const { history: pruned, target } = stepHistory(history, dir, locate, tries === 0 ? before : null);
            history = pruned;
            if (!target) return;
            if (!(await selectPane(target))) {
                history = dropEntry(history, target);
                continue;
            }
            history = moveTo(history, target);
            const restored = await restoreView(target);
            if (restored === "ok") place = "restored";
            if (restored === "gone") {
                history = setViewAt(history, history.index, undefined);
                // The pane is the same and its place was gone: it shows nothing new, so go on.
                if (samePlace(currentPlace(), before)) continue;
            }
            return;
        }
    } finally {
        // Let the selection effect see the settled pane before recording resumes.
        await nextFrame();
        navigating--;
        lastPush = null;
        const tab = atoms.activeTabId() === tabBefore ? "same" : "switched";
        console.info(`[perf] history step ${dir < 0 ? "back" : "forward"} ${Math.round(performance.now() - started)}ms tab=${tab} place=${place}`);
    }
}

function enqueue(dir: -1 | 1): void {
    if (pending >= MAX_PENDING) return;
    pending++;
    const startEpoch = epoch;
    queue = queue
        .then(() => step(dir, startEpoch))
        .catch((e: unknown) => console.error("[pane-history] step failed:", e))
        .finally(() => {
            pending--;
        });
}

/** Resolves once no gesture is running: a step does not look at a trail a gesture is about to add to. */
function idle(): Promise<void> {
    return gestures === 0 ? Promise.resolve() : new Promise<void>((resolve) => idleWaiters.push(resolve));
}

/** Go to the place before this one, switching tab if it is in another and putting the view back. */
export function paneHistoryBack(): void {
    enqueue(-1);
}

/** Go to the place after this one, after going Back. */
export function paneHistoryForward(): void {
    enqueue(1);
}

/**
 * Run `run`, which moves the person (follows a link, opens a pane), as one step of the trail: the selection
 * effect is held while it runs, and when it is done and the view has taken the request (up to half a second,
 * for a view that has joined) the place the person is at is recorded once, pane and view together. Only a
 * person's gesture is run through here; an agent's open takes no focus and records nothing.
 */
export async function runGesture<T>(run: () => Promise<T> | T): Promise<T> {
    gestures++;
    try {
        return await run();
    } finally {
        try {
            await settleGesture();
        } finally {
            gestures--;
            if (gestures === 0) {
                const here = blocked() ? null : currentPlace();
                if (here) {
                    const pushed = commit(here);
                    lastGesture = { blockId: here.blockId, at: performance.now(), index: history.index, pushed };
                }
                const waiting = idleWaiters;
                idleWaiters = [];
                for (const resolve of waiting) resolve();
            }
        }
    }
}

/** A frame for the layout to move focus, and for a view that joins, until it has registered and taken the ask. */
async function settleGesture(): Promise<void> {
    await nextFrame();
    const blockId = focusManager.blockFocusAtom();
    if (!blockId) return;
    const source = await whenPlaceSource(blockId, VIEW_WAIT_MS);
    const until = performance.now() + VIEW_WAIT_MS;
    while (source?.settled && !source.settled() && performance.now() < until) await nextFrame();
}

/**
 * Run `run`, which moves the person's selection on its own (an agent's reveal, the host): the trail does not
 * record it. Back then returns to where the trail left the person, before going further back.
 */
export async function withoutHistory<T>(run: () => Promise<T> | T): Promise<T> {
    suppressed++;
    try {
        return await run();
    } finally {
        // Let the selection effect see the settled pane before recording resumes.
        await nextFrame();
        suppressed--;
    }
}

/**
 * The person moved inside the view of pane `blockId` (opened a card, picked a board): a step, from the view's
 * own click handler, never from an effect that follows data. Inside a gesture, a step or a suppressed move the
 * gesture or step has it covered.
 */
export function recordPlaceMove(blockId: string): void {
    if (blocked() || !placeSourceOf(blockId)) return;
    const tabId = atoms.activeTabId();
    if (tabId) commit(withView({ tabId, blockId }));
}

/**
 * A link's request reached the view of pane `blockId` after the gesture that made it had already recorded its
 * place (the request travels through the pane's meta): the place the view shows now is that gesture's step,
 * not another one. Nothing else calls it: a request with no gesture of the person just before it (an agent's)
 * is not a step.
 */
export function recordLinkedPlace(blockId: string): void {
    const g = lastGesture;
    if (blocked() || !g || g.blockId !== blockId || performance.now() - g.at > LATE_REQUEST_MS) return;
    const here = currentPlace();
    if (!here || here.blockId !== blockId || history.index !== g.index) return;
    lastGesture = null;
    if (g.pushed) history = setViewAt(history, g.index, here.view);
    else commit(here);
}

/**
 * Record every pane the window selects, from the one place that knows: the
 * active tab and its focused pane. Called once at window start.
 */
export function installPaneHistory(): () => void {
    const restoreRunners = installTrailRunners(runGesture, withoutHistory);
    return createRoot((disposeRoot) => {
        const dispose = () => {
            restoreRunners();
            disposeRoot();
        };
        createEffect(() => {
            const tabId = atoms.activeTabId();
            const blockId = focusManager.blockFocusAtom();
            if (!tabId || !blockId || blocked()) return;
            commit(withView({ tabId, blockId }));
        });
        return dispose;
    });
}
