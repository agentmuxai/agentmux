// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Show this pane": one intent at a time, from the click to a verified result.
 * `transition` is pure: it takes a state and an event and returns the next state and the effects to
 * run, so every interleaving (a newer click while an older one is in flight, a layout that never
 * arrives, a tab that closes) is a table row in the tests. `selection-effects.ts` runs the effects.
 *
 * Spec: docs/specs/SPEC_PANE_SELECTION_STATE_MACHINE_AND_SWARM_CLICK_2026_10_18.md §6.2.
 */

export interface Intent {
    /** Increases with each request; an event or an effect that names an older one is dropped. */
    id: number;
    blockId: string;
    /** Who asked: "swarm", "tower", "tokens", "picker", ... */
    source: string;
    /** The window tab the caller expects to hold the block. */
    tabHint?: string;
    /** Put the caret in the block (its composer or terminal). */
    focusCaret: boolean;
}

/** Why a pane could not be shown, in the words of the notice (selection-trace and notice text map them). */
export type FailReason =
    | "not-found"
    | "resolve-timeout"
    | "tab-timeout"
    | "layout-timeout"
    | "stuck"
    | "mismatch"
    | "remote-timeout"
    | "gone"
    | "error";

export type SelectionFailure = { ok: false; reason: FailReason | "superseded"; detail?: string };
export type SelectionResult = { ok: true } | SelectionFailure;

/** Whether a result is a failure (the repo does not run strictNullChecks, so `ok` alone does not narrow). */
export const isFailure = (r: SelectionResult): r is SelectionFailure => r.ok === false;

interface Live {
    intent: Intent;
    /** Counts every transition, so a deadline armed for one state cannot end a later one. */
    step: number;
}

export type SelectionState =
    | { kind: "idle"; step: number }
    | ({ kind: "resolving" } & Live)
    | ({ kind: "switchingTab"; tabId: string } & Live)
    | ({ kind: "awaitingLayout"; tabId: string } & Live)
    | ({ kind: "showing"; tabId: string; retried: boolean } & Live)
    | ({ kind: "focusing"; tabId: string; retried: boolean } & Live)
    | ({ kind: "verifying"; tabId: string; retried: boolean } & Live)
    | ({ kind: "handoff" } & Live)
    | ({ kind: "settled" } & Live)
    | ({ kind: "failed"; reason: FailReason; detail?: string } & Live);

export type SelectionStateKind = SelectionState["kind"];

export type SelectionEvent =
    | { type: "select"; intent: Intent }
    | { type: "resolved"; intentId: number; where: "here"; tabId: string; active: boolean }
    | { type: "resolved"; intentId: number; where: "other" }
    | { type: "resolved"; intentId: number; where: "none" }
    | { type: "tabSwitched"; intentId: number }
    | { type: "layoutReady"; intentId: number }
    | { type: "shown"; intentId: number }
    | { type: "focused"; intentId: number }
    | { type: "verified"; intentId: number; ok: boolean; detail?: string }
    | { type: "remoteDone"; intentId: number; found: boolean }
    | { type: "effectFailed"; intentId: number; detail: string }
    | { type: "deadline"; intentId: number; step: number }
    | { type: "blockGone"; intentId: number }
    /** The user selected something else (another window tab) while an intent was in flight. */
    | { type: "userMoved" };

export type SelectionEffect =
    | { type: "resolve"; intent: Intent }
    | { type: "switchTab"; intentId: number; tabId: string }
    | { type: "awaitLayout"; intentId: number; tabId: string; blockId: string }
    | { type: "show"; intentId: number; tabId: string; blockId: string }
    | { type: "focus"; intentId: number; tabId: string; blockId: string; caret: boolean }
    | { type: "verify"; intentId: number; tabId: string; blockId: string; caret: boolean }
    | { type: "handoff"; intentId: number; blockId: string }
    | { type: "arm"; intentId: number; step: number; ms: number }
    /** The locate flash: played when the pane is shown. */
    | { type: "flash"; blockId: string }
    /** A quiet "Couldn't show <name>: <reason>". */
    | { type: "notify"; intent: Intent; reason: FailReason; detail?: string }
    | { type: "finish"; intentId: number; result: SelectionResult };

/** How long each state may last, in ms; a state past its deadline fails with a named reason. */
export const DEADLINES_MS: Readonly<Partial<Record<SelectionStateKind, number>>> = {
    resolving: 1000,
    switchingTab: 3000,
    awaitingLayout: 1500,
    showing: 200,
    focusing: 200,
    verifying: 300,
    handoff: 3000,
};

export const IDLE: SelectionState = { kind: "idle", step: 0 };

export interface Transition {
    state: SelectionState;
    effects: SelectionEffect[];
}

const same = (state: SelectionState): Transition => ({ state, effects: [] });

/** The intent a state is working on; none when idle or ended. */
export function liveIntent(state: SelectionState): Intent | null {
    return state.kind === "idle" || state.kind === "settled" || state.kind === "failed" ? null : state.intent;
}

type Next =
    | { kind: "resolving" }
    | { kind: "switchingTab"; tabId: string }
    | { kind: "awaitingLayout"; tabId: string }
    | { kind: "showing"; tabId: string; retried: boolean }
    | { kind: "focusing"; tabId: string; retried: boolean }
    | { kind: "verifying"; tabId: string; retried: boolean }
    | { kind: "handoff" }
    | { kind: "settled" }
    | { kind: "failed"; reason: FailReason; detail?: string };

