// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for check-transcript-scroll-writers.mjs.

import { describe, expect, it } from "vitest";
import { EXEMPT, checkFile, collect } from "./check-transcript-scroll-writers.mjs";

const AGENT = "frontend/app/view/agent/hooks/x.ts";

describe("check-transcript-scroll-writers", () => {
    it("flags each kind of scroll write, with its line", () => {
        const text = [
            "const a = 1;",
            "el.scrollTop = 5;",
            "el.scrollTop += d;",
            "el.scrollTop-=1;",
            "el.scrollTo({ top: 1 });",
            "el.scrollBy(0, 4);",
            "row.scrollIntoView({ block: 'nearest' });",
        ].join("\n");
        expect(checkFile(AGENT, text).map((v) => v.line)).toEqual([2, 3, 4, 5, 6, 7]);
    });

    it("does not count reads, comparisons, comments or the exported functions", () => {
        const text = [
            "const t = el.scrollTop;",
            "if (el.scrollTop === 0) return;",
            "if (el.scrollTop == 0) return;",
            "// el.scrollTop = 5 once pinned",
            " * scrollTo( the node",
            "scrollTranscriptTo(el, 5);",
            "scrollTranscriptBy(el, 5);",
        ].join("\n");
        expect(checkFile(AGENT, text)).toEqual([]);
    });

    it("leaves the writer, the exempt files, tests and other directories alone", () => {
        const write = "el.scrollTop = 5;\n";
        expect(checkFile("frontend/app/view/agent/scroll/one-way-flow.ts", write)).toEqual([]);
        for (const path of Object.keys(EXEMPT)) expect(checkFile(path, write)).toEqual([]);
        expect(checkFile("frontend/app/view/agent/hooks/x.test.ts", write)).toEqual([]);
        expect(checkFile("frontend/app/store/agent-pane-layout-store.bench.ts", write)).toEqual([]);
        expect(checkFile("frontend/app/element/install/LogView.tsx", write)).toEqual([]);
    });

    it("finds none in the tree today", () => {
        expect(collect()).toEqual([]);
    });
});
