// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tool-output render caps — bound the DOM a single tool's output body
 * contributes to the conversation, so a long tool call can't tank scroll.
 *
 * Interim mitigation per SPEC_TOOL_OUTPUT_CAP_2026_05_30.md: the agent
 * conversation is virtualized, but a single tool's output body is not — a
 * long Bash run is otherwise one <pre> per chunk (or one enormous <pre>),
 * thousands of in-flow nodes. We cap what is *rendered inline*; the full
 * output stays in ToolNode state and is reachable via "open in pane".
 */

/** Max rendered lines of a single text body (Bash stdout, Read content, …). */
export const MAX_TOOL_OUTPUT_LINES = 1000;

/** Max rendered characters of a single text body. Bounds a payload that fits
 *  the line budget but is one enormous line (minified JSON, base64, a long
 *  compiler line). ~1 MB: comfortably above any real multi-line output, well
 *  below the multi-MB blobs that bloat the conversation DOM. */
export const MAX_TOOL_OUTPUT_CHARS = 1_000_000;

export interface CappedText {
    text: string;
    /** Lines dropped (0 when under budget). */
    hiddenLines: number;
}

/**
 * Cap a text body to `max` lines, keeping the head or the tail. Logs and
 * command output use "tail" (the latest matters); file/diff content uses
 * "head" (read top-down). Exactly-at-budget is not considered capped. The
 * result is additionally bounded to MAX_TOOL_OUTPUT_CHARS so one enormous
 * line (minified JSON, base64) can't render an unbounded <pre>.
 */
export function capText(
    text: string,
    max: number = MAX_TOOL_OUTPUT_LINES,
    from: "head" | "tail" = "tail",
): CappedText {
    if (!text) return { text: text ?? "", hiddenLines: 0 };
    const lines = text.split("\n");
    // A trailing newline yields a phantom empty final segment — exclude it so
    // exactly-`max` lines followed by "\n" is not treated as over budget.
    const body = lines.length > 1 && lines[lines.length - 1] === "" ? lines.slice(0, -1) : lines;
    let hiddenLines = 0;
    let out = text;
    if (body.length > max) {
        const kept = from === "tail" ? body.slice(body.length - max) : body.slice(0, max);
        hiddenLines = body.length - max;
        out = kept.join("\n");
    }
    // Character bound: a payload within the line budget can still be enormous
    // as one long line (minified JSON, base64). Trim by chars with a visible
    // marker so the rendered <pre> stays bounded regardless of line count.
    if (out.length > MAX_TOOL_OUTPUT_CHARS) {
        out = from === "tail"
            ? "…(truncated)\n" + out.slice(out.length - MAX_TOOL_OUTPUT_CHARS)
            : out.slice(0, MAX_TOOL_OUTPUT_CHARS) + "\n…(truncated)";
    }
    return { text: out, hiddenLines };
}

/**
 * Trim a single string to `max` characters (keeping the tail) with a visible
 * marker. For the streamed-chunk path, which renders raw chunk content rather
 * than routing through capText — a single multi-MB one-line chunk would
 * otherwise render an unbounded <pre> even though it is under the line budget.
 */
export function capChars(text: string, max: number = MAX_TOOL_OUTPUT_CHARS): string {
    if (text.length <= max) return text;
    return "…(truncated)\n" + text.slice(text.length - max);
}

export interface CappedChunks<T> {
    chunks: ReadonlyArray<T>;
    /** Lines retained in the rendered window (== total when under budget). */
    keptLines: number;
}

/**
 * Cap an append-only chunk list (one <pre> per chunk) to the tail whose
 * cumulative line count first reaches `maxLines`. Walks ONLY the kept tail —
 * O(kept), never the dropped prefix. Retained chunk objects keep their
 * identity, so a `<For>` keyed by reference only churns at the window
 * boundary. A single oversized boundary chunk is line-trimmed to its tail
 * (not kept whole) so one chunk can't render an unbounded <pre>.
 *
 * Returns the kept line count; pair it with a running total
 * (createChunkCapper) to derive the hidden-line count without ever rescanning
 * the dropped prefix.
 */