/** Enter `next` for the state's intent: a new step, and the deadline armed for it. */
function enter(from: SelectionState, intent: Intent, next: Next, effects: SelectionEffect[]): Transition {
    const step = from.step + 1;
    const state = { ...next, intent, step } as SelectionState;
    const ms = DEADLINES_MS[next.kind];
    return { state, effects: ms != null ? [...effects, { type: "arm", intentId: intent.id, step, ms }] : effects };
}

function fail(from: SelectionState, intent: Intent, reason: FailReason, detail?: string): Transition {
    return enter(from, intent, { kind: "failed", reason, detail }, [
        { type: "notify", intent, reason, detail },
        { type: "finish", intentId: intent.id, result: { ok: false, reason, detail } },
    ]);
}

/** The next state and the effects, for `event` in `state`. An event about an intent that is no longer
 *  the current one changes nothing: the newest request wins, and what an older one finds later is dropped. */
export function transition(state: SelectionState, event: SelectionEvent): Transition {
    if (event.type === "select") {
        const prev = liveIntent(state);
        const superseded: SelectionEffect[] = prev
            ? [{ type: "finish", intentId: prev.id, result: { ok: false, reason: "superseded" } }]
            : [];
        return enter(state, event.intent, { kind: "resolving" }, [...superseded, { type: "resolve", intent: event.intent }]);
    }
    if (event.type === "userMoved") {
        const prev = liveIntent(state);
        if (!prev) return same(state);
        return {
            state: { kind: "idle", step: state.step + 1 },
            effects: [{ type: "finish", intentId: prev.id, result: { ok: false, reason: "superseded" } }],
        };
    }
    const intent = liveIntent(state);
    if (!intent || event.intentId !== intent.id) return same(state);
    const blockId = intent.blockId;

    switch (event.type) {
        case "resolved": {
            if (state.kind !== "resolving") return same(state);
            if (event.where === "none") return fail(state, intent, "not-found");
            if (event.where === "other") {
                return enter(state, intent, { kind: "handoff" }, [{ type: "handoff", intentId: intent.id, blockId }]);
            }
            return event.active
                ? enter(state, intent, { kind: "awaitingLayout", tabId: event.tabId }, [
                      { type: "awaitLayout", intentId: intent.id, tabId: event.tabId, blockId },
                  ])
                : enter(state, intent, { kind: "switchingTab", tabId: event.tabId }, [
                      { type: "switchTab", intentId: intent.id, tabId: event.tabId },
                  ]);
        }
        case "tabSwitched":
            if (state.kind !== "switchingTab") return same(state);
            return enter(state, intent, { kind: "awaitingLayout", tabId: state.tabId }, [
                { type: "awaitLayout", intentId: intent.id, tabId: state.tabId, blockId },
            ]);
        case "layoutReady":
            if (state.kind !== "awaitingLayout") return same(state);
            return enter(state, intent, { kind: "showing", tabId: state.tabId, retried: false }, [
                { type: "show", intentId: intent.id, tabId: state.tabId, blockId },
            ]);
        case "shown":
            if (state.kind !== "showing") return same(state);
            return enter(state, intent, { kind: "focusing", tabId: state.tabId, retried: state.retried }, [
                { type: "focus", intentId: intent.id, tabId: state.tabId, blockId, caret: intent.focusCaret },
            ]);
        case "focused":
            if (state.kind !== "focusing") return same(state);
            return enter(state, intent, { kind: "verifying", tabId: state.tabId, retried: state.retried }, [
                { type: "verify", intentId: intent.id, tabId: state.tabId, blockId, caret: intent.focusCaret },
            ]);
        case "verified":
            if (state.kind !== "verifying") return same(state);
            if (event.ok) {
                return enter(state, intent, { kind: "settled" }, [
                    { type: "flash", blockId },
                    { type: "finish", intentId: intent.id, result: { ok: true } },
                ]);
            }
            return mismatch(state, intent, event.detail ?? "it did not come to the front");
        case "remoteDone":
            if (state.kind !== "handoff") return same(state);
            return event.found
                ? enter(state, intent, { kind: "settled" }, [{ type: "finish", intentId: intent.id, result: { ok: true } }])
                : fail(state, intent, "not-found");
        case "blockGone":
            return fail(state, intent, "gone");
        case "effectFailed":
            return fail(state, intent, "error", event.detail);
        case "deadline": {
            if (event.step !== state.step) return same(state);
            switch (state.kind) {
                case "resolving":
                    return fail(state, intent, "resolve-timeout");
                case "switchingTab":
                    return fail(state, intent, "tab-timeout");
                case "awaitingLayout":
                    return fail(state, intent, "layout-timeout");
                case "showing":
                case "focusing":
                    return fail(state, intent, "stuck", state.kind);
                case "verifying":
                    return mismatch(state, intent, "the check did not finish");
                case "handoff":
                    return fail(state, intent, "remote-timeout");
                default:
                    return same(state);
            }
        }
    }
}

/** What was read back disagrees with the intent: show it once more, then say so. */
function mismatch(state: SelectionState & { kind: "verifying" }, intent: Intent, detail: string): Transition {
    if (state.retried) return fail(state, intent, "mismatch", detail);
    return enter(state, intent, { kind: "showing", tabId: state.tabId, retried: true }, [
        { type: "show", intentId: intent.id, tabId: state.tabId, blockId: intent.blockId },
    ]);
}
