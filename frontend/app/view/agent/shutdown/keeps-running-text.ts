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

/** "1 agent keeps running without a window", "3 agents keep running without a window". */
export function windowKeepsRunningLine(count: number): string {
    return count === 1 ? "1 agent keeps running without a window" : `${count} agents keep running without a window`;
}

/** "KCont keeps running without a pane", "A and B keep…", "A, B and 2 more keep…". */
export function keepsRunningTitle(names: string[]): string {
    if (names.length === 1) return `${names[0]} keeps running without a pane`;
    const shown = names.length === 2 ? `${names[0]} and ${names[1]}` : `${names[0]}, ${names[1]} and ${names.length - 2} more`;
    return `${shown} keep running without a pane`;
}
