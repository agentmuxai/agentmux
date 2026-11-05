// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A dock row's title is the whole summary on one line: the row gives it every pixel up to the stats
 * on the right and its own width cuts it with an ellipsis. Only a runaway text is cut here, at
 * `DOCK_LABEL_MAX_CHARS`, and the whole text goes in `fullTitle`, which the row shows as its tooltip.
 */

export const DOCK_LABEL_MAX_CHARS = 240;

/** `title` for a row, and `fullTitle` when the text had to be cut. */
export function dockTitle(text: string): { title: string; fullTitle?: string } {
    const full = text.trim().split(/\s+/).join(" ");
    if (full.length <= DOCK_LABEL_MAX_CHARS) return { title: full };
    return { title: `${full.slice(0, DOCK_LABEL_MAX_CHARS).trimEnd()}…`, fullTitle: full };
}
