// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Safe split points for incremental markdown rendering.
 *
 * During agent streaming, `<Markdown>` re-parses the entire growing message on
 * every commit (~11/sec). That is O(n) per commit and O(n²) over a message:
 * rendering one 32KB response burns 1.7s of main thread in blocks up to 92ms,
 * which starves keystrokes in the composer. Measured in
 * `docs/analysis/ANALYSIS_AGENT_PANE_TYPING_UNDER_LOAD_2026_09_22.md` §6.
 *
 * The fix is to parse each byte once: freeze the part of the message that can
 * no longer change meaning, and re-parse only the trailing open block. This
 * module decides where that freeze line can safely go.
 *
 * SAFETY IS THE WHOLE POINT. Splitting markdown is only sound where the two
 * halves parse to the same thing separately as they would together. Markdown
 * has several constructs that reach across blank lines, so this is deliberately
 * CONSERVATIVE: it returns -1 ("do not split, parse it all") whenever it cannot
 * prove a point is safe. A missed split costs performance; a wrong split
 * corrupts what the user reads. Those are not comparable, so this errs hard in
 * one direction.
 */

/**
 * Link reference definitions (`[label]: https://…`) and footnote definitions
 * can be *used* anywhere in the document, including before they appear. Split
 * the document and a reference in one half can no longer see its definition in
 * the other, silently degrading a link to literal text. Cheap to detect, and
 * rare enough in agent output that bailing entirely costs little.
 */
const REF_DEFINITION = /^ {0,3}\[[^\]]+\]:/m;
const FOOTNOTE = /\[\^[^\]]+\]/;

/**
 * A tail may only begin with something that unambiguously STARTS a block.
 * Rejected leading characters, and why each would be unsafe:
 *   -  *  +   list item — would start a NEW list instead of continuing one,
 *              and `-`/`=` also form setext heading underlines, which would
 *              retroactively turn the prefix's last line into a heading
 *   digit.     ordered list item — same continuation problem
 *   >          blockquote — would start a new quote rather than continue
 *   |          table row — would lose its header row and stop being a table
 *   =          setext heading underline
 *   whitespace indented code block, or a lazy continuation line
 */
const UNSAFE_TAIL_START = /^(?:[-*+>|=]|\d+[.)]|[ \t])/;

/** Below this, splitting is not worth the bookkeeping. */
const MIN_PREFIX_CHARS = 512;

/**
 * Returns the offset at which `text` may be cut into an independently-parseable
 * prefix and tail, or -1 when no such point can be proven safe.
 *
 * The returned offset is always just past a blank line, at top level — never
 * inside a fenced code block or an HTML comment, both of which survive blank
 * lines and would otherwise be cut in half.
 */
export function findSafeSplitPoint(text: string): number {
    if (text.length < MIN_PREFIX_CHARS) return -1;
    if (REF_DEFINITION.test(text) || FOOTNOTE.test(text)) return -1;

    let inFence = false;
    let fenceMarker = "";
    let inComment = false;
    let lastSafe = -1;

    let lineStart = 0;
    let prevLineBlank = false;

    while (lineStart <= text.length) {
        let lineEnd = text.indexOf("\n", lineStart);
        if (lineEnd === -1) lineEnd = text.length;
        const line = text.slice(lineStart, lineEnd);
        const trimmed = line.trim();

        if (inComment) {
            if (trimmed.includes("-->")) inComment = false;
        } else if (inFence) {
            // A closing fence must be at least as long as the opener and use
            // the same character; anything else is fence content.
            if (trimmed.startsWith(fenceMarker)) {
                inFence = false;
                fenceMarker = "";
            }
        } else {
            const fence = /^(`{3,}|~{3,})/.exec(trimmed);
            if (fence) {
                inFence = true;
                fenceMarker = fence[1];
            } else if (trimmed.startsWith("<!--") && !trimmed.includes("-->")) {
                inComment = true;
            } else if (prevLineBlank && trimmed.length > 0 && !UNSAFE_TAIL_START.test(line)) {
                // `lineStart` begins a fresh top-level block and everything
                // before it is closed — a provably safe cut.
                if (lineStart >= MIN_PREFIX_CHARS) lastSafe = lineStart;
            }
        }

        prevLineBlank = !inFence && !inComment && trimmed.length === 0;
        if (lineEnd === text.length) break;
        lineStart = lineEnd + 1;
    }

    return lastSafe;
}
