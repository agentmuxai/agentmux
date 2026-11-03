// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A dock row reads as a short ambient phrase, not the command or a long
 * description. The row keeps its first `DOCK_LABEL_WORDS` words; the text it was cut from goes in
 * `fullTitle`, which the row shows as its tooltip.
 */

export const DOCK_LABEL_WORDS = 5;

/** `title` for a row, and `fullTitle` when the text had to be cut. */
export function dockTitle(text: string): { title: string; fullTitle?: string } {
    const full = text.trim();
    const words = full.split(/\s+/);
    if (words.length <= DOCK_LABEL_WORDS) return { title: full };
    return { title: `${words.slice(0, DOCK_LABEL_WORDS).join(" ")}…`, fullTitle: full };
}
