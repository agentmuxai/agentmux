// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    formatMarkdownPreview,
    formatCodePreview,
    normalizeIndentWidth,
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

    it("is a no-op on tab-indented text (preview-text/ expands tabs before it runs)", () => {
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