export function capChunksByLines<T extends { content: string }>(
    chunks: ReadonlyArray<T>,
    maxLines: number = MAX_TOOL_OUTPUT_LINES,
): CappedChunks<T> {
    let budget = maxLines;
    let i = chunks.length - 1;
    for (; i >= 0; i--) {
        // Each chunk renders one <pre>, so it costs at least one line of
        // budget — otherwise a run of empty chunks would retain unbounded nodes.
        const n = Math.max(1, countLines(chunks[i].content));
        if (n > budget) break; // this chunk overflows the remaining budget
        budget -= n;
    }
    // Under budget: keep everything (kept == total == maxLines - budget).
    if (i < 0) return { chunks, keptLines: maxLines - budget };
    // Capping: the window holds exactly `maxLines` lines (whole tail chunks
    // plus, if the budget didn't land on a chunk boundary, a trimmed boundary).
    if (budget <= 0) {
        return { chunks: chunks.slice(i + 1), keptLines: maxLines };
    }
    const trimmed = capText(chunks[i].content, budget, "tail");
    return {
        chunks: [{ ...chunks[i], content: trimmed.text } as T, ...chunks.slice(i + 1)],
        keptLines: maxLines,
    };
}

/** A capped chunk window plus how many earlier lines are hidden. */
export interface CappedChunkView<T> {
    chunks: ReadonlyArray<T>;
    hiddenLines: number;
}

/**
 * Stateful capper for an append-only chunk stream. Tracks the total line
 * count incrementally — each call scans only the chunks appended since the
 * previous call — so the hidden-line count (total − kept) never rescans the
 * growing dropped prefix (which would reintroduce O(n^2) over a long stream).
 * One instance per stream; on the rare reset (the array shrinks) it recounts.
 */
export function createChunkCapper(maxLines: number = MAX_TOOL_OUTPUT_LINES) {
    let total = 0;
    let counted = 0;
    let anchor: { content: string } | undefined;
    return function cap<T extends { content: string }>(chunks: ReadonlyArray<T>): CappedChunkView<T> {
        // Reset the running total unless this is an append-only continuation of
        // the same stream. A recycled ChunkList can be handed a different
        // tool's chunks — even with the same length — so anchor on the first
        // chunk's identity (stable under append), not just the array length.
        if (chunks.length < counted || chunks[0] !== anchor) {
            total = 0;
            counted = 0;
            anchor = chunks[0];
        }
        // Match capChunksByLines: each chunk costs >= 1 line (it renders a <pre>).
        for (; counted < chunks.length; counted++) total += Math.max(1, countLines(chunks[counted].content));
        const r = capChunksByLines(chunks, maxLines);
        return { chunks: r.chunks, hiddenLines: Math.max(0, total - r.keptLines) };
    };
}

/** Braille and quarter-circle spinner-frame characters used by ora, listr,
 *  tqdm, etc. ASCII chars (-|/\) are intentionally excluded — they also
 *  appear as legitimate single-char output (table separators, diff/graph
 *  markers, line continuations) and would produce false positives. */
const SPINNER_CHARS = new Set([
    '⠋','⠙','⠹','⠸','⠼','⠴','⠦','⠧','⠇','⠏',
    '⣾','⣽','⣻','⢿','⡿','⣟','⣯','⣷',
    '◐','◓','◑','◒','◴','◷','◶','◵',
]);

export interface SpinnerCollapseResult<T> {
    display: T[];
    spinnerSlot: { content: string; kind: string } | null;
}

/** Matches any SPINNER_CHARS glyph, for stripping mid-line occurrences in
 *  `normalizeForCompare` (SPINNER_CHARS.has() alone only matches a chunk
 *  that IS a bare glyph, not one trailing/leading other text). */
const SPINNER_CHAR_RE = new RegExp(`[${[...SPINNER_CHARS].join("")}]`, "g");

/** Below this, two lines are considered a redraw of each other only if they
 *  differ solely by digits/percent/fill-run/spinner-glyph content (spec
 *  §B). The dominant real cases (spinner glyph or percentage trailing
 *  static text) hit the exact-match fast path in `looksLikeRedraw` below
 *  regardless of this threshold — normalizeForCompare already reduces
 *  both frames to an identical string in those cases. This threshold only
 *  governs the narrower fallback (e.g. a progress-bar fill run too short
 *  to hit the {3,} collapse regex). 0.82 (the spec's initial estimate)
 *  false-positived on same-shape-different-content lines seen in real
 *  build output — "Compiling src/main.rs" vs "Compiling src/lib.rs"
 *  scores ~0.86, well above 0.82, but these are NOT a redraw. Raised to
 *  0.92 to keep that firmly below-threshold while still catching
 *  near-total-match progress-bar variations. */
