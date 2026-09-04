// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    formatMarkdownPreview,
    formatCodePreview,
    formatReadPreview,
    normalizeIndentWidth,
    renderNumberedGutter,
    splitNumberedGutter,
    stripCommonIndent,
    stripCommonIndentSharedPrefix,
} from "./dedent";
import { TRUNCATED_MARKER } from "./output-cap";

describe("stripCommonIndent", () => {
    it("strips a uniform space indent to flush, preserving relative levels", () => {
        const input = "    if (x) {\n        doThing();\n    }";
        const out = stripCommonIndent(input);
        expect(out).toBe("if (x) {\n    doThing();\n}");
    });

    it("strips a uniform tab indent by literal tab prefix", () => {
        const input = "\tif (x) {\n\t\tdoThing();\n\t}";
        const out = stripCommonIndent(input);
        expect(out).toBe("if (x) {\n\tdoThing();\n}");
    });

    it("is a no-op when tabs and spaces disagree at the same depth (no true common prefix)", () => {
        const input = "\tfoo();\n    bar();";
        expect(stripCommonIndent(input)).toBe(input);
    });

    it("ignores blank lines when computing the common prefix, and leaves them empty in output", () => {
        const input = "    a();\n\n    b();";
        expect(stripCommonIndent(input)).toBe("a();\n\nb();");
    });

    it("ignores whitespace-only blank lines the same as fully-empty ones", () => {
        const input = "    a();\n   \n    b();";
        const out = stripCommonIndent(input);
        expect(out).toBe("a();\n   \nb();");
    });

    it("is a no-op for already-flush content (column-0 lines)", () => {
        const input = "a();\nb();\nif (x) {\n    c();\n}";
        expect(stripCommonIndent(input)).toBe(input);
    });

    it("handles a single-line input", () => {
        expect(stripCommonIndent("    solo();")).toBe("solo();");
    });

    it("handles all-blank content as a no-op", () => {
        expect(stripCommonIndent("   \n\t\n   ")).toBe("   \n\t\n   ");
    });

    it("handles an empty string", () => {
        expect(stripCommonIndent("")).toBe("");
    });

    it("does not treat a CRLF line's trailing \\r as part of the indent", () => {
        const input = "    a();\r\n    b();\r";
        const out = stripCommonIndent(input);
        expect(out).toBe("a();\r\nb();\r");
    });

    it("dedents by the SHALLOWEST line's indent, not the deepest", () => {
        const input = "  outer();\n    inner();\n  outer2();";
        const out = stripCommonIndent(input);
        expect(out).toBe("outer();\n  inner();\nouter2();");
    });

    it("ignores capText's truncation marker line when computing the common prefix, and leaves it unstripped", () => {
        const input = `    a();\n    b();\n${TRUNCATED_MARKER}`;
        const out = stripCommonIndent(input);
        expect(out).toBe(`a();\nb();\n${TRUNCATED_MARKER}`);
    });
});

describe("normalizeIndentWidth", () => {
    it("rescales a 4-space-per-level file to 2 columns per level", () => {
        const input = "a();\n    b();\n        c();\n            d();";
        expect(normalizeIndentWidth(input)).toBe("a();\n  b();\n    c();\n      d();");
    });

    it("rescales an 8-space unit the same way — the unit is inferred, not assumed", () => {
        const input = "a();\n        b();\n                c();";
        expect(normalizeIndentWidth(input)).toBe("a();\n  b();\n    c();");
    });

    it("rescales a 3-space unit", () => {
        expect(normalizeIndentWidth("a();\n   b();\n      c();")).toBe("a();\n  b();\n    c();");
    });

    it("is a no-op when the file is already at or below the target unit", () => {
        const input = "a();\n  b();\n    c();";
        expect(normalizeIndentWidth(input)).toBe(input);
    });

    it("DECLINES when a continuation-alignment line makes the unit irregular", () => {
        // `19` is not a multiple of 4, so the GCD collapses to 1 and the whole
        // transform backs off — rescaling would shift `b` out of alignment
        // with `a`. This is the property that makes the heuristic safe.
        const input = "    return foo(a,\n                   b);\n        next();";
        expect(normalizeIndentWidth(input)).toBe(input);
    });

    it("is a no-op on tab-indented text — tab width is CSS's business", () => {
        const input = "a();\n\tb();\n\t\tc();";
        expect(normalizeIndentWidth(input)).toBe(input);
    });

    it("is a no-op when tabs and spaces are mixed in a leading run", () => {
        const input = "a();\n    b();\n\t    c();";
        expect(normalizeIndentWidth(input)).toBe(input);
    });

    it("never rewrites whitespace inside a line, only the leading run", () => {
        const input = "a();\n    b = 1;    // aligned comment";
        expect(normalizeIndentWidth(input)).toBe("a();\n  b = 1;    // aligned comment");
    });

    it("leaves blank lines and the truncation marker untouched", () => {
        const input = `    a();\n\n${TRUNCATED_MARKER}\n        b();`;
        expect(normalizeIndentWidth(input)).toBe(`  a();\n\n${TRUNCATED_MARKER}\n    b();`);
    });

    it("is a no-op when nothing is indented, and on empty input", () => {
        expect(normalizeIndentWidth("a();\nb();")).toBe("a();\nb();");
        expect(normalizeIndentWidth("")).toBe("");
    });

    it("honours an explicit target unit", () => {
        expect(normalizeIndentWidth("a();\n    b();", 1)).toBe("a();\n b();");
    });
});

