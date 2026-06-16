// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PaneRow — the shared "auxiliary pin" row primitive for the agent pane.
 *
 * One uniform, status-accented row: sigil + title + meta + tail + actions,
 * with an optional inline-expanded body slot. It is the chrome the pinned
 * ActivityDock rows use and the forthcoming fork bar will reuse, so every
 * pin-shaped surface shares one look, one set of status accents, and one
 * interaction model instead of bespoke per-surface markup.
 *
 * Presentational only — it owns no data and derives nothing. Callers pass a
 * fully-resolved row (the "derive pins from a source of truth" rule lives in
 * the caller). Spec: docs/specs/SPEC_AGENT_PANE_FORKS_AND_AUX_PINS_2026_06_15.md §5.2.
 */

import clsx from "clsx";
import { For, Show, type JSX } from "solid-js";
import "./PaneRow.scss";

/** Status accent — drives the 3px left-border colour (and dim for terminal). */
export type PaneRowAccent =
    | "running"
    | "active"
    | "idle"
    | "done"
    | "error"
    | "stopped"
    | "neutral";

export interface PaneRowAction {
    /** Single-glyph button label (e.g. "■" stop, "×" dismiss, "⌫" close). */
    glyph: string;
    /** Optional text rendered after the glyph (e.g. "Retry now", "Login Again").
     *  Turns the icon button into a labelled button for action-heavy rows like
     *  the failure-recovery row. */
    label?: string;
    /** Accessible name / tooltip. */
    title: string;
    onClick: () => void;
    /** Tints the glyph on hover with the error colour (destructive actions). */
    danger?: boolean;
    /** Emphasise as the primary action (filled accent). */
    primary?: boolean;
    /** Disable the button (e.g. a Retry mid-flight). */
    disabled?: boolean;
}

export interface PaneRowProps {
    sigil: string;
    title: string;
    /** Compact right-of-title metadata (elapsed, token totals, branch label…). */
    meta?: string;
    /** Latest line / last message — ellipsised, monospace. */
    tail?: string;
    /** Status accent; defaults to "neutral". */
    accent?: PaneRowAccent;
    actions?: PaneRowAction[];
    /** Applies the expanded modifier and reveals the body slot. */
    expanded?: boolean;
    /** Clicking the summary (e.g. toggle-expand or switch-to). */
    onActivate?: () => void;
    /** Inline-expanded content (live log, preview…), shown when `expanded`. */
    children?: JSX.Element;
}

export const PaneRow = (props: PaneRowProps): JSX.Element => {
    return (
        <div
            class={clsx("pane-row", `pane-row--${props.accent ?? "neutral"}`, {
                "pane-row--expanded": props.expanded,
            })}
        >
            <div
                class="pane-row-summary"
                onClick={() => props.onActivate?.()}
            >
                <span class="pane-row-sigil" aria-hidden="true">{props.sigil}</span>
                <span class="pane-row-title">{props.title}</span>
                <Show when={props.meta}>
                    <span class="pane-row-meta">{props.meta}</span>
                </Show>
                <Show when={props.tail}>
                    <span class="pane-row-tail">↳ {props.tail}</span>
                </Show>
                <For each={props.actions ?? []}>
                    {(action) => (
                        <button
                            type="button"
                            class={clsx("pane-row-action", {
                                "pane-row-action--danger": action.danger,
                                "pane-row-action--labeled": action.label,
                                "pane-row-action--primary": action.primary,
                            })}
                            title={action.title}
                            aria-label={action.title}
                            disabled={action.disabled}
                            // stopPropagation so an action never also fires onActivate.
                            onClick={(e) => { e.stopPropagation(); action.onClick(); }}
                        >
                            <span aria-hidden="true">{action.glyph}</span>
                            <Show when={action.label}>
                                <span class="pane-row-action-text">{action.label}</span>
                            </Show>
                        </button>
                    )}
                </For>
            </div>

            <Show when={props.expanded && props.children}>
                <div class="pane-row-body" onClick={(e) => e.stopPropagation()}>
                    {props.children}
                </div>
            </Show>
        </div>
    );
};

PaneRow.displayName = "PaneRow";
