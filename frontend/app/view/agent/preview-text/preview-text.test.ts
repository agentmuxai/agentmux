// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The preview text stage (REPORT_TOOL_PREVIEW_TEXT_PIPELINE_2026_10_08.md
 * §5.1), with fixtures shaped like real tool output.
 */

import { describe, expect, it } from "vitest";
import { capLines } from "./cap";
import {
    chunksDoc,
    codeDoc,
    commandDoc,
    createChunkWindow,
    diffDocFromSides,
    diffDocFromUnified,
    jsonDoc,
    markdownBodyText,
    outputDoc,
    proseDoc,
    rawDoc,
    splitGutter,
} from "./docs";
import { CODE_TAB_WIDTH, expandTabs, OUTPUT_TAB_WIDTH } from "./tabs";
import { decodeTerminal } from "./terminal";

const texts = (lines: { text: string }[]) => lines.map((l) => l.text);
const ESC = "\x1b";

describe("expandTabs", () => {
    it("moves to the next stop, counted from the start of the line", () => {
        expect(expandTabs("\tx", 2)).toBe("  x");
        expect(expandTabs("a\tb", 2)).toBe("a b");
        expect(expandTabs("ab\tc", 2)).toBe("ab  c");
        expect(expandTabs("abc1234\tfix", 8)).toBe("abc1234 fix");
        expect(expandTabs("abc12345\tfix", 8)).toBe("abc12345        fix");
    });
    it("leaves a line without tabs alone", () => {
        const s = "no tabs here";
        expect(expandTabs(s, 2)).toBe(s);
    });
});

describe("decodeTerminal", () => {
    it("splits plain text as is", () => {
        expect(decodeTerminal("a\nb", 8)).toEqual([{ text: "a" }, { text: "b" }]);
    });

    it("drops bashwrap's echoed cursor report at the start (results recorded before #4508)", () => {
        expect(texts(decodeTerminal("^[[1;1Rhello\nworld", 8))).toEqual(["hello", "world"]);
        // Only at the very start: the same text inside output is content.
        expect(texts(decodeTerminal("x ^[[1;1R", 8))).toEqual(["x ^[[1;1R"]);
    });

    it("keeps colour across lines, as a terminal does", () => {
        const [a, b, c] = decodeTerminal(`${ESC}[31mred\nstill red${ESC}[0m\nplain`, 8);
        expect(a.spans).toEqual([{ text: "red", classes: "text-ansi-red" }]);
        expect(b.spans).toEqual([{ text: "still red", classes: "text-ansi-red" }]);
        expect(c.spans).toBeUndefined();
        expect(texts([a, b, c])).toEqual(["red", "still red", "plain"]);
    });

    it("applies a carriage return like a terminal: the last redraw wins", () => {
        expect(texts(decodeTerminal("10%\r55%\r100%\ndone", 8))).toEqual(["100%", "done"]);
        expect(texts(decodeTerminal("abcdef\rXY", 8))).toEqual(["XYcdef"]);
    });

    it("treats CRLF as one line break", () => {
        expect(texts(decodeTerminal("a\r\nb\r\n", 8))).toEqual(["a", "b", ""]);
    });

    it("applies backspace, erase-to-end-of-line and go-to-column", () => {
        expect(texts(decodeTerminal("abc\bd", 8))).toEqual(["abd"]);
        expect(texts(decodeTerminal(`progress 50%\r${ESC}[Kdone`, 8))).toEqual(["done"]);
        expect(texts(decodeTerminal(`xxxx${ESC}[1Gab`, 8))).toEqual(["abxx"]);
    });

    it("drops other escape sequences and control characters", () => {
        expect(texts(decodeTerminal(`${ESC}]0;title\x07${ESC}[?25lok\x07${ESC}[?25h`, 8))).toEqual(["ok"]);
        expect(texts(decodeTerminal(`${ESC}]8;;https://x${ESC}\\link${ESC}]8;;${ESC}\\`, 8))).toEqual(["link"]);
    });

    it("expands tabs against the terminal column", () => {
        expect(texts(decodeTerminal("a\tb\nab\tc", OUTPUT_TAB_WIDTH))).toEqual(["a       b", "ab      c"]);
        expect(texts(decodeTerminal(`${ESC}[1mab${ESC}[0m\tc`, OUTPUT_TAB_WIDTH))).toEqual(["ab      c"]);
    });
});

