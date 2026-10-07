// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Opening a file at a line: `pane.open {view: "editor", file, line}` writes the
// one-shot `editor:line`, which Remotes' "Edit in ssh config" uses to land on
// the host's `Host` line (SPEC_REMOTES_PANE_2026_10_05.md §4.3).

import type { EditorState, TransactionSpec } from "@codemirror/state";
import { EditorView } from "codemirror";

export const META_OPEN_AT_LINE = "editor:line";

/** The 1-based line `meta` asks its `file` to open at, or `null`. */
export function readOpenAtLine(meta: Record<string, unknown> | undefined, file: unknown): number | null {
    const line = meta?.[META_OPEN_AT_LINE];
    if (typeof file !== "string" || !file) return null;
    if (typeof line !== "number" || !Number.isInteger(line) || line < 1) return null;
    return line;
}

/** Puts the cursor at the start of `line` (clamped to the document) and
 *  scrolls it to the middle of the view. */
export function cursorAtLine(state: EditorState, line: number): TransactionSpec {
    const target = state.doc.line(Math.min(Math.max(1, line), state.doc.lines));
    return {
        selection: { anchor: target.from },
        effects: EditorView.scrollIntoView(target.from, { y: "center" }),
    };
}
