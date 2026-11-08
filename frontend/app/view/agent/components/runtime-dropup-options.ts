// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** What AgentRuntimeDropup offers and the rows it lays out: the option lists, the mode colours and the row types. */

import type { PermissionMode } from "../../../agent-model/types";
import type { ProviderModel } from "../providers";

// The same colours as text (the mode's name): the status colours' text variants, which pass AA on every theme.
export const PERMISSION_TEXT_COLORS: Record<PermissionMode, string> = {
    bypass: "var(--error-text-color)",
    auto: "var(--accent-fg-color, var(--accent-color))",
    acceptEdits: "var(--warning-text-color)",
    plan: "var(--success-text-color)",
    default: "var(--main-text-color)",
};

export const PERMISSION_COLORS: Record<PermissionMode, string> = {
    bypass: "var(--error-color)",
    auto: "var(--accent-color)",
    acceptEdits: "var(--warning-color)",
    plan: "var(--success-color)",
    default: "var(--main-text-color)",
};

// Mode: trigger shows the short `label`; the panel shows `menuLabel` (the
// descriptive form) when present — matches the prior StripSelect convention.
export const MODE_OPTIONS = [
    { value: "bypass", label: "Bypass", menuLabel: "Bypass (no prompts)" },
    { value: "auto", label: "Auto", menuLabel: "Auto (AI classifier)" },
    { value: "acceptEdits", label: "Accept Edits" },
    { value: "plan", label: "Plan", menuLabel: "Plan (read-only)" },
    { value: "default", label: "Default", menuLabel: "Default (prompt all)" },
] as const;

// Fallback when a provider defines no static model list — matches the prior
// StripSelect fallback exactly.
export const FALLBACK_MODEL_OPTIONS: ProviderModel[] = [
    { value: "opus", label: "Opus" },
    { value: "sonnet", label: "Sonnet" },
    { value: "haiku", label: "Haiku" },
];

export const EFFORT_OPTIONS = [
    { value: "low", label: "low" },
    { value: "medium", label: "medium" },
    { value: "high", label: "high" },
    { value: "xhigh", label: "xhigh" },
    { value: "max", label: "max" },
] as const;

export type Section = "mode" | "model" | "effort";

export interface OptionRow {
    section: Section;
    value: string;
    label: string;
    description?: string;
    /** The full explanation, shown as the row's tooltip (the description is its short form). */
    detail?: string;
    current: boolean;
    color?: string;
    /** Shown but not selectable: the choice would be recorded and do nothing. */
    disabled?: boolean;
}

export type Row =
    // `note`: why the section's options do not apply, on the header's line.
    | { kind: "header"; section: Section; note?: string }
    // A heading inside a section: a model route's name, with the billing note that goes with it.
    | { kind: "group"; section: Section; text: string; note?: string }
    // What the process runs, under the Model rows; `placeholder` until it is known.
    | { kind: "note"; section: Section; text: string; placeholder?: boolean }
    | (OptionRow & { kind: "option" });

export interface AgentRuntimeDropupProps {
    blockId: string;
    blockAtom: () => Block | undefined;
    providerId: string;
    /** The resolved model id on the last main-agent reply, if there has been one. */
    lastReplyModel?: string | null;
    /** How much the conversation holds now (the harness's last usage report), for the cost of re-reading it on another model. */
    contextTokens?: number | null;
}