describe("Read gutter", () => {
    it("splits the CLI's numbered lines, in every format it uses", () => {
        expect(splitGutter(["     1\tconst a = 1;", "     2\t", "    10\tb"])).toEqual({
            numbers: [1, 2, 10],
            code: ["const a = 1;", "", "b"],
        });
        expect(splitGutter(["9\tx", "10\ty"])).toEqual({ numbers: [9, 10], code: ["x", "y"] });
        expect(splitGutter(["  1→x", "  2→y"])).toEqual({ numbers: [1, 2], code: ["x", "y"] });
    });
    it("splits a line holding a lone \\r or U+2028 too", () => {
        expect(splitGutter(["  1\ta\rb", "  2\tc\u2028d"])).toEqual({ numbers: [1, 2], code: ["a\rb", "c\u2028d"] });
    });
    it("declines a body that isn't numbered", () => {
        expect(splitGutter(["const a = 1;", "     2\tx"])).toBeNull();
        expect(splitGutter(["", ""])).toBeNull();
    });
});

describe("codeDoc", () => {
    const goLines = [
        "package main",
        "",
        "func main() {",
        "\tfor i := 0; i < 3; i++ {",
        "\t\tif i%2 == 0 {",
        "\t\t\tfmt.Println(i)",
        "\t\t}",
        "\t}",
        "}",
    ];
    const asRead = (lines: string[], start = 1) =>
        lines.map((l, i) => `${String(start + i).padStart(6)}\t${l}`).join("\n");

    it("indents a tab-indented file 2 columns a level, wherever the gutter ends", () => {
        for (const start of [1, 95, 995]) {
            const doc = codeDoc(asRead(goLines, start), { path: "/x/main.go", gutter: true });
            expect(doc.lines.map((l) => l.number)).toEqual(goLines.map((_, i) => start + i));
            expect(texts(doc.lines)).toEqual([
                "package main",
                "",
                "func main() {",
                "  for i := 0; i < 3; i++ {",
                "    if i%2 == 0 {",
                "      fmt.Println(i)",
                "    }",
                "  }",
                "}",
            ]);
            expect(doc.lines.some((l) => l.text.includes("\t"))).toBe(false);
        }
    });

    it("narrows a space-indented file to 2 columns a level", () => {
        const doc = codeDoc(asRead(["def f():", "    if x:", "        return 1", "    return 2"]), {
            path: "a.py",
            gutter: true,
        });
        expect(texts(doc.lines)).toEqual(["def f():", "  if x:", "    return 1", "  return 2"]);
    });

    it("a file mixing tabs and spaces: tabs at 2, spaces as written (a tab's intended width is unknowable)", () => {
        const doc = codeDoc(asRead(["def f():", "\tif x:", "    return 2"]), { path: "a.py", gutter: true });
        expect(texts(doc.lines)).toEqual(["def f():", "  if x:", "    return 2"]);
    });

    it("dedents a slice read from the middle of a file", () => {
        const doc = codeDoc(asRead(["        a();", "            b();", "        c();"], 40), {
            path: "a.ts",
            gutter: true,
        });
        expect(texts(doc.lines)).toEqual(["a();", "  b();", "c();"]);
    });

    it("detects the language from the code, not the gutter, so a shebang is seen", () => {
        expect(codeDoc(asRead(["#!/usr/bin/env python3", "print(1)"]), { path: "/x/tool", gutter: true }).lang).toBe(
            "python"
        );
    });

    it("shows a body that isn't numbered as plain code", () => {
        const doc = codeDoc("a\n  b", { path: "a.ts", gutter: true });
        expect(doc.lines.every((l) => l.number === undefined)).toBe(true);
        expect(texts(doc.lines)).toEqual(["a", "  b"]);
    });

    it("caps at the head and says how many lines were left out", () => {
        const doc = codeDoc(Array.from({ length: 1005 }, (_, i) => `l${i}`).join("\n"), { path: "a.txt" });
        expect(doc.lines).toHaveLength(1000);
        expect(doc.hidden).toEqual({ count: 5, from: "head" });
    });

    // Lifted from a stored Read tool_result (crates/srv/src/server/identity_handlers.rs),
    // the shape the CLI emits: 1-digit and 3-digit line numbers in one body,
    // the mix that made the old inline `<N>\t` gutter step sideways.
    const REAL =
        "1\t// Copyright 2026, AgentMux Corp.\n" +
        "2\t// SPDX-License-Identifier: Apache-2.0\n" +
        "3\t\n" +
        "4\t//! Pre-launch OAuth flow RPC handlers.\n" +
        "125\t        Box::new(move |data, _ctx| {\n" +
        "126\t            let mgr = mgr.clone();\n" +
        "127\t            let mstore = mstore.clone();\n" +
        "128\t            let broker = broker.clone();\n" +
        "129\t            Box::pin(async move {\n" +
        "130\t                let req: StartProviderAuthReq = serde_json::from_value(data)";

    it("a real Read: numbers as data, every nesting level kept at half the width", () => {
        const doc = codeDoc(REAL, { path: "identity_handlers.rs", gutter: true });
        expect(doc.lines.map((l) => l.number)).toEqual([1, 2, 3, 4, 125, 126, 127, 128, 129, 130]);
        expect(doc.lang).toBe("rust");
        const depths = doc.lines
            .filter((l) => l.text.trim() !== "")
            .map((l) => l.text.length - l.text.trimStart().length);
        // Source depths 0/8/12/16 (unit 4) narrow to 0/4/6/8.
        expect(depths).toEqual([0, 0, 0, 4, 6, 6, 6, 6, 8]);
    });

    it("gives Markdown the body without the gutter, and doesn't narrow it (indentation is syntax there)", () => {
        expect(markdownBodyText("1\t# Title\n2\t\n3\t    code block line\n", { gutter: true })).toContain(
            "    code block line"
        );
    });

    it("gives Markdown the body without the gutter", () => {
        expect(markdownBodyText(asRead(["# Title", "", "text"]), { gutter: true })).toBe("# Title\n\ntext");
    });
});

