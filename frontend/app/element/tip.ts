// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The attributes that give an element a hover label, without a native `title=` and without a
// wrapper element (a <Tooltip> is a <div>, which breaks a table cell, an inline run of text
// and an ellipsis). title-tooltip.ts shows `data-hover-tip` through the shared near-instant tooltip
// (the status bar's `data-tip` is its own, StatusBarTip.tsx).
//
//     <span class="name" {...tip(props.fullName)}>{shortName()}</span>
//     <i class="dot" {...tip("Unseen", { label: true })} />
//
// The label is also the element's accessible description, or its accessible name when the
// element has no text of its own (`label: true`), as a native title would be. Never an
// `aria-label` on an element with text: it would replace that text. A control keeps the kit's
// Button / IconButton, and a tip never goes on an element that is itself `disabled` (a disabled
// control gets no pointer events): put it on a wrapper (`span.ui-tooltip-anchor`).

export interface TipAttributes {
    "data-hover-tip"?: string;
    "aria-description"?: string;
    "aria-label"?: string;
}

/** Attributes for an element that shows `text` on hover; none when `text` is empty. */
export function tip(text: string | null | undefined, options?: { label?: boolean }): TipAttributes {
    if (!text) return {};
    return options?.label ? { "data-hover-tip": text, "aria-label": text } : { "data-hover-tip": text, "aria-description": text };
}
