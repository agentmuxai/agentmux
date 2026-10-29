// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Button } from "@/app/element/ui";
import { createEffect, Show, type JSX } from "solid-js";

export interface NamePromptProps {
    /** Prompt above the input, e.g. "Rename to:". */
    label: string;
    placeholder: string;
    value: string;
    loading: boolean;
    error: string | null;
    /** Text on the confirm button; swapped for "…" while loading. */
    submitLabel: string;
    inputTestid: string;
    submitTestid: string;
    onInput: (value: string) => void;
    onSubmit: () => void;
    onCancel: () => void;
}

/**
 * The "type a name, Enter to confirm, Escape to back out" sub-panel, shared
 * by the fork prompt's session naming and the row menu's Rename
 * (docs/specs/SPEC_AGENT_DELETE_2026_09_16.md §4.3 — Rename was specified to
 * "reuse the existing fork-prompt naming sub-pattern", which until now meant
 * a second copy of the same markup, autofocus effect and key handling).
 * Purely presentational: every piece of state lives in the caller's own
 * per-row map.
 */
export const NamePrompt = (props: NamePromptProps): JSX.Element => (
    <div class="agent-fork-naming">
        <label class="agent-fork-label">{props.label}</label>
        <div class="agent-fork-input-row">
            <input
                type="text"
                class="agent-fork-input"
                value={props.value}
                disabled={props.loading}
                placeholder={props.placeholder}
                data-testid={props.inputTestid}
                onInput={(e) => props.onInput(e.currentTarget.value)}
                onKeyDown={(e) => {
                    if (e.key === "Enter") props.onSubmit();
                    if (e.key === "Escape") props.onCancel();
                }}
                ref={(el) => {
                    createEffect(() => {
                        if (!props.loading) el.focus();
                    });
                }}
            />
            <Button
                tone="accent"
                class="agent-fork-btn agent-fork-btn--primary"
                disabled={props.loading || !props.value.trim()}
                onClick={() => props.onSubmit()}
                data-testid={props.submitTestid}
            >
                {props.loading ? "…" : props.submitLabel}
            </Button>
            <Button
                tone="quiet"
                class="agent-fork-btn agent-fork-btn--ghost"
                onClick={() => props.onCancel()}
                aria-label="Cancel"
            >
                ✕
            </Button>
        </div>
        <Show when={props.error}>
            <span class="agent-fork-error">{props.error}</span>
        </Show>
    </div>
);

NamePrompt.displayName = "NamePrompt";