describe("diffs", () => {
    it("builds markers as data, with tabs expanded from the code's own start", () => {
        const doc = diffDocFromSides("if (a) {\n\tx();\n}", "if (a) {\n\ty();\n}", "a.ts");
        expect(doc.lines.map((l) => [l.marker, l.text])).toEqual([
            ["@@", "@@ -1,3 +1,3 @@"],
            [" ", "if (a) {"],
            ["-", "  x();"],
            ["+", "  y();"],
            [" ", "}"],
        ]);
        expect(doc.lang).toBe("typescript");
    });

    it("dedents both sides by one shared prefix", () => {
        const doc = diffDocFromSides("        a();", "        if (x) {\n            a();\n        }", "a.ts");
        expect(doc.lines.slice(1).map((l) => [l.marker, l.text])).toEqual([
            ["-", "a();"],
            ["+", "if (x) {"],
            ["+", "  a();"],
            ["+", "}"],
        ]);
    });

    it("reads a unified diff the tool returned", () => {
        const doc = diffDocFromUnified("@@ -1,2 +1,2 @@\n ctx\n-\told\n+\tnew\n", "a.go");
        expect(doc.lines.map((l) => [l.marker, l.text])).toEqual([
            ["@@", "@@ -1,2 +1,2 @@"],
            [" ", "ctx"],
            ["-", "  old"],
            ["+", "  new"],
        ]);
    });
});

