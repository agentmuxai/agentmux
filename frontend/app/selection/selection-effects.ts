// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Runs the selection machine's effects and feeds what they find back to it as events. Every
 * effect names its intent and every event that comes back is checked against the current one by
 * `transition`, so what an older request finds after a newer one started is dropped.
 * The things an effect touches (tabs, layout, focus, another window) are `SelectionDeps`, so the tests
 * run the machine against a fake with a controllable completion order.
 */

import {
    IDLE,
    liveIntent,
    transition,
    type Intent,
    type SelectionEffect,
    type SelectionEvent,
    type SelectionResult,
    type SelectionState,
    isFailure,
} from "./selection-machine";
import type { SelectionTrace } from "./selection-trace";

export type ResolveOutcome = { where: "here"; tabId: string; active: boolean } | { where: "other" } | { where: "none" };
export type VerifyOutcome = { ok: true } | { ok: false; detail: string } | { gone: true };

const verdict = (o: VerifyOutcome): "gone" | "ok" | { detail: string } =>
    "gone" in o ? "gone" : "detail" in o ? { detail: o.detail } : "ok";

export interface SelectionDeps {
    /** Which window tab of this window holds the block, if one does. */
    resolve(intent: Intent): Promise<ResolveOutcome>;
    switchTab(tabId: string): Promise<void>;
    /** Whether the tab's layout contains the block yet. */
    layoutHas(tabId: string, blockId: string): boolean;
    /** Restore it if minimized, un-magnify another pane, make it its pane's active tab. */
    show(tabId: string, blockId: string): void;
    /** Focus its pane and, with `caret`, put the caret in it. */
    focus(tabId: string, blockId: string, caret: boolean): void;
    /** Resolves when what a tab switch schedules for later (a refocus) has had its turn. */
    settle(): Promise<void>;
    /** Read the state back and compare it with the intent. */
    verify(tabId: string, blockId: string, caret: boolean): VerifyOutcome;
    /** Ask another window to show it; whether one was found. */
    handoff(blockId: string): Promise<boolean>;
    /** The locate flash. */
    flash(blockId: string): void;
    /** The quiet notice for a pane that could not be shown. */
    notify(intent: Intent, reason: string, detail?: string): void;
    setTimer(fn: () => void, ms: number): unknown;
    clearTimer(handle: unknown): void;
    now(): number;
}

/**
 * The window tabs this runner asked to switch to, for a while after. The active tab is committed by
 * the workspace push, which can land after the switch's own reply, and after a newer request has
 * begun; such a change is ours (a request that was replaced, say) and not the user switching tab.
 */
export function createRequestedTabs(now: () => number = Date.now, keepMs = 5000) {
    const until = new Map<string, number>();
    return {
        asked(tabId: string): void {
            until.set(tabId, now() + keepMs);
        },
        /** Whether a change of the active tab to `tabId` is one this runner caused. */
        isOurs(tabId: string): boolean {
            return (until.get(tabId) ?? 0) > now();
        },
        /** Whether a switch to some tab other than `tabId` may still be on its way, so `tabId` being
         *  the committed active tab does not mean it will stay that. */
        pendingElsewhere(tabId: string): boolean {
            for (const [tab, expires] of until) if (tab !== tabId && expires > now()) return true;
            return false;
        },
    };
}

export interface SelectionRequest {
    blockId: string;
    source?: string;
    tabHint?: string;
    /** Default true. */
    focusCaret?: boolean;
}

export interface SelectionRunner {
    /** Show the pane. Resolves when this request ends: shown and verified, failed, or superseded. */
    select(req: SelectionRequest): Promise<SelectionResult>;
    /** The user selected something else: end what is in flight, without a notice. */
    userMoved(): void;
    state(): SelectionState;
}

/** How often to look for a just-activated tab's layout; the machine's deadline ends the wait. */
export const LAYOUT_POLL_MS = 50;

