// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { createSignal, For, type JSX } from "solid-js";
import { rovingTarget } from "./shared";

import "./swatch.scss";

// A type specimen, the same in every language: not text to translate.
const DEFAULT_SAMPLE = "Aa";

export interface SwatchProps {
    /** The colour the swatch is drawn in: a CSS colour, set as a custom property, so a runtime value never becomes a literal in a stylesheet. */
    color: string;
    /** The colour of the sample text drawn on it (a mini-pane); the theme's text colour when omitted. */
    ink?: string;
    /** The name a screen reader says. */
    label: string;
    /** More for a screen reader after the name, e.g. a contrast readout. */
    detail?: string;
    selected: boolean;
    onSelect: () => void;
    /** What the swatch shows as a sample; `Aa` by default. */
    sample?: JSX.Element;
    disabled?: boolean;
    tabIndex?: number;
    ref?: (el: HTMLButtonElement) => void;
    onKeyDown?: (e: KeyboardEvent) => void;
    onPreview?: (on: boolean) => void;
    class?: string;
}

/**
 * One colour sample you can choose: a small pane in the colour, with sample text in its own ink, a ring when it is
 * the chosen one. A radio to assistive tech. Use it inside a `SwatchGrid`, which holds the group and the keys.
 */
export function Swatch(props: SwatchProps): JSX.Element {
    return (
        <button
            ref={props.ref}
            type="button"
            role="radio"
            class={clsx("ui-swatch", props.class)}
            style={{ "--swatch-surface": props.color, "--swatch-ink": props.ink }}
            aria-checked={props.selected ? "true" : "false"}
            aria-label={props.label}
            aria-description={props.detail}
            tabIndex={props.tabIndex ?? 0}
            disabled={props.disabled}
            onClick={() => props.onSelect()}
            onKeyDown={(e) => props.onKeyDown?.(e)}
            onMouseEnter={() => props.onPreview?.(true)}
            onMouseLeave={() => props.onPreview?.(false)}
            onFocus={() => props.onPreview?.(true)}
            onBlur={() => props.onPreview?.(false)}
        >
            <span class="ui-swatch-sample" aria-hidden="true">
                {props.sample ?? DEFAULT_SAMPLE}
            </span>
        </button>
    );
}

export interface SwatchOption<T extends string> {
    value: T;
    label: string;
    color: string;
    ink?: string;
    detail?: string;
    disabled?: boolean;
}

export interface SwatchGridProps<T extends string> {
    options: SwatchOption<T>[];
    /** The chosen option; a value that matches none (or undefined) when the pane has no choice yet. */
    value: T | undefined;
    onChange: (value: T) => void;
    /** Swatches per row; the Up and Down keys move by this much. */
    columns?: number;
    /** The option being looked at (hover or keyboard focus), or `undefined` when none is: for a live preview. */
    onPreview?: (value: T | undefined) => void;
    ariaLabel: string;
    class?: string;
}

/**
 * A grid of swatches, one chosen. Arrow keys move focus across the grid, Up and Down by a row; Home and End go to
 * the ends; Enter or Space chooses the focused one; Escape ends a preview. Unlike a segmented control, moving does
 * not choose: a preview is what moving shows. The whole grid is one Tab stop (the chosen swatch, or the first enabled one).
 */
export function SwatchGrid<T extends string>(props: SwatchGridProps<T>): JSX.Element {
    let group: HTMLDivElement | undefined;
    // The grid's own buttons in their current order, looked up when a key is pressed: a list kept from the ref callbacks
    // would go stale when `For` reuses a button after the options reorder or one is removed.
    const buttonAt = (i: number) => group?.querySelectorAll<HTMLButtonElement>("[role=radio]")[i];
    const columns = () => Math.max(1, props.columns ?? 4);
    const isDisabled = (i: number) => !!props.options[i]?.disabled;
    const chosen = () => props.options.findIndex((o) => o.value === props.value);
    const [active, setActive] = createSignal<number | undefined>(undefined);
    // The one swatch Tab reaches: where focus last was, else the chosen one, else the first; a disabled one is never the
    // stop, so the grid always has a tabbable swatch while any is enabled.
    const tabStop = () => {
        const want = active() ?? Math.max(0, chosen());
        if (want < props.options.length && !isDisabled(want)) return want;
        return Math.max(0, props.options.findIndex((_, i) => !isDisabled(i)));
    };

    const move = (to: number | null) => {
        if (to == null || isDisabled(to)) return;
        setActive(to);
        buttonAt(to)?.focus();
    };

    const onKeyDown = (e: KeyboardEvent, index: number) => {
        if (e.key === "Escape") {
            props.onPreview?.(undefined);
            return;
        }
        if (e.key === "ArrowUp" || e.key === "ArrowDown") {
            const to = index + (e.key === "ArrowUp" ? -columns() : columns());
            // A column stays a column: the keys stop at the top and bottom row.
            if (to < 0 || to >= props.options.length) return e.preventDefault();
            e.preventDefault();
            return move(to);
        }
        const to = rovingTarget(e.key, index, props.options.length, "horizontal", isDisabled);
        if (to == null) return;
        e.preventDefault();
        move(to);
    };

    return (
        <div
            ref={group}
            role="radiogroup"
            aria-label={props.ariaLabel}
            class={clsx("ui-swatch-grid", props.class)}
            style={{ "--swatch-columns": String(columns()) }}
        >
            <For each={props.options}>
                {(option, i) => (
                    <Swatch
                        color={option.color}
                        ink={option.ink}
                        label={option.label}
                        detail={option.detail}
                        disabled={option.disabled}
                        selected={option.value === props.value}
                        tabIndex={i() === tabStop() ? 0 : -1}
                        onSelect={() => props.onChange(option.value)}
                        onKeyDown={(e) => onKeyDown(e, i())}
                        onPreview={(on) => {
                            if (on) setActive(i());
                            props.onPreview?.(on ? option.value : undefined);
                        }}
                    />
                )}
            </For>
        </div>
    );
}