describe("output", () => {
    it("a final newline doesn't add an empty line", () => {
        expect(texts(outputDoc("a\nb\n", { from: "tail" }).lines)).toEqual(["a", "b"]);
    });

    it("keeps the tail of a command's output", () => {
        const doc = outputDoc(Array.from({ length: 1003 }, (_, i) => `l${i}`).join("\n"), { from: "tail" });
        expect(doc.lines[0].text).toBe("l3");
        expect(doc.hidden).toEqual({ count: 3, from: "tail" });
    });

    it("puts stderr after stdout, marked", () => {
        const doc = commandDoc("^[[1;1Rout\n", "err\n");
        expect(doc.lines).toEqual([{ text: "out" }, { text: "err", stream: "stderr" }]);
    });

    it("an empty command shows nothing", () => {
        expect(commandDoc("", "").lines).toEqual([]);
    });

    it("streamed chunks join into the same lines as the finished output", () => {
        const text = "first line\nsecond\tcol\nthird";
        const chunks = [
            { kind: "stdout", content: "first li" },
            { kind: "stdout", content: "ne\nsec" },
            { kind: "stdout", content: "ond\tcol\nthird" },
        ];
        expect(chunksDoc(chunks).lines).toEqual(outputDoc(text, { from: "tail" }).lines);
        expect(
            chunksDoc([
                { kind: "stdout", content: "a\n" },
                { kind: "stdout", content: "b\n" },
            ]).lines
        ).toEqual([{ text: "a" }, { text: "b" }]);
    });

    it("an empty chunk adds nothing", () => {
        expect(
            chunksDoc([
                { kind: "stdout", content: "a\n" },
                { kind: "stdout", content: "" },
            ]).lines
        ).toEqual([{ text: "a" }]);
    });

    it("streamed colour carries from one line to the next, as in the finished output", () => {
        const chunks = [
            { kind: "stdout", content: `${ESC}[31mred\n` },
            { kind: "stdout", content: `still red${ESC}[0m\nplain\n` },
        ];
        const text = chunks.map((c) => c.content).join("");
        expect(chunksDoc(chunks).lines).toEqual(outputDoc(text, { from: "tail" }).lines);
        expect(chunksDoc(chunks).lines[1].spans).toEqual([{ text: "still red", classes: "text-ansi-red" }]);
    });

    it("a chunk from another stream starts a new line instead of joining an open one", () => {
        const chunks = [
            { kind: "stdout", content: "out" },
            { kind: "stderr", content: "err" },
        ];
        expect(chunksDoc(chunks).lines).toEqual(commandDoc("out", "err").lines);
        expect(createChunkWindow(10)(chunks, () => false).total).toBe(2);
    });

    it("a streamed line keeps the stream of the chunk that started it", () => {
        expect(
            chunksDoc([
                { kind: "stderr", content: "boom\n" },
                { kind: "stdout", content: "ok" },
            ]).lines
        ).toEqual([{ text: "boom", stream: "stderr" }, { text: "ok" }]);
    });
});

describe("createChunkWindow", () => {
    const c = (content: string) => ({ kind: "stdout", content });
    const notWhole = () => false;

    it("counts joined lines incrementally, the same as counting from scratch", () => {
        const stream = [c("a"), c("b\nc"), c("\n"), c("d\n"), c("e")];
        const w = createChunkWindow(10);
        expect(w(stream.slice(0, 2), notWhole).total).toBe(chunksDoc(stream.slice(0, 2)).lines.length);
        expect(w(stream, notWhole).total).toBe(chunksDoc(stream).lines.length);
        expect(createChunkWindow(10)(stream, notWhole).total).toBe(4);
    });

    it("starts over for a new stream", () => {
        const w = createChunkWindow(10);
        w([c("a\n"), c("b\n")], notWhole);
        expect(w([c("x\n")], notWhole).total).toBe(1);
    });

    it("counts a whole-line chunk as its own line", () => {
        const frame = c("50%");
        expect(createChunkWindow(10)([c("building"), frame, c("done")], (x) => x === frame).total).toBe(3);
    });

    it("cuts an oversized first chunk to its last lines, keeping it whole if it was", () => {
        const big = c(Array.from({ length: 50 }, (_, i) => `l${i}`).join("\n") + "\n");
        const { chunks, whole } = createChunkWindow(5)([big], (x) => x === big);
        expect(chunks[0]).not.toBe(big);
        expect(chunks[0].content.split("\n").filter(Boolean)).toEqual(["l44", "l45", "l46", "l47", "l48", "l49"]);
        expect(whole.has(chunks[0])).toBe(true);
    });
});

