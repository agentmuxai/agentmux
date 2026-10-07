// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { EditorState } from "@codemirror/state";
import { describe, expect, it } from "vitest";
import { cursorAtLine, readOpenAtLine } from "./open-at-line";

describe("readOpenAtLine", () => {
    it("reads a positive whole line for a file", () => {
        expect(readOpenAtLine({ "editor:line": 12 }, "/home/u/.ssh/config")).toBe(12);
    });

    it("is no request without a file, or with a line that isn't a positive whole number", () => {
        expect(readOpenAtLine({ "editor:line": 12 }, undefined)).toBeNull();
        expect(readOpenAtLine({ "editor:line": 0 }, "/f")).toBeNull();
        expect(readOpenAtLine({ "editor:line": 2.5 }, "/f")).toBeNull();
        expect(readOpenAtLine({ "editor:line": "12" }, "/f")).toBeNull();
        expect(readOpenAtLine({}, "/f")).toBeNull();
    });
});

describe("cursorAtLine", () => {
    const state = EditorState.create({ doc: "Host *\n  User u\n\nHost db1\n  HostName db1.internal" });
    const headAfter = (line: number) => state.update(cursorAtLine(state, line)).state.selection.main.head;

    it("puts the cursor at the start of the line", () => {
        expect(headAfter(4)).toBe(state.doc.line(4).from);
    });

    it("clamps a line past the end to the last line", () => {
        expect(headAfter(99)).toBe(state.doc.line(5).from);
    });
});
