// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Indentation handling for tool previews (Read/Write/Edit). Two concerns,
 * applied in this order by the `format*Preview` entry points at the bottom:
 *
 *  1. **Dedent** — a mid-file snippet (a Read at an offset, an Edit hunk deep
 *     inside a nested scope) carries the source file's original leading
 *     indentation on every line even though the preview can't show the
 *     enclosing scopes that indentation refers to. Stripping the indentation
 *     common to every displayed line makes the shallowest line render
 *     flush-left while keeping the *relative* indentation between lines — the
 *     part that actually encodes structure.
 *
 *  2. **Narrow** (`normalizeIndentWidth`) — dedent only removes what's
 *     *common*, so a preview that includes a column-0 line keeps every deeper
 *     line at full width. A 4-space-per-level file still spends 20 columns on
 *     five levels of nesting, and no CSS property can narrow a space the way
 *     `tab-size` narrows a tab. Rescaling the leading run to a 2-column unit
 *     is the only lever that works on space-indented source.
 *
 * The Read line-number gutter is no longer text: preview-text/docs.ts takes
 * it off as line numbers before these run, and expands tabs first, so both
 * functions here only ever see spaces
 * (docs/reports/REPORT_TOOL_PREVIEW_TEXT_PIPELINE_2026_10_08.md).
 *
 * SPEC_TOOL_PREVIEW_DEDENT_2026_08_08.md,
 * docs/analysis/tool-preview-indentation-and-wrapping-2026-09-02.md.
 */

import { TRUNCATED_MARKER } from "./output-cap";

/** Target width, in columns, of one level of indentation in a preview. Tabs
 *  in code expand to the same width (preview-text/tabs.ts `CODE_TAB_WIDTH`),
 *  so tab- and space-indented files render at the same width. */
export const PREVIEW_INDENT_UNIT = 2;

/** Leading run of spaces/tabs. `\r` is deliberately excluded so a CRLF
 *  line's trailing `\r` (see `splitLines`) never gets treated as part of
 *  the indent. */
const LEADING_WHITESPACE_RE = /^[ \t]*/;

/** Is `line` ignorable for common-indent purposes — a blank line, or
 *  `capText`'s char-budget truncation marker (`output-cap.ts`)? Both can
 *  land inside otherwise-uniformly-indented preview text: a blank line
 *  carries no indentation signal, and the marker is a column-0 line
 *  injected by capping, not real file content. Either would otherwise force
 *  the common prefix to empty (or, for the numbered Read variant, break the
 *  "every non-blank line is numbered" check and fall through to a
 *  whole-text dedent that finds no common prefix at all) — so both are
 *  excluded from the computation and left unstripped in the output, exactly
 *  like a blank line already was. */
function isIgnorableForIndent(line: string): boolean {
    return line.trim() === "" || line === TRUNCATED_MARKER;
}

/** Split on `\n`, tolerating a CRLF source (`\r` stays attached to each
 *  line here; callers that need to inspect content should trim it, but
 *  dedent itself only ever reads/removes the LEADING whitespace run, which
 *  `LEADING_WHITESPACE_RE` never includes `\r` in — so a trailing `\r` is
 *  simply carried through untouched in both the prefix computation and the
 *  stripped output). */
function splitLines(text: string): string[] {
    return text.split("\n");
}

/** Literal longest-common-leading-whitespace prefix across `lines`,
 *  ignoring lines {@link isIgnorableForIndent} flags (blank lines, and
 *  `capText`'s truncation marker) for the computation — neither carries an
 *  indentation signal and either would otherwise force the common prefix to
 *  empty for an otherwise-uniformly-indented snippet. Compared as a literal
 *  string prefix (not by counting tab-equivalent columns), so tabs and
 *  spaces are never conflated: `"\t\tfoo"` and `"    foo"` share no common
 *  prefix and dedent is correctly a no-op — there is no reliable way to know
 *  how wide a tab renders, so guessing would sometimes be wrong; declining
 *  to dedent is always safe. */
function commonLeadingWhitespace(lines: readonly string[]): string {
    let common: string | null = null;
    for (const line of lines) {
        if (isIgnorableForIndent(line)) continue;
        const indent = LEADING_WHITESPACE_RE.exec(line)![0];
        if (common === null) {
            common = indent;
            continue;
        }
        // Shrink `common` to the literal shared prefix of itself and `indent`.
        let i = 0;
        const max = Math.min(common.length, indent.length);
        while (i < max && common[i] === indent[i]) i++;
        common = common.slice(0, i);
        if (common === "") break; // already empty — no further line can add one
    }
    return common ?? "";
}

/** Strip `prefix` (a literal leading-whitespace run) from every line of
 *  `lines` that isn't {@link isIgnorableForIndent}; blank lines and the
 *  truncation marker pass through unchanged — there is nothing meaningful
 *  to strip from either. */
function stripPrefixFromLines(lines: readonly string[], prefix: string): string[] {
    if (prefix === "") return lines.slice();
    return lines.map((line) => (isIgnorableForIndent(line) ? line : line.slice(prefix.length)));
}

/**
 * Strip the whitespace indentation common to every non-blank line of
 * `text`, so the shallowest line renders flush-left. Relative indentation
 * between lines is always preserved — only the shared prefix is removed.
 * A no-op (returns `text` unchanged, no allocation) when there is no
 * common indentation to strip, which covers the common case of an
 * already-flush Write body.
 */
export function stripCommonIndent(text: string): string {
    if (!text) return text;
    const lines = splitLines(text);
    const prefix = commonLeadingWhitespace(lines);
    if (prefix === "") return text;
    return stripPrefixFromLines(lines, prefix).join("\n");
}

