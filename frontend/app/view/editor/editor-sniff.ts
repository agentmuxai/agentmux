// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Sniff content that's unsafe to hand to CodeMirror. Returns a human-readable
 *  refusal message (used as the tab's loadError) or null when the content is
 *  fine to render. Cheap — scans up to the first 4096 bytes for NUL.
 *
 *  Why this lives in the model: CodeMirror will faithfully try to lay out
 *  half-a-megabyte of binary on one line (the trigger case was a Windows
 *  NTUSER.DAT regtrans-ms file at 512KB) and block the renderer indefinitely.
 *  The 10MB read cap doesn't catch it because the file is small. */
const MAX_BYTES_FOR_NUL_SNIFF = 4096;
const MAX_SINGLE_LINE_CHARS = 100_000;

export function sniffUnopenable(content: string, allowBinary = false): string | null {
    // NUL-byte sniff — any U+0000 in the first few KB means binary. Skipped
    // for "Open as text anyway" (the user asked to see the bytes); the
    // long-line guard below never is.
    const sniffLen = allowBinary ? 0 : Math.min(content.length, MAX_BYTES_FOR_NUL_SNIFF);
    for (let i = 0; i < sniffLen; i++) {
        if (content.charCodeAt(i) === 0) {
            return "This file looks binary (contains NUL bytes) and can't be displayed in the editor.";
        }
    }
    // Single-line-too-long sniff — even a "text" file is unsafe to render if
    // it has no newline within the first 100K chars. CodeMirror's layout
    // engine handles long files line-by-line; a single 100K+ line stalls it.
    const firstNewline = content.indexOf("\n");
    const lineLen = firstNewline === -1 ? content.length : firstNewline;
    if (lineLen > MAX_SINGLE_LINE_CHARS) {
        return `This file's first line is ${lineLen.toLocaleString()} characters long — too wide to render. Open it in an external editor.`;
    }
    return null;
}