describe("splitNumberedGutter", () => {
    it("splits the <N>\\t prefix off every line", () => {
        const out = splitNumberedGutter("9\t  a();\n10\t  b();")!;
        expect(out.numbers).toEqual(["9", "10"]);
        expect(out.body).toBe("  a();\n  b();");
    });

    it("keeps positional alignment for blank lines and the marker", () => {
        const out = splitNumberedGutter(`1\ta();\n2\t\n${TRUNCATED_MARKER}`)!;
        expect(out.numbers).toEqual(["1", "2", ""]);
        expect(out.body).toBe(`a();\n\n${TRUNCATED_MARKER}`);
    });

    it("returns null for non-uniformly-numbered and for unnumbered input", () => {
        expect(splitNumberedGutter("1\tfoo();\nnot numbered\n3\tbar();")).toBeNull();
        expect(splitNumberedGutter("    a();\n    b();")).toBeNull();
        expect(splitNumberedGutter("")).toBeNull();
    });
});

describe("renderNumberedGutter", () => {
    it("right-aligns to a fixed width so the code column never steps sideways", () => {
        // The whole point: with the original `9\t` / `10\t`, the code column
        // moved at the 9→10 boundary. Here both land at column 3.
        expect(renderNumberedGutter(["9", "10"], "a();\nb();")).toBe(" 9 a();\n10 b();");
    });

    it("emits no trailing pad for a blank code line", () => {
        expect(renderNumberedGutter(["1", "2", "3"], "a();\n\nb();")).toBe("1 a();\n2\n3 b();");
    });

    it("leaves unnumbered lines (marker) at column 0", () => {
        expect(renderNumberedGutter(["1", ""], `a();\n${TRUNCATED_MARKER}`)).toBe(`1 a();\n${TRUNCATED_MARKER}`);
    });
});

describe("formatReadPreview", () => {
    it("dedents, narrows, and re-emits an aligned gutter — end to end", () => {
        const input = "80\t        unsetEnv: [\"CLAUDECODE\"],\n81\t            authConfigDirEnvVar: \"X\",";
        const out = formatReadPreview(input);
        // common 8-space prefix stripped; the remaining 4-space level narrowed
        // to 2; gutter right-aligned at width 2 with a single space.
        expect(out.withGutter).toBe('80 unsetEnv: ["CLAUDECODE"],\n81   authConfigDirEnvVar: "X",');
        // `body` feeds the Markdown renderer, so it is dedent-only — the 4-space
        // relative level survives rather than being halved (codex P2 on #2958).
        expect(out.body).toBe('unsetEnv: ["CLAUDECODE"],\n    authConfigDirEnvVar: "X",');
    });

    it("removes the 9→10 column step that the raw <N>\\t gutter produced", () => {
        const out = formatReadPreview("9\ta();\n10\tb();").withGutter;
        const [l1, l2] = out.split("\n");
        expect(l1.indexOf("a();")).toBe(l2.indexOf("b();"));
    });

    it("body drops the gutter entirely, for the markdown path", () => {
        const out = formatReadPreview("1\t# Title\n2\t\n3\tsome text");
        expect(out.body).toBe("# Title\n\nsome text");
    });

    it("degrades to plain dedent+narrow for unnumbered input", () => {
        const out = formatReadPreview("        a();\n            b();");
        expect(out.withGutter).toBe("a();\n  b();");
        // dedent-only, deliberately NOT narrowed — see formatMarkdownPreview
        expect(out.body).toBe("a();\n    b();");
    });

    it("handles an empty string", () => {
        expect(formatReadPreview("")).toEqual({ withGutter: "", body: "" });
    });
});

