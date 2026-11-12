// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The words and the setting rule of the keeps-running close, apart from the stores so a test can load them.

/** The setting as srv reads it (`always_available::enabled`): on unless it says false, "false", "off", "no" or 0. */
export function keepsRunningSetting(value: unknown): boolean {
    if (typeof value === "boolean") return value;
    if (typeof value === "string") return !["false", "off", "no", "0"].includes(value.trim().toLowerCase());
    if (typeof value === "number") return value !== 0;
    return true;
}

/** What a close that parked agents tells the person: closing keeps them, and Shut down is what ends one. */
export const KEEPS_RUNNING_NOTE = "Closing keeps it running. Shut down ends it.";

/** "KCont keeps running without a pane", "A and B keep…", "A, B and 2 more keep…". */
export function keepsRunningTitle(names: string[]): string {
    if (names.length === 1) return `${names[0]} keeps running without a pane`;
    const shown = names.length === 2 ? `${names[0]} and ${names[1]}` : `${names[0]}, ${names[1]} and ${names.length - 2} more`;
    return `${shown} keep running without a pane`;
}

/** The toast while the Shutdown works: one agent by name, several by count. */
export function shuttingDownTitle(names: string[]): string {
    return names.length === 1 ? `Shutting down ${names[0]}…` : `Shutting down ${names.length} agents…`;
}

/** How far several agents have got. */
export function shuttingDownProgress(done: number, total: number): string {
    return `${done} of ${total} done`;
}

/** The toast when every one was shut down. */
export function shutDownTitle(names: string[]): string {
    return names.length === 1 ? `${names[0]} was shut down` : `${names.length} agents were shut down`;
}

/** The toast when some could not be: the one that failed by name, several by count. */
export function notShutDownTitle(failed: string[], total: number): string {
    return failed.length === 1 ? `Couldn't shut down ${failed[0]}` : `Couldn't shut down ${failed.length} of ${total} agents`;
}

/** `text` as a sentence of a toast's message: ends in a full stop (or the mark it has), so the next one reads apart. */
export function sentence(text: string): string {
    const trimmed = text.trim();
    return /[.!?…]$/.test(trimmed) ? trimmed : `${trimmed}.`;
}