describe("rawDoc", () => {
    it("shows the text as received: control characters visible, nothing applied or dropped", () => {
        const raw = `[JEKT:FROM=x]\nvisible\rhidden${ESC}[2K\b${ESC}]0;t\x07\ttab`;
        expect(texts(rawDoc(raw).lines)).toEqual([
            "[JEKT:FROM=x]",
            "visible\u240dhidden\u241b[2K\u2408\u241b]0;t\u2407       tab",
        ]);
    });
});

describe("json and prose", () => {
    it("pretty-prints a structured result", () => {
        expect(texts(jsonDoc({ a: [1] }).lines)).toEqual(["{", '  "a": [', "    1", "  ]", "}"]);
    });
    it("prose keeps its text and expands tabs", () => {
        expect(texts(proseDoc("hello\tworld\nline two").lines)).toEqual(["hello   world", "line two"]);
    });
});

describe("the character cap", () => {
    const huge = "x".repeat(1_000_100);

    it("says when it cut text, and which end it kept", () => {
        expect(outputDoc(huge, { from: "tail" }).truncated).toBe("tail");
        expect(codeDoc(huge, { path: "a.min.js" }).truncated).toBe("head");
        expect(jsonDoc({ blob: huge }).truncated).toBe("head");
        expect(commandDoc(huge, "").truncated).toBe("tail");
        expect(proseDoc(huge).truncated).toBe("head");
        expect(rawDoc(huge).truncated).toBe("head");
    });

    it("says nothing when nothing was cut", () => {
        expect(outputDoc("short", { from: "tail" }).truncated).toBeUndefined();
        expect(codeDoc("short", { path: "a.ts" }).truncated).toBeUndefined();
    });
});

describe("capLines", () => {
    it("leaves a short doc alone", () => {
        const doc = { kind: "output" as const, lines: [{ text: "a" }] };
        expect(capLines(doc, "head", 5)).toBe(doc);
    });
});

it("code and output tab widths are what the report decided", () => {
    expect(CODE_TAB_WIDTH).toBe(2);
    expect(OUTPUT_TAB_WIDTH).toBe(8);
});

describe("Codex's second pass on #4512", () => {
    const huge = "x".repeat(1_000_100);

    it("a diff built from the edit's strings is capped by characters too, and says so", () => {
        const doc = diffDocFromSides("a", huge, "a.ts");
        expect(doc.lines.reduce((n, l) => n + l.text.length, 0)).toBeLessThanOrEqual(1_000_100);
        expect(doc.truncated).toBe("head");
    });

    it("selective SGR resets end what they reset, even when carried across lines", () => {
        const [, second] = decodeTerminal(`${ESC}[31mred\n${ESC}[39mplain`, 8);
        expect(second).toEqual({ text: "plain" });
        const [, bold] = decodeTerminal(`${ESC}[1mbold\n${ESC}[22mnormal`, 8);
        expect(bold).toEqual({ text: "normal" });
    });

    it("an extended colour's parameters aren't read as other codes (2 isn't faint)", () => {
        const [line] = decodeTerminal(`${ESC}[38;2;255;10;10mtruecolor${ESC}[0m`, 8);
        expect(line.spans?.[0].classes ?? "").not.toContain("opacity-75");
    });

    it("colour doesn't carry from one stream into the next", () => {
        const doc = chunksDoc([
            { kind: "stdout", content: `${ESC}[32mgreen\n` },
            { kind: "stderr", content: "error line\n" },
        ]);
        expect(doc.lines[1]).toEqual({ text: "error line", stream: "stderr" });
    });

    it("the window counts a stream change as a line end, so alternating chunks stay bounded", () => {
        const stream = Array.from({ length: 4000 }, (_, i) => ({
            kind: i % 2 ? "stderr" : "stdout",
            content: `c${i}`,
        }));
        const { chunks, total } = createChunkWindow(100)(stream, () => false);
        expect(total).toBe(4000);
        expect(chunks.length).toBeLessThan(200);
    });

    it("Markdown text is capped without a marker line in it", () => {
        expect(markdownBodyText(`# T\n${huge}`, { gutter: false })).not.toContain("…(truncated)");
    });
});