export function createSelectionRunner(deps: SelectionDeps, trace: SelectionTrace): SelectionRunner {
    let state: SelectionState = IDLE;
    let nextId = 0;
    let deadlineTimer: unknown = null;
    let pollTimer: unknown = null;
    const startedAt = new Map<number, number>();
    const waiting = new Map<number, (r: SelectionResult) => void>();

    const clearTimers = () => {
        if (deadlineTimer != null) deps.clearTimer(deadlineTimer);
        if (pollTimer != null) deps.clearTimer(pollTimer);
        deadlineTimer = pollTimer = null;
    };

    // One event at a time: an effect that finishes at once (show, focus) queues its event instead of
    // running the next transition inside its own, so the effects of one transition all run before
    // the next begins, and a deadline is armed after the effects that precede it.
    const queue: SelectionEvent[] = [];
    let draining = false;
    const dispatch = (event: SelectionEvent): void => {
        queue.push(event);
        if (draining) return;
        draining = true;
        try {
            for (let e = queue.shift(); e; e = queue.shift()) step(e);
        } finally {
            draining = false;
        }
    };

    const step = (event: SelectionEvent): void => {
        const before = state;
        const next = transition(state, event);
        state = next.state;
        if (state !== before) clearTimers();
        const intent = event.type === "select" ? event.intent : liveIntent(before);
        if (intent && state.kind !== before.kind) {
            const end = next.effects.find((e) => e.type === "finish" && e.intentId === intent.id);
            const result = end?.type === "finish" && isFailure(end.result) ? end.result : undefined;
            trace.transition({
                intentId: intent.id,
                source: intent.source,
                blockId: intent.blockId,
                from: event.type === "select" ? "idle" : before.kind,
                to: state.kind,
                sinceStartMs: deps.now() - (startedAt.get(intent.id) ?? deps.now()),
                reason: result?.reason,
                detail: result?.detail,
            });
        }
        for (const effect of next.effects) {
            // An intent that was replaced or abandoned: say so; its waiting caller is told below.
            if (effect.type === "finish" && isFailure(effect.result) && effect.result.reason === "superseded") {
                trace.note(`[select] id=${effect.intentId} superseded`);
            }
            run(effect);
        }
    };

    const guard = (intentId: number, work: () => void) => {
        try {
            work();
        } catch (e) {
            dispatch({ type: "effectFailed", intentId, detail: e instanceof Error ? e.message : String(e) });
        }
    };

    const run = (effect: SelectionEffect): void => {
        switch (effect.type) {
            case "resolve": {
                const id = effect.intent.id;
                deps.resolve(effect.intent).then(
                    (o) =>
                        dispatch(
                            o.where === "here"
                                ? { type: "resolved", intentId: id, where: "here", tabId: o.tabId, active: o.active }
                                : { type: "resolved", intentId: id, where: o.where }
                        ),
                    (e) => dispatch({ type: "effectFailed", intentId: id, detail: String(e) })
                );
                return;
            }
            case "switchTab":
                deps.switchTab(effect.tabId).then(
                    () => dispatch({ type: "tabSwitched", intentId: effect.intentId }),
                    (e) => dispatch({ type: "effectFailed", intentId: effect.intentId, detail: String(e) })
                );
                return;
            case "awaitLayout": {
                const look = () => {
                    pollTimer = null;
                    if (liveIntent(state)?.id !== effect.intentId || state.kind !== "awaitingLayout") return;
                    if (deps.layoutHas(effect.tabId, effect.blockId)) {
                        dispatch({ type: "layoutReady", intentId: effect.intentId });
                        return;
                    }
                    pollTimer = deps.setTimer(look, LAYOUT_POLL_MS);
                };
                look();
                return;
            }
            case "show":
                guard(effect.intentId, () => {
                    deps.show(effect.tabId, effect.blockId);
                    dispatch({ type: "shown", intentId: effect.intentId });
                });
                return;
            case "focus":
                guard(effect.intentId, () => {
                    deps.focus(effect.tabId, effect.blockId, effect.caret);
                    dispatch({ type: "focused", intentId: effect.intentId });
                });
                return;
            case "verify":
                deps.settle().then(
                    () =>
                        guard(effect.intentId, () => {
                            const v = verdict(deps.verify(effect.tabId, effect.blockId, effect.caret));
                            if (v === "gone") dispatch({ type: "blockGone", intentId: effect.intentId });
                            else if (v === "ok") dispatch({ type: "verified", intentId: effect.intentId, ok: true });
                            else dispatch({ type: "verified", intentId: effect.intentId, ok: false, detail: v.detail });
                        }),
                    (e) => dispatch({ type: "effectFailed", intentId: effect.intentId, detail: String(e) })
                );
                return;
            case "handoff":
                deps.handoff(effect.blockId).then(
                    (found) => dispatch({ type: "remoteDone", intentId: effect.intentId, found }),
                    (e) => dispatch({ type: "effectFailed", intentId: effect.intentId, detail: String(e) })
                );
                return;
            case "arm":
                deadlineTimer = deps.setTimer(() => {
                    deadlineTimer = null;
                    dispatch({ type: "deadline", intentId: effect.intentId, step: effect.step });
                }, effect.ms);
                return;
            case "flash":
                deps.flash(effect.blockId);
                return;
            case "notify":
                deps.notify(effect.intent, effect.reason, effect.detail);
                return;
            case "finish": {
                const done = waiting.get(effect.intentId);
                waiting.delete(effect.intentId);
                startedAt.delete(effect.intentId);
                done?.(effect.result);
                return;
            }
        }
    };

    return {
        select(req) {
            const intent: Intent = {
                id: ++nextId,
                blockId: req.blockId,
                source: req.source ?? "unknown",
                tabHint: req.tabHint,
                focusCaret: req.focusCaret !== false,
            };
            startedAt.set(intent.id, deps.now());
            const result = new Promise<SelectionResult>((resolve) => waiting.set(intent.id, resolve));
            dispatch({ type: "select", intent });
            return result;
        },
        userMoved() {
            dispatch({ type: "userMoved" });
        },
        state: () => state,
    };
}
