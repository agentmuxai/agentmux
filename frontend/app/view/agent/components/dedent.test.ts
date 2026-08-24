// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { stripCommonIndent, stripCommonIndentNumbered, stripCommonIndentSharedPrefix } from "./dedent";
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

describe("stripCommonIndentNumbered", () => {
    it("preserves the N\\t prefix and dedents only the code portion", () => {
        const input = "80\t        unsetEnv: [\"CLAUDECODE\"],\n81\t            authConfigDirEnvVar: \"X\",";
        const out = stripCommonIndentNumbered(input);
        expect(out).toBe('80\tunsetEnv: ["CLAUDECODE"],\n81\t    authConfigDirEnvVar: "X",');
    });

    it("falls through to plain dedent for non-uniformly-numbered input", () => {
        const input = "1\tfoo();\nnot numbered\n3\tbar();";
        // Not every non-blank line matches the numbered shape, so this
        // falls through to stripCommonIndent on the raw text — which finds
        // no common leading whitespace here (lines start with digits/text),
        // so it's a no-op.
        expect(stripCommonIndentNumbered(input)).toBe(input);
    });

    it("falls through to plain dedent for completely unnumbered input", () => {
        const input = "    a();\n    b();";
        expect(stripCommonIndentNumbered(input)).toBe("a();\nb();");
    });

    it("is a no-op when the numbered code portions have no common indent", () => {
        const input = "1\ta();\n2\t    b();";
        expect(stripCommonIndentNumbered(input)).toBe(input);
    });

    it("handles blank lines within numbered content", () => {
        const input = "1\t    a();\n2\t\n3\t    b();";
        const out = stripCommonIndentNumbered(input);
        expect(out).toBe("1\ta();\n2\t\n3\tb();");
    });

    it("handles an empty string", () => {
        expect(stripCommonIndentNumbered("")).toBe("");
    });

    it("handles a single numbered line", () => {
        expect(stripCommonIndentNumbered("1\t    solo();")).toBe("1\tsolo();");
    });

    it("tolerates multi-digit line numbers and varying digit widths", () => {
        const input = "9\t    a();\n10\t    b();\n100\t    c();";
        const out = stripCommonIndentNumbered(input);
        expect(out).toBe("9\ta();\n10\tb();\n100\tc();");
    });

    it("ignores a trailing truncation marker so the numbered-shape check still passes and the code still dedents", () => {
        const input = `80\t    a();\n81\t    b();\n${TRUNCATED_MARKER}`;
        const out = stripCommonIndentNumbered(input);
        expect(out).toBe(`80\ta();\n81\tb();\n${TRUNCATED_MARKER}`);
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
