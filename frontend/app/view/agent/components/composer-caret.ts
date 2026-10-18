// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Where the composer's caret is, for ArrowUp/ArrowDown history recall
// (AgentFooter.tsx).

/**
 * Resolves the offset of the selection's "moving" end — the one that
 * ArrowUp/ArrowDown (with or without Shift) actually relocates. For a
 * backward-direction selection (anchor after focus — e.g. a collapsed
 * cursor extended upward via Shift+ArrowUp), that's `selectionStart`; for
 * "forward" or "none" (anchor before/equal to focus — includes every
 * collapsed-cursor case, where start===end so direction is moot), that's
 * `selectionEnd`. Getting this wrong measures the *fixed* anchor instead of
 * the caret that's actually moving for a forward-direction selection (e.g.
 * shift-click below the original cursor, then Shift+ArrowUp to extend
 * further) — see docs/specs/SPEC_COMPOSER_SHIFT_UP_SELECTION_VS_HISTORY_RACE_2026-08-11.md §2.3.
 */
export function activeSelectionEdge(ta: HTMLTextAreaElement): number {
    return ta.selectionDirection === "backward" ? ta.selectionStart : ta.selectionEnd;
}

/**
 * Returns whether the textarea's active selection edge (see
 * `activeSelectionEdge`) is at the true start/end of the content — i.e.
 * ArrowUp/ArrowDown has nowhere left to move, so it's safe to intercept for
 * history navigation instead.
 *
 * This used to be a "first/last *visual* line" check (a mirror-div
 * measurement replicating the textarea's CSS to detect soft-wrapped row
 * boundaries — see the 2026-06-23 retro,
 * docs/retro/RETRO_COMPOSER_ARROWKEY_HISTORY_SOFT_WRAP_2026_06_23.md). That
 * was solving a real problem (character-scanning `\n` alone can't detect
 * soft-wrap boundaries) but in service of an insufficiently strict
 * condition: being on visual row 0 doesn't mean there's nowhere left to
 * move within the line (ArrowUp can still legitimately move the caret to
 * column 0 of that same row) — only true absolute position 0 means that.
 * Once the condition is tightened to require the true start/end of content,
 * the visual-row measurement becomes unnecessary entirely: position 0 is
 * always the very first character of the very first visual row regardless
 * of wrapping, and a plain string-length comparison answers the question
 * more simply, more cheaply (no DOM mutation per keystroke), and more
 * correctly (also fixes the it-should-take-one-more-keystroke bug — see the
 * spec cited above) than the mirror-div approach ever could.
 */
export function caretAtSelectionEdge(ta: HTMLTextAreaElement): { first: boolean; last: boolean } {
    const pos = activeSelectionEdge(ta);
    return { first: pos === 0, last: pos === ta.value.length };
}
