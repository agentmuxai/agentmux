// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { explainLinks, linkify } from "./linkify";
import { joined, linksOf, testContext } from "./test-context";

const links = (text: string, over = {}) => linksOf(linkify(text, testContext(over)));
const REMOTE = { author: { kind: "agent" as const, id: "pane-1", confined: false, remote: true } };

describe("files", () => {
    it("links a name with a known extension, a relative path and an absolute path", () => {
        expect(links("the audit table (findings.md) and docs/specs/X.md")).toEqual(["file:findings.md", "file:docs/specs/X.md"]);
        expect(links("see C:/work/notes/plan.md, C:\\work\\a.ts and /home/u/src/main.rs")).toEqual(["file:C:/work/notes/plan.md", "file:C:\\work\\a.ts", "file:/home/u/src/main.rs"]);
        expect(links("in ./src/a.tsx and ../x/y.json")).toEqual(["file:./src/a.tsx", "file:../x/y.json"]);
    });
    it("reads a bare name of every ending the web linker would also take for a country as a file", () => {
        expect(links("run tool.pl then parse.ml, parser.rs, pkg.sh and analyze.py", { author: { kind: "agent", id: "pane-1", confined: false } })).toEqual([
            "file:tool.pl",
            "file:parse.ml",
            "file:parser.rs",
            "file:pkg.sh",
            "file:analyze.py",
        ]);
    });
    it("takes an optional line and column into the words, and not into the path", () => {
        const link = linkify("fails at src/a.rs:120:4.", testContext()).find((s) => s.type === "link");
        expect(link).toMatchObject({ label: "src/a.rs:120:4", destination: { kind: "file", path: "src/a.rs" } });
    });
    it("builds the destination from the pane that shows the text, never from the text", () => {
        const link = linkify("open findings.md", testContext({ author: { kind: "agent", id: "pane-7", confined: true } })).find((s) => s.type === "link");
        expect(link).toMatchObject({ kind: "file", state: "unchecked", name: "Open file findings.md", destination: { kind: "file", path: "findings.md", blockId: "pane-7", confined: true } });
    });
    it("makes no file link for text no pane wrote, the operator's, or another computer's", () => {
        const files = (text: string, over = {}) => links(text, over).filter((l) => l.startsWith("file:"));
        expect(files("findings.md and docs/a.md", { author: { kind: "app" } })).toEqual([]);
        expect(files("findings.md and docs/a.md", { author: { kind: "operator" } })).toEqual([]);
        expect(files("findings.md and docs/a.md", REMOTE)).toEqual([]);
    });
    it("leaves a version, a time, a bare domain, an address and an unknown extension alone", () => {
        expect(links("v1.2.3 at 10:30 and 3.5.1")).toEqual([]);
        expect(links("see https://example.com/docs/a.md and www.example.org/x/y.md")).toEqual(["address:https://example.com/docs/a.md", "address:www.example.org/x/y.md"]);
        expect(links("see .github/workflows/ci.yml")).toEqual(["file:.github/workflows/ci.yml"]);
        expect(links("a thing.unknownext and word.")).toEqual([]);
    });
    it("does not link the middle of a longer word or a path segment of an email", () => {
        expect(links("opaz@findings.md and xfindings.md.bak")).toEqual([]);
    });
    it("is the same text back, byte for byte", () => {
        const text = "Done: findings.md, docs/x.md:3 and C:/a/b.ts.";
        expect(joined(linkify(text, testContext()))).toBe(text);
    });
    it("explains a file it saw in code and left alone", () => {
        const trace = explainLinks("run `cat findings.md`", testContext());
        expect(trace.some((t) => t.kind === "file" && t.outcome.startsWith("protected"))).toBe(true);
    });
});
