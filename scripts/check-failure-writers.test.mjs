// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for check-failure-writers.mjs.

import { describe, expect, it } from "vitest";
import { checkFile, collect } from "./check-failure-writers.mjs";

const dispatch = 'model.dispatchPane({ type: "FailureObserved", failure: f, at: 1 });\n';

describe("check-failure-writers", () => {
    it("flags a direct raise and a direct clear, with their lines", () => {
        const text = `const a = 1;\n${dispatch}opts.model.dispatchPane({ type: 'FailureCleared' });\n`;
        expect(checkFile("frontend/app/view/agent/hooks/x.ts", text).map((v) => v.line)).toEqual([2, 3]);
    });

    it("leaves the writer, the store and tests alone", () => {
        expect(checkFile("frontend/app/view/agent/failure/failure-writer.ts", dispatch)).toEqual([]);
        expect(checkFile("frontend/app/store/agent-pane-state/reducer.ts", dispatch)).toEqual([]);
        expect(checkFile("frontend/app/view/agent/hooks/x.test.ts", dispatch)).toEqual([]);
    });

    it("does not count a mention of the command in a comment or an event name", () => {
        expect(checkFile("frontend/app/view/agent/x.ts", "// FailureObserved ends a turn\nconst e = 'failure-observed';\n")).toEqual([]);
    });

    it("finds none in the tree today", () => {
        expect(collect()).toEqual([]);
    });
});