describe("formatReadPreview — real transcript sample", () => {
    // Lifted verbatim from a stored `Read` tool_result
    // (agentmux-srv/src/server/identity_handlers.rs), so this asserts against
    // the shape the CLI actually emits rather than an invented one. Note the
    // 1-digit and 3-digit line numbers in the same body — that mix is what
    // made the raw `<N>\t` gutter step sideways.
    const REAL =
        "1\t// Copyright 2026, AgentMux Corp.\n" +
        "2\t// SPDX-License-Identifier: Apache-2.0\n" +
        "3\t\n" +
        "4\t//! Pre-launch OAuth flow RPC handlers.\n" +
        "125\t        Box::new(move |data, _ctx| {\n" +
        "126\t            let mgr = mgr.clone();\n" +
        "127\t            let wstore = wstore.clone();\n" +
        "128\t            let broker = broker.clone();\n" +
        "129\t            Box::pin(async move {\n" +
        "130\t                let req: StartProviderAuthReq = serde_json::from_value(data)";

    /** Column the first source character lands on, expanding tabs the way the
     *  browser does at `tabSize` — i.e. what the eye actually sees, not the
     *  character offset. Everything up to and including the gutter separator
     *  is skipped. */
    const codeColumn = (line: string, tabSize = 2): number => {
        const gutter = /^\s*\d+[\t ]/.exec(line);
        if (!gutter) return 0;
        let col = 0;
        for (const ch of line.slice(0, gutter[0].length) + line.slice(gutter[0].length).match(/^[ \t]*/)![0]) {
            if (ch === "\t") col = (Math.floor(col / tabSize) + 1) * tabSize;
            else col += 1;
        }
        return col;
    };

    it("ends the gutter at the same column on every line, whatever the digit count", () => {
        // This is the regression the 08-24 tab-size change introduced: with the
        // raw `<N>\t`, a 1-digit number put the gutter at column 2 and a
        // 3-digit one at column 4, so the code's left edge stepped sideways.
        const rawGutterCols = new Set(
            REAL.split("\n")
                .filter((l) => /\S/.test(l.replace(/^\d+\t/, "")))
                .map((l) => codeColumn(l.replace(/^(\d+\t)[ \t]*/, "$1"))),
        );
        expect(rawGutterCols.size).toBeGreaterThan(1); // ragged before

        const fixedGutterCols = new Set(
            formatReadPreview(REAL)
                .withGutter.split("\n")
                .filter((l) => /^\s*\d+ \S/.test(l))
                .map((l) => /^\s*\d+ /.exec(l)![0].length),
        );
        expect([...fixedGutterCols]).toEqual([4]); // 3-digit gutter + one space
    });

    it("halves the deepest line's rendered indentation", () => {
        const before = REAL.split("\n").at(-1)!;
        const after = formatReadPreview(REAL).withGutter.split("\n").at(-1)!;
        expect(codeColumn(before)).toBe(20); // "130" → tab stop 4, + 16 spaces
        expect(codeColumn(after)).toBe(12); // 3-digit gutter + space, + 8 spaces
    });

    it("keeps relative structure intact — the nesting levels all survive", () => {
        // Source depths are 0/8/12/16; GCD is 4, so each halves to 0/4/6/8.
        // Same number of distinct levels, same ordering, half the width.
        // Measured on the CODE half (`withGutter`), which is the one that gets
        // narrowed; `body` is dedent-only for the Markdown renderer's sake.
        const depths = formatReadPreview(REAL)
            .withGutter.split("\n")
            .map((l) => l.replace(/^\s*\d+ ?/, ""))
            // strip the gutter FIRST — a blank source line is gutter-only, and
            // filtering before stripping would count it as a depth-0 code line
            .filter((l) => l.trim() !== "")
            .map((l) => l.length - l.trimStart().length);
        expect(depths).toEqual([0, 0, 0, 4, 6, 6, 6, 6, 8]);

        const sourceDepths = REAL.split("\n")
            .map((l) => l.replace(/^\d+\t/, ""))
            .filter((l) => l.trim() !== "")
            .map((l) => l.length - l.trimStart().length);
        expect(sourceDepths).toEqual([0, 0, 0, 8, 12, 12, 12, 12, 16]);
        expect(new Set(depths).size).toBe(new Set(sourceDepths).size);
    });

    it("contains no tab characters at all — the gutter tab is gone", () => {
        expect(formatReadPreview(REAL).withGutter).not.toContain("\t");
    });
});

