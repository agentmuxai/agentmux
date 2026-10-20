// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { Show, type JSX } from "solid-js";

import "./pill.scss";

/**
 * What a pill says: neutral (a plain fact: "managed", "checking"), ok,
 * warn (needs attention), error, or accent (a call to notice).
 */
export type PillTone = "neutral" | "ok" | "warn" | "error" | "accent";

export interface PillProps {
    /** Default `neutral`. */
    tone?: PillTone;
    /** A Font Awesome solid icon name without its prefix: "check", "circle-up". Decorative: the words carry the meaning. */
    icon?: string;
    /** Longer explanation, shown as the tooltip. */
    title?: string;
    class?: string;
    children: JSX.Element;
}

/**
 * A small text status chip. The wash and the line come from the theme's own
 * semantic colours; the text stays the main text colour, because those
 * colours are not tuned for 11px text on a wash of themselves
 * (pill-colors.test.ts measures every theme). Status is therefore carried by
 * the words and the icon as well as the colour, never by colour alone.
 */
export function Pill(props: PillProps): JSX.Element {
    return (
        <span class={clsx("ui-pill", `ui-pill--${props.tone ?? "neutral"}`, props.class)} title={props.title}>
            <Show when={props.icon}>
                <i class={`fa-solid fa-${props.icon}`} aria-hidden="true" />
            </Show>
            {props.children}
        </span>
    );
}
