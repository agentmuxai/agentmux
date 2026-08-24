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

/** Leading run of spaces/tabs. `\r` is deliberately excluded so a CRLF
 *  line's trailing `\r` (see `splitLines`) never gets treated as part of
 *  the indent. */
const LEADING_WHITESPACE_RE = /^[ \t]*/;

/** A Claude Code `Read` result line: `<line number><tab><code>`. Matched
 *  per-line (not per-text) so a partially-numbered body — which shouldn't
 *  occur in practice, but isn't assumed away — degrades to the plain
 *  dedent path rather than corrupting a subset of lines. */
const NUMBERED_LINE_RE = /^\s*\d+\t/;

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
 *  ignoring blank (whitespace-only) lines for the computation — a blank
 *  line carries no indentation signal and would otherwise force the
 *  common prefix to empty for an all-but-one-blank-line snippet. Compared
 *  as a literal string prefix (not by counting tab-equivalent columns), so
 *  tabs and spaces are never conflated: `"\t\tfoo"` and `"    foo"` share
 *  no common prefix and dedent is correctly a no-op — there is no reliable
 *  way to know how wide a tab renders, so guessing would sometimes be
 *  wrong; declining to dedent is always safe. */
function commonLeadingWhitespace(lines: readonly string[]): string {
    let common: string | null = null;
    for (const line of lines) {
        if (line.trim() === "") continue; // blank line — no signal
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

/** Strip `prefix` (a literal leading-whitespace run) from every non-blank
 *  line of `lines`; blank lines pass through unchanged (there is nothing
 *  to strip, and forcing them to `""` would be indistinguishable from
 *  already-empty input — they already read as empty either way). */
function stripPrefixFromLines(lines: readonly string[], prefix: string): string[] {
    if (prefix === "") return lines.slice();
    return lines.map((line) => (line.trim() === "" ? line : line.slice(prefix.length)));
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
    const nonBlank = lines.filter((l) => l.trim() !== "");
    const allNumbered = nonBlank.length > 0 && nonBlank.every((l) => NUMBERED_LINE_RE.test(l));
    if (!allNumbered) return stripCommonIndent(text);

    const numberPrefixes: string[] = [];
    const codes: string[] = [];
    for (const line of lines) {
        if (line.trim() === "") {
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
