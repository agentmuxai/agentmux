// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a button does with a click handler that returns a promise: it locks at once, shows a spinner if the action
 * runs longer than `PENDING_SPINNER_DELAY_MS`, and unlocks when the promise settles, however it ends. Shared by the
 * kit's `Button` and `CloseButton`
 * (docs/specs/SPEC_BUTTON_PRESSED_AND_PENDING_FEEDBACK_2026_11_20.md §4).
 */

import { batch, createSignal, type JSX, onCleanup } from "solid-js";
import { describeButton, logSlowPress, recordPress, TRACE_LOG_MS } from "./button-trace";

/**
 * How long an action runs before its button shows a spinner. Under the point where a wait starts to feel like one
 * (about 100 to 200 ms), so a quick action never flashes one; the press and the lock show at once.
 */
export const PENDING_SPINNER_DELAY_MS = 120;

/** A button still pending after this adds a status line for a screen reader. */
export const WORKING_ANNOUNCE_MS = TRACE_LOG_MS;

/**
 * What a button does while its click handler's promise is unsettled. Default (omitted): it locks, so a second click
 * does nothing, and shows a spinner once `PENDING_SPINNER_DELAY_MS` has passed. `"lock"`: it locks and never shows a
 * spinner. `false`: neither, for a button meant to be pressed again before the first has finished (a stepper) or whose
 * promise is a person's answer, not work.
 */
export type PendingMode = false | "lock";

export type ClickHandler = JSX.EventHandlerUnion<HTMLButtonElement, MouseEvent>;
export type ClickEvent = MouseEvent & { currentTarget: HTMLButtonElement; target: Element };

/** Run Solid's two handler forms: `fn`, and `[fn, data]`. */
function runHandler(handler: ClickHandler | undefined, e: ClickEvent): unknown {
    if (!handler) return undefined;
    if (typeof handler === "function") return handler(e);
    const [fn, data] = handler as [(data: unknown, e: ClickEvent) => unknown, unknown];
    return fn(data, e);
}

const isPromise = (r: unknown): r is PromiseLike<unknown> => typeof (r as { then?: unknown } | null | undefined)?.then === "function";

export interface PendingPress {
    /** The click handler to put on the button. */
    click: (e: ClickEvent) => void;
    /** An action is running: the button is locked. */
    pending: () => boolean;
    /** The spinner shows: pending past the delay, and the mode allows one. */
    spinning: () => boolean;
    /** Pending past a second: a screen reader is told. */
    slow: () => boolean;
}

/**
 * @param handler the caller's `onClick`, read at click time
 * @param mode the caller's `pending` prop, read at click time
 * @param skip true for a button that never goes pending (a toggle: its result is its new state)
 */
export function createPendingPress(handler: () => ClickHandler | undefined, mode: () => PendingMode | undefined, skip: () => boolean): PendingPress {
    const [pending, setPending] = createSignal(false);
    const [late, setLate] = createSignal(false);
    const [slow, setSlow] = createSignal(false);
    let timers: ReturnType<typeof setTimeout>[] = [];
    const clearTimers = () => {
        timers.forEach(clearTimeout);
        timers = [];
    };
    onCleanup(clearTimers);

    const track = (result: PromiseLike<unknown>, e: ClickEvent) => {
        const started = performance.now();
        const { label, testId } = describeButton(e.currentTarget);
        setPending(true);
        timers.push(setTimeout(() => setLate(true), PENDING_SPINNER_DELAY_MS));
        timers.push(
            setTimeout(() => {
                setSlow(true);
                logSlowPress(label);
            }, WORKING_ANNOUNCE_MS)
        );
        const settle = (failed: boolean) => {
            clearTimers();
            batch(() => {
                setPending(false);
                setLate(false);
                setSlow(false);
            });
            recordPress({ label, testId, ms: performance.now() - started, failed });
        };
        // A failure is rethrown through the derived promise, so it reaches the error path an unhandled rejection
        // reaches today (log/error-forwarder.ts). Nothing is hidden and no toast is added here.
        result.then(
            () => settle(false),
            (err) => {
                settle(true);
                throw err;
            }
        );
    };

    const click = (e: ClickEvent) => {
        if (pending()) {
            e.preventDefault();
            return;
        }
        const result = runHandler(handler(), e);
        if (mode() !== false && !skip() && isPromise(result)) track(result, e);
    };

    return { click, pending, spinning: () => mode() !== "lock" && pending() && late(), slow };
}