/** Greatest common divisor, for inferring a file's indent unit. */
function gcd(a: number, b: number): number {
    while (b !== 0) {
        const t = b;
        b = a % b;
        a = t;
    }
    return a;
}

/**
 * Rescale each line's leading whitespace so one level of indentation renders
 * `targetUnit` columns wide instead of whatever the source file used.
 *
 * The unit is inferred as the GCD of every non-zero leading-run width, which
 * makes the transform **self-limiting in exactly the right way**: a file
 * indented in clean multiples (4, 8) yields a GCD equal to its indent unit and
 * gets rescaled, while a file containing continuation-alignment lines (`foo(a,`
 * / 19 spaces / `b)`) yields a GCD of 1 and is left completely alone. Guessing
 * a unit that isn't really there would shift aligned code out of alignment —
 * so, exactly like {@link commonLeadingWhitespace}'s refusal to conflate tabs
 * with spaces, this declines rather than guesses.
 *
 * A no-op (returns `text` unchanged) when: any leading run contains a tab (its
 * rendered width is a CSS `tab-size` concern, not a character count — see the
 * module header), no line is indented at all, or the inferred unit is already
 * at or below `targetUnit`.
 *
 * Only the LEADING run is touched. Whitespace inside a line — aligned trailing
 * comments, ASCII tables — is never rewritten.
 */
export function normalizeIndentWidth(text: string, targetUnit: number = PREVIEW_INDENT_UNIT): string {
    if (!text) return text;
    const lines = splitLines(text);

    const widths: number[] = [];
    for (const line of lines) {
        if (isIgnorableForIndent(line)) continue;
        const indent = LEADING_WHITESPACE_RE.exec(line)![0];
        if (indent.includes("\t")) return text; // tab width is CSS's business
        if (indent.length > 0) widths.push(indent.length);
    }
    if (widths.length === 0) return text;

    let unit = widths[0];
    for (const w of widths) unit = gcd(unit, w);
    if (unit <= targetUnit) return text;

    return lines
        .map((line) => {
            if (isIgnorableForIndent(line)) return line;
            const indent = LEADING_WHITESPACE_RE.exec(line)![0];
            if (indent.length === 0) return line;
            return " ".repeat((indent.length / unit) * targetUnit) + line.slice(indent.length);
        })
        .join("\n");
}

/**
 * Full preview pipeline for non-Read code bodies (Write, and anything else
 * that arrives as plain source): dedent to flush-left, then narrow the
 * remaining relative indentation.
 */
export function formatCodePreview(text: string): string {
    return normalizeIndentWidth(stripCommonIndent(text));
}

/**
 * Preview pipeline for text that will be handed to a **Markdown renderer**:
 * dedent only, **never** {@link normalizeIndentWidth}.
 *
 * Markdown is indentation-sensitive in a way source code is not. Four leading
 * spaces make an indented code block; rescaling them to two turns that block
 * into ordinary prose, and the same rescale silently re-nests nested lists.
 * Width normalisation is a readability win for a syntax-highlighted source
 * preview and a correctness bug for a rendered one (codex P2 on PR #2958).
 *
 * Dedent is kept because it only removes a prefix *common to every line*, which
 * is the pre-existing behaviour for this path and does not change relative
 * structure.
 */
export function formatMarkdownPreview(text: string): string {
    return stripCommonIndent(text);
}

/**
 * Dedent an Edit hunk's `old_string`/`new_string` pair with ONE shared
 * prefix computed across both sides together, then applied to each
 * independently — not two independent dedents. A shared prefix preserves
 * add/del line alignment; if one side happened to have a shallower minimum
 * indent than the other (e.g. `new_string` adds a less-nested wrapper
 * line), an independent per-side dedent would shift that side by a
 * different amount than the other, manufacturing a phantom indentation
 * diff that was never actually part of the edit.
 */
export function stripCommonIndentSharedPrefix(oldStr: string, newStr: string): { oldStr: string; newStr: string } {
    const oldLines = splitLines(oldStr ?? "");
    const newLines = splitLines(newStr ?? "");
    const prefix = commonLeadingWhitespace([...oldLines, ...newLines]);
    if (prefix === "") return { oldStr, newStr };
    return {
        oldStr: stripPrefixFromLines(oldLines, prefix).join("\n"),
        newStr: stripPrefixFromLines(newLines, prefix).join("\n"),
    };
}

/**
 * Full preview pipeline for an Edit hunk: the shared-prefix dedent above,
 * then the indent narrowing — with ONE unit inferred across both sides
 * together, for exactly the same reason the dedent prefix is shared.
 * Inferring per-side could pick a different unit for each (say `old` contains
 * a continuation-alignment line that collapses its GCD to 1 while `new`
 * doesn't) and rescale one side but not the other, manufacturing an
 * indentation diff that was never part of the edit.
 *
 * Must run BEFORE the diff is built, while the two sides are still plain
 * source: once `+`/`-` markers are prepended, the leading run every function
 * in this module looks at is the marker, not the indentation.
 */
export function formatDiffSides(oldStr: string, newStr: string): { oldStr: string; newStr: string } {
    const stripped = stripCommonIndentSharedPrefix(oldStr ?? "", newStr ?? "");
    const oldLines = splitLines(stripped.oldStr);
    const newLines = splitLines(stripped.newStr);
    // Line count is preserved by normalizeIndentWidth, so the split is exact.
    const combined = splitLines(normalizeIndentWidth([...oldLines, ...newLines].join("\n")));
    return {
        oldStr: combined.slice(0, oldLines.length).join("\n"),
        newStr: combined.slice(oldLines.length).join("\n"),
    };
}
