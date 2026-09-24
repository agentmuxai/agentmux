// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Shared vocabulary of the universal install dialog
 * (SPEC_UNIVERSAL_INSTALL_DIALOG_2026_09_23.md §5–§7): step rows, line
 * tones, error categories, and the `StepTracker` contract every install
 * kind's classifier implements so `InstallSession` can drive any of them.
 */

export type InstallStepStatus = "pending" | "active" | "done" | "failed" | "skipped";

export interface InstallStep<Id extends string = string> {
    id: Id;
    label: string;
    status: InstallStepStatus;
    /** Right-aligned short fact, e.g. "189 packages". */
    hint?: string;
    /** One muted live line under the step, e.g. "Downloading chalk". */
    subline?: string;
}

/** How Details should colour a line. Only `error`/`warning` get colour. */
export type LineTone = "normal" | "command" | "warning" | "error";

export type InstallErrorCategory =
    | "network"
    | "permission"
    | "disk"
    | "missing_prereq"
    | "not_on_path"
    | "cancelled"
    | "unknown";

export interface InstallFailure {
    category: InstallErrorCategory;
    /** Plain-language Layer 1 message. */
    message: string;
    /** 0-based index into the lines fed to `line()` of the first error line. */
    firstErrorLine?: number;
}

/**
 * Turns one install kind's raw output into Layer 1 steps. Pure and
 * clock-injected so recorded logs replay deterministically in tests.
 */
export interface StepTracker {
    /** Current steps, as a fresh copy safe to hand to a signal. */
    snapshot(): InstallStep[];
    /** Resets every step for a new run (also used by Retry). */
    start(): void;
    /** Classifies one output line, advances steps, returns its tone. */
    line(text: string, now: number): LineTone;
    /** Called periodically while running, for time-based transitions. */
    tick(now: number): void;
    /** The install finished successfully. */
    succeed(): void;
    /** The install failed; `error` is the `done` event's string or typed error. */
    fail(error: unknown): InstallFailure;
}

// The backend's typed `AgentMuxError` codes (agentmux-common errors.rs),
// e.g. when creating an install directory fails before anything runs.
const TYPED_CODE_CATEGORY: Record<string, InstallErrorCategory> = {
    "AMX-IO-001": "disk",
    "AMX-IO-002": "permission",
};

export function typedErrorCategory(error: unknown): InstallErrorCategory | undefined {
    if (error == null || typeof error !== "object") return undefined;
    const code = (error as { code?: unknown }).code;
    return typeof code === "string" ? TYPED_CODE_CATEGORY[code] : undefined;
}

/** The Layer 1 message for a category (spec §6.3 table). */
export function failureMessage(
    category: InstallErrorCategory,
    opts: { name: string; failedLabel: string; missingTool?: string },
): string {
    switch (category) {
        case "network":
            return "Couldn't reach the package server.";
        case "permission":
            return "AgentMux wasn't allowed to write the files.";
        case "disk":
            return "The disk is full.";
        case "missing_prereq":
            return `${opts.missingTool ?? "A required tool"} is needed first.`;
        case "not_on_path":
            return `Installed, but AgentMux can't find ${opts.name} yet.`;
        case "cancelled":
            return "Cancelled. Nothing was left behind.";
        default:
            return `Something went wrong while running "${opts.failedLabel}".`;
    }
}
