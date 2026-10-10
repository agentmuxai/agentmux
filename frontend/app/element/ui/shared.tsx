// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Shared pieces of the element/ui/ component set
// (SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md §5).

import clsx from "clsx";
import type { JSX } from "solid-js";

/** What a control is for, not what colour it is. Each maps to theme tokens.
 *  `attention` (filled) is only for the main action of a call to action an
 *  agent waits on. */
export type UiTone = "accent" | "neutral" | "danger" | "quiet" | "attention";

/**
 * `comfortable` (the default) for full panes and modals; `compact` for the
 * composer strip, drawers and popovers. Omit it to inherit from the nearest
 * container that sets a density class.
 */
export type UiDensity = "compact" | "comfortable";

export function densityClass(density: UiDensity | undefined): string | undefined {
    return density ? `ui-density-${density}` : undefined;
}

export function toneClass(tone: UiTone | undefined): string {
    return `ui-tone-${tone ?? "neutral"}`;
}

/** A Font Awesome icon by bare name (`"plus"`), decorative only. */
export function UiIcon(props: { name: string; spin?: boolean }): JSX.Element {
    return <i class={clsx("fa-sharp fa-solid", `fa-${props.name}`, props.spin && "fa-spin")} aria-hidden="true" />;
}

/**
 * Roving-focus key handling for a row or column of options (tabs, segmented
 * control). Returns the index to move to, or null when the key isn't a
 * navigation key. Disabled indices are skipped; movement wraps.
 */
export function rovingTarget(
    key: string,
    current: number,
    count: number,
    orientation: "horizontal" | "vertical",
    isDisabled: (index: number) => boolean = () => false
): number | null {
    if (count === 0) return null;
    const prevKeys = orientation === "horizontal" ? ["ArrowLeft"] : ["ArrowUp"];
    const nextKeys = orientation === "horizontal" ? ["ArrowRight"] : ["ArrowDown"];
    let step: number;
    let start: number;
    if (prevKeys.includes(key)) {
        step = -1;
        start = current;
    } else if (nextKeys.includes(key)) {
        step = 1;
        start = current;
    } else if (key === "Home") {
        step = 1;
        start = -1;
    } else if (key === "End") {
        step = -1;
        start = count;
    } else {
        return null;
    }
    for (let i = 1; i <= count; i++) {
        const candidate = (((start + step * i) % count) + count) % count;
        if (!isDisabled(candidate)) return candidate;
    }
    return null;
}
