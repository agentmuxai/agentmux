// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { JSX, splitProps } from "solid-js";

import { Button as LineButton, type UiTone } from "./ui";

// The legacy button, now drawn by element/ui's line-style Button
// (SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md §5.6). Its old
// `className` vocabulary is mapped to a tone here, once, so every call site
// changed look without being edited. New code imports from "./ui" and
// passes `tone` directly.

interface ButtonProps extends Omit<JSX.ButtonHTMLAttributes<HTMLButtonElement>, "class"> {
    className?: string;
    children?: JSX.Element;
}

/** The legacy category and colour words; everything else in `className` is passed through. */
const LEGACY_WORDS = new Set(["solid", "outlined", "outline", "ghost", "green", "grey", "red", "yellow"]);

/**
 * Tone for a legacy `className`. The old default (no words) was a solid
 * accent block, so it maps to `accent` — except a modal's dismiss button,
 * which was the same block as its main action and is the neutral one.
 */
export function legacyTone(className: string | undefined, isDismiss: boolean): UiTone {
    const words = new Set((className ?? "").split(/\s+/));
    if (words.has("red")) return "danger";
    if (words.has("ghost")) return "quiet";
    if (words.has("grey") || words.has("yellow")) return "neutral";
    return isDismiss ? "neutral" : "accent";
}

function Button(props: ButtonProps): JSX.Element {
    const [local, rest] = splitProps(props, ["className", "children"]);
    const kept = () => (local.className ?? "").split(/\s+/).filter((w) => w && !LEGACY_WORDS.has(w));
    const isDismiss = () => (rest as Record<string, unknown>)["data-modal-dismiss"] != null;
    return (
        // `wave-button` stays: a few surfaces (tab close, block header, menu
        // button) still size it in context.
        <LineButton {...rest} tone={legacyTone(local.className, isDismiss())} class={clsx("wave-button", kept())}>
            {local.children}
        </LineButton>
    );
}

export { Button };
