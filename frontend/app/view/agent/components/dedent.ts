// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Common-indentation stripping for tool previews (Read/Write/Edit). A
 * mid-file snippet — a Read at an offset, or an Edit hunk deep inside a
 * nested scope — carries the source file's original leading indentation on
 * every line even though the preview can't show the enclosing scopes that
 * indentation refers to. Stripping the indentation common to every
 * displayed line makes the shallowest line render flush-left while keeping
 * the *relative* indentation between lines — the part that actually
 * encodes structure.
 *
 * SPEC_TOOL_PREVIEW_DEDENT_2026_08_08.md.
 */

import { TRUNCATED_MARKER } from "./output-cap";

/** Leading run of spaces/tabs. `\r` is deliberately excluded so a CRLF
 *  line's trailing `\r` (see `splitLines`) never gets treated as part of
 *  the indent. */
const LEADING_WHITESPACE_RE = /^[ \t]*/;

/** A Claude Code `Read` result line: `<line number><tab><code>`. Matched
 *  per-line (not per-text) so a partially-numbered body — which shouldn't
 *  occur in practice, but isn't assumed away — degrades to the plain
 *  dedent path rather than corrupting a subset of lines. */
const NUMBERED_LINE_RE = /^\s*\d+\t/;

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

/**
 * Read-tool variant of {@link stripCommonIndent}: Claude Code's `Read`
 * result lines are `<N>\t<code>` (verified against real session
 * transcripts) — a plain whole-line dedent would see every line start with
 * a different digit and find no common prefix at all. When every non-blank
 * line matches that shape, this splits off the `<N>\t` prefix, dedents only
 * the code portions together (so relative indentation across lines is
 * still computed correctly), and rejoins. Any line that doesn't match the
 * numbered shape — including a completely unnumbered body, e.g. if this
 * ever runs on non-Read content — falls through to the plain
 * {@link stripCommonIndent} over the whole text, which is always safe (at
 * worst a no-op).
 */
export function stripCommonIndentNumbered(text: string): string {
    if (!text) return text;
    const lines = splitLines(text);
    // Blank lines AND the truncation marker are excluded from the
    // "every line is numbered" check, same rationale as everywhere else in
    // this file: neither is real Read content, so requiring either to match
    // the `<N>\t` shape would wrongly reject an otherwise-fully-numbered,
    // truncated body and fall through to a whole-text dedent that finds no
    // common prefix at all (every real line still starts with a digit).
    const relevant = lines.filter((l) => !isIgnorableForIndent(l));
    const allNumbered = relevant.length > 0 && relevant.every((l) => NUMBERED_LINE_RE.test(l));
    if (!allNumbered) return stripCommonIndent(text);

    const numberPrefixes: string[] = [];
    const codes: string[] = [];
    for (const line of lines) {
        if (isIgnorableForIndent(line)) {
            numberPrefixes.push("");
            codes.push(line);
            continue;
        }
        const m = NUMBERED_LINE_RE.exec(line)!;
        numberPrefixes.push(m[0]);
        codes.push(line.slice(m[0].length));
    }
    const commonCodeIndent = commonLeadingWhitespace(codes);
    if (commonCodeIndent === "") return text;
    const dedentedCodes = stripPrefixFromLines(codes, commonCodeIndent);
    return lines.map((_, i) => numberPrefixes[i] + dedentedCodes[i]).join("\n");
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
