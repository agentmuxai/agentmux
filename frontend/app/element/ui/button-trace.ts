// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The presses that stayed pending: a ring of the last 50 that took more than 300 ms, read from the console as
 * `__buttonTrace()`. It is the list of actions to make fast, produced by use instead of guessed: a spinner makes a
 * slow button look like it is working, and this says which ones were slow
 * (docs/specs/SPEC_BUTTON_PRESSED_AND_PENDING_FEEDBACK_2026_11_20.md §6).
 */

export const TRACE_SIZE = 50;
/** A press pending longer than this goes in the ring. */
export const TRACE_MIN_MS = 300;
/** A press still pending after this logs one console line. */
export const TRACE_LOG_MS = 1000;

export interface ButtonPress {
    /** The button's accessible name: its `aria-label`, else its text. */
    label: string;
    testId?: string;
    /** How long it was pending. */
    ms: number;
    failed: boolean;
}

const ring: ButtonPress[] = [];

/** What a button is called in the trace: the accessible name, at most 60 characters. */
export function describeButton(el: HTMLElement): { label: string; testId?: string } {
    const name = el.getAttribute("aria-label") || el.textContent || "";
    const label = name.replace(/\s+/g, " ").trim().slice(0, 60) || "(no name)";
    const testId = el.getAttribute("data-testid") ?? undefined;
    return testId ? { label, testId } : { label };
}

/** A press settled: kept when it was slow. */
export function recordPress(press: ButtonPress): void {
    if (press.ms <= TRACE_MIN_MS) return;
    ring.push({ ...press, ms: Math.round(press.ms) });
    if (ring.length > TRACE_SIZE) ring.shift();
}

/** A press is still pending after `TRACE_LOG_MS`: one console line, so a stuck button names itself. */
export function logSlowPress(label: string): void {
    console.warn(`[button] "${label}" is still pending after ${TRACE_LOG_MS} ms`);
}

export function buttonTrace(): ButtonPress[] {
    return ring.slice();
}

export function clearButtonTrace(): void {
    ring.length = 0;
}

(globalThis as { __buttonTrace?: () => ButtonPress[] }).__buttonTrace = buttonTrace;