describe("formatMarkdownPreview — indentation is load-bearing in markdown", () => {
    // codex P2 on PR #2958. Width normalisation is a readability win for a
    // syntax-highlighted source preview and a correctness bug for a rendered one.

    it("does NOT rescale a four-space indented code block into prose", () => {
        const md = "# Title\n\n    const x = 1;\n    const y = 2;\n";
        // 4 leading spaces = an indented code block. formatCodePreview would
        // halve it to 2 and markdown would render it as an ordinary paragraph.
        expect(formatCodePreview(md)).toContain("  const x = 1;");
        expect(formatMarkdownPreview(md)).toContain("    const x = 1;");
    });

    it("does NOT re-nest nested lists", () => {
        const md = "- a\n    - b\n        - c\n";
        expect(formatMarkdownPreview(md)).toBe(md);
    });

    it("still dedents a uniformly-indented body (pre-existing behaviour)", () => {
        expect(formatMarkdownPreview("  # Title\n  text")).toBe("# Title\ntext");
    });

    it("formatReadPreview.body is markdown-safe while withGutter is normalised", () => {
        const read = "1\t# Title\n2\t\n3\t    code block line\n";
        const out = formatReadPreview(read);
        expect(out.body).toContain("    code block line");
        expect(out.body).not.toMatch(/^\s*\d+ /m);
        // the source-preview half keeps the narrowing
        expect(out.withGutter).toContain("  code block line");
    });
});

describe("formatCodePreview", () => {
    it("dedents then narrows", () => {
        expect(formatCodePreview("    a();\n        b();\n            c();")).toBe("a();\n  b();\n    c();");
    });

    it("narrows even when dedent finds nothing to strip", () => {
        expect(formatCodePreview("a();\n    b();")).toBe("a();\n  b();");
    });
});

describe("stripCommonIndentSharedPrefix", () => {
    it("dedents both sides by ONE shared prefix, preserving add/del alignment", () => {
        const oldStr = "        const x = 1;\n        return x;";
        const newStr = "        const x = 2;\n        return x;";
        const { oldStr: o, newStr: n } = stripCommonIndentSharedPrefix(oldStr, newStr);
        expect(o).toBe("const x = 1;\nreturn x;");
        expect(n).toBe("const x = 2;\nreturn x;");
    });

    it("uses the MINIMUM shared indent across both sides when they differ", () => {
        // new_string adds a shallower wrapper line — the shared prefix must
        // be the min of the two sides' own minimums, not either side alone,
        // so relative indentation between old and new lines up correctly.
        const oldStr = "        inner();";
        const newStr = "      if (cond) {\n        inner();\n      }";
        const { oldStr: o, newStr: n } = stripCommonIndentSharedPrefix(oldStr, newStr);
        expect(o).toBe("  inner();");
        expect(n).toBe("if (cond) {\n  inner();\n}");
    });

    it("is a no-op when there is no shared prefix across both sides", () => {
        const oldStr = "\tfoo();";
        const newStr = "    bar();";
        const result = stripCommonIndentSharedPrefix(oldStr, newStr);
        expect(result.oldStr).toBe(oldStr);
        expect(result.newStr).toBe(newStr);
    });

    it("handles empty old_string (pure insertion)", () => {
        const { oldStr, newStr } = stripCommonIndentSharedPrefix("", "    added();");
        expect(oldStr).toBe("");
        expect(newStr).toBe("added();");
    });

    it("handles empty new_string (pure deletion)", () => {
        const { oldStr, newStr } = stripCommonIndentSharedPrefix("    removed();", "");
        expect(oldStr).toBe("removed();");
        expect(newStr).toBe("");
    });

    it("handles both sides empty", () => {
        const result = stripCommonIndentSharedPrefix("", "");
        expect(result).toEqual({ oldStr: "", newStr: "" });
    });
});
