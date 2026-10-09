// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The structured form every tool preview is rendered from
 * (docs/reports/REPORT_TOOL_PREVIEW_TEXT_PIPELINE_2026_10_08.md §5.1).
 *
 * Builders in this folder turn raw tool output into a {@link PreviewDoc}; one
 * renderer (`components/PreviewLines.tsx`) draws any of them. Everything that
 * is not the content itself — line numbers, diff markers, which stream a line
 * came from, how much was cut — is data here, never text, so it can't take
 * part in tab stops, indentation, highlighting or wrapping.
 */

/** What the text is. Decides tab width and the default wrap mode. */
export type PreviewKind = "code" | "diff" | "output" | "json" | "prose";

/** Whether long lines scroll sideways or wrap. */
export type PreviewMode = "scroll" | "wrap";

export type DiffMarker = "+" | "-" | " " | "@@";

/** A run of text with terminal colour classes (from SGR codes). */
export interface StyleSpan {
    text: string;
    /** Tailwind classes from `element/ansiline.tsx`'s map; "" for plain. */
    classes: string;
}

export interface PreviewLine {
    /** The line's text: no line break, no control characters, tabs expanded. */
    text: string;
    /** Terminal colours, when the source had any. Joined, equals `text`. */
    spans?: StyleSpan[];
    /** A Read's line number, from the CLI's gutter. */
    number?: number;
    /** A diff line's marker. */
    marker?: DiffMarker;
    /** Output from a command's error stream, or a note from the wrapper
     *  around it (bashwrap), rather than from the command. */
    stream?: "stderr" | "system";
}

export interface PreviewDoc {
    kind: PreviewKind;
    /** Shiki language id for code and diffs; absent or "text" for none. */
    lang?: string;
    lines: PreviewLine[];
    /** Lines left out by the cap: from the end ("head" kept the start) or the start ("tail"). */
    hidden?: { count: number; from: "head" | "tail" };
}