const REDRAW_SIMILARITY_THRESHOLD = 0.92;

/** Strip spinner glyphs, percentages, and progress-bar fill runs so
 *  "Installing... ⠋" / "Installing... ⠙" / "Downloading (45%)" / "(46%)"
 *  normalize to the same or a near-identical string.
 *
 *  The `%` in `\d+%` is NOT optional — a bare digit run with no percent
 *  sign is left alone. `\d+%?` (optional `%`) would strip every plain
 *  number anywhere in the line, so "case 1 passed" / "case 2 passed" (two
 *  genuinely different lines, nothing to do with an animation) would
 *  normalize to the same string and get collapsed into one, silently
 *  discarding real output (reagent P1, PR #2330). */
function normalizeForCompare(s: string): string {
    return s
        .replace(SPINNER_CHAR_RE, "")
        .replace(/\d+%/g, "#")
        .replace(/[#=\-\s]{3,}/g, "#")
        .trim();
}

/** Normalized Levenshtein similarity in [0, 1]; 1 = identical, 0 = totally
 *  dissimilar. Classic O(a*b) DP with a rolling two-row buffer. */
function levenshteinRatio(a: string, b: string): number {
    if (a === b) return 1;
    const maxLen = Math.max(a.length, b.length);
    if (maxLen === 0) return 1;
    let prev = new Array<number>(b.length + 1);
    let curr = new Array<number>(b.length + 1);
    for (let j = 0; j <= b.length; j++) prev[j] = j;
    for (let i = 1; i <= a.length; i++) {
        curr[0] = i;
        for (let j = 1; j <= b.length; j++) {
            const cost = a[i - 1] === b[j - 1] ? 0 : 1;
            curr[j] = Math.min(prev[j] + 1, curr[j - 1] + 1, prev[j - 1] + cost);
        }
        [prev, curr] = [curr, prev];
    }
    return 1 - prev[b.length] / maxLen;
}

/** Above this length, skip the Levenshtein fallback entirely rather than pay
 *  its O(a·b) cost — real spinner/progress lines are short human-readable
 *  text; a single bashwrap chunk can be up to ~8KiB (one large non-progress
 *  chunk, e.g. minified JSON or base64, would otherwise run tens of millions
 *  of DP cells synchronously on the UI thread). The two fast paths above
 *  (exact match, post-strip equality) already catch every real redraw
 *  shape regardless of length; only the fuzzy fallback needs bounding. */
const REDRAW_COMPARE_MAX_LEN = 300;

/** Below this length, skip the Levenshtein fallback too — for the opposite
 *  reason: ratio = 1 - editDistance/maxLen means a fixed similarity
 *  threshold is only meaningful for strings long enough that a genuine
 *  one-character difference doesn't itself exceed it. At
 *  `REDRAW_SIMILARITY_THRESHOLD` (0.92), any two SHORT strings differing by
 *  exactly one character will score >= threshold regardless of what that
 *  character is — "case 1 passed" vs "case 2 passed" (13 chars, real,
 *  unrelated lines that just happen to share a template) scores ~0.923,
 *  same shape as a genuine redraw. reagent's own example, PR #2330. No
 *  currently-legitimate redraw case in this codebase needs the fuzzy path
 *  below this length — every collapsing test in output-cap.test.ts reaches
 *  the exact-match fast path above instead (spinner glyphs and %-progress
 *  both strip to identical strings, not merely similar ones). */
const REDRAW_COMPARE_MIN_LEN = 16;

/**
 * Does `next` look like an in-place terminal redraw of `prev` — a spinner
 * glyph or progress text trailing/leading otherwise-static content, rather
 * than the isolated-bare-glyph case `SPINNER_CHARS` already catches? Used
 * as the fallback when neither line is a whole spinner-glyph chunk (spec
 * §B) — the backend's `\r`/CSI normalization (bash_wrap.rs §A1/A2) handles
 * the structural cases; this catches whatever still slips through as
 * separate chunks.
 */
function looksLikeRedraw(prev: string, next: string): boolean {
    if (prev === next) return false; // identical repeats aren't animation
    const stripPrev = normalizeForCompare(prev);
    const stripNext = normalizeForCompare(next);
    if (!stripPrev && !stripNext) return false; // both fully stripped — no real content to compare
    if (stripPrev === stripNext) return true; // differs only in glyph/%/count
    const maxLen = Math.max(stripPrev.length, stripNext.length);
    if (maxLen > REDRAW_COMPARE_MAX_LEN || maxLen < REDRAW_COMPARE_MIN_LEN) return false;
    return levenshteinRatio(stripPrev, stripNext) >= REDRAW_SIMILARITY_THRESHOLD;
}

/** Does `anchorTrimmed` (a chunk with no accepted run yet) start a run, given
 *  the very next chunk's trimmed content? Note the asymmetry with
 *  `continuesRun` below: this checks SPINNER_CHARS against the ANCHOR itself
 *  (a lone bare glyph starts a run even if what follows doesn't look like a
 *  redraw of it — the run then absorbs purely by each subsequent candidate's
 *  own merits), not against the candidate. `nextTrimmed === null` (anchor is
 *  the current last-known chunk) means there is nothing yet to compare
 *  against, so only the bare-glyph half of the check applies. */
function startsRun(anchorTrimmed: string, nextTrimmed: string | null): boolean {
    return SPINNER_CHARS.has(anchorTrimmed) || (nextTrimmed !== null && looksLikeRedraw(anchorTrimmed, nextTrimmed));
}

/** Does a new candidate frame continue a run whose most-recently-accepted
 *  frame's trimmed content is `lastTrimmed`? Shared by the batch algorithm
 *  (`collapseSpinnerChunks`) and the incremental one (`createSpinnerCollapser`)
 *  so the two can never diverge on what counts as "still redrawing." Checks
 *  SPINNER_CHARS against the CANDIDATE (not the anchor) — see `startsRun`'s
 *  doc for why these two checks are asymmetric. */
function continuesRun(lastTrimmed: string, candidateTrimmed: string): boolean {
    return SPINNER_CHARS.has(candidateTrimmed) || looksLikeRedraw(lastTrimmed, candidateTrimmed);
}

/**
 * Collapse consecutive redraw-of-each-other chunks in a capped chunk array:
 * a bare spinner glyph (`SPINNER_CHARS`, the original narrow case) OR — per
 * spec §B — a spinner glyph/progress text trailing or leading otherwise-
 * static content (`looksLikeRedraw`). Trailing run → spinnerSlot (a single
 * DOM node Solid updates in place). Completed run (followed by unrelated
 * output) → last frame frozen in display.
 *
 * Full rescan of `chunks` every call — fine for one-off use (tests, a
 * completed/static chunk array) but each new streamed chunk arriving during
 * a live run means re-walking and re-comparing (Levenshtein) the ENTIRE
 * capped window again. For a long-running stream (npm/cargo install output
 * is exactly this shape), that is an O(n·L²) rescan on every single chunk
 * (reagent P1, PR #2330) — use `createSpinnerCollapser()` instead for a live
 * stream; it does the same comparisons but only against chunks appended
 * since its last call.
 */
export function collapseSpinnerChunks<T extends { kind: string; content: string }>(
    chunks: ReadonlyArray<T>,
): SpinnerCollapseResult<T> {
    const display: T[] = [];
    let spinnerSlot: { content: string; kind: string } | null = null;
    for (let i = 0; i < chunks.length; i++) {
        const chunk = chunks[i];
        let lastTrimmed = capChars(chunk.content).trim();
        let last = chunk;
        const nextTrimmed = i + 1 < chunks.length ? capChars(chunks[i + 1].content).trim() : null;
        if (startsRun(lastTrimmed, nextTrimmed)) {
            while (i + 1 < chunks.length) {
                const candidate = capChars(chunks[i + 1].content).trim();
                if (!continuesRun(lastTrimmed, candidate)) break;
                i++;
                last = chunks[i];
                lastTrimmed = candidate;
            }
            if (i === chunks.length - 1) {
                spinnerSlot = { content: lastTrimmed, kind: last.kind };
            } else {
                display.push({ ...last, content: lastTrimmed });
                spinnerSlot = null;
            }
        } else {
            display.push(chunk);
            spinnerSlot = null;
        }
    }
    return { display, spinnerSlot };
}

/**
 * Stateful, incremental version of `collapseSpinnerChunks` for a live,
 * append-only chunk stream — mirrors `createChunkCapper`'s pattern (one
 * instance per stream; each call only does work proportional to chunks
 * appended since the previous call, not the whole capped window).
 *
 * Why this is safe to make incremental: a chunk's "does it start/continue a
 * run" decision only ever depends on chunks at or before it, EXCEPT for the
 * single most-recent chunk, whose decision needs the chunk that comes after
 * it — which doesn't exist yet in a live stream. So at most one chunk (or
 * one in-progress run, if it's still growing) is ever "pending" — safe to
 * reconsider on the next call — while everything before that is already
 * fully decided and can be committed to `finalized` for good. This exactly
 * mirrors why `collapseSpinnerChunks`, when rerun from scratch as more
 * chunks arrive, sometimes retroactively groups a previously-standalone
 * trailing chunk into a new run: the pending chunk here is that same
 * retroactively-reconsiderable one, just tracked directly instead of
 * rediscovered by rescanning.
 */
export function createSpinnerCollapser<T extends { kind: string; content: string }>() {
    let finalized: T[] = [];
    // The tail run not yet committed — length 0 (nothing seen yet), 1 (one
    // chunk whose run-membership isn't decided until the next chunk arrives),
    // or >1 (a confirmed run, still possibly growing).
    let pending: T[] = [];
    let pendingLastTrimmed = "";
    let processedCount = 0;
    let anchor: T | undefined;

    return function collapse(chunks: ReadonlyArray<T>): SpinnerCollapseResult<T> {
        // Reset on non-append (new stream / shrink) — same contract as
        // createChunkCapper: anchor on the first chunk's identity, not just
        // length, since a recycled stream could otherwise be same-length.
        if (chunks.length < processedCount || chunks[0] !== anchor) {
            finalized = [];
            pending = [];
            pendingLastTrimmed = "";
            processedCount = 0;
            anchor = chunks[0];
        }

        for (; processedCount < chunks.length; processedCount++) {
            const next = chunks[processedCount];
            const nextTrimmed = capChars(next.content).trim();
            if (pending.length === 0) {
                pending = [next];
                pendingLastTrimmed = nextTrimmed;
                continue;
            }
            // pending.length === 1: this is the "startsRun" decision for
            // pending[0], now that its next chunk (this one) is finally
            // known — checks SPINNER_CHARS against pending[0] itself.
            // pending.length > 1: an already-confirmed run's continuation
            // check — checks SPINNER_CHARS against the new candidate
            // instead. NOT interchangeable (see startsRun/continuesRun's
            // docs) — mixing them up absorbs an unrelated chunk into a run
            // just because it's a bare spinner glyph, regardless of whether
            // the run's own last frame looked anything like it.
            const matches = pending.length === 1
                ? startsRun(pendingLastTrimmed, nextTrimmed)
                : continuesRun(pendingLastTrimmed, nextTrimmed);
            if (matches) {
                pending.push(next);
                pendingLastTrimmed = nextTrimmed;
            } else {
                const last = pending[pending.length - 1];
                finalized.push(pending.length > 1 ? ({ ...last, content: pendingLastTrimmed } as T) : last);
                pending = [next];
                pendingLastTrimmed = nextTrimmed;
            }
        }

        const display = finalized.slice();
        let spinnerSlot: { content: string; kind: string } | null = null;
        if (pending.length > 1) {
            spinnerSlot = { content: pendingLastTrimmed, kind: pending[pending.length - 1].kind };
        } else if (pending.length === 1) {
            if (SPINNER_CHARS.has(pendingLastTrimmed)) {
                spinnerSlot = { content: pendingLastTrimmed, kind: pending[0].kind };
            } else {
                // Not (yet) confirmed as a run — collapseSpinnerChunks would
                // show it in `display` for THIS snapshot too (its startsRun
                // check only rules out SPINNER_CHARS without a next chunk to
                // compare against). Kept in `pending`, not `finalized`, so a
                // future chunk can still retroactively absorb it into a run.
                display.push(pending[0]);
            }
        }
        return { display, spinnerSlot };
    };
}

/** Visual line count of a string (newline-delimited), allocation-free. */
function countLines(s: string): number {
    if (!s) return 0;
    let n = 1;
    for (let i = 0; i < s.length; i++) {
        if (s.charCodeAt(i) === 10 /* \n */) n++;
    }
    // A trailing newline terminates the last line — don't count a phantom.
    if (s.charCodeAt(s.length - 1) === 10) n--;
    return n;
}
