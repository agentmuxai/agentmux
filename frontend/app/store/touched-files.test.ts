// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it } from "vitest";
import { childKey, noteToolCall, noteToolResult, resetTouchedForTests, TOUCH_TTL_MS, touched, touchesUnder } from "./touched-files";

afterEach(resetTouchedForTests);

const agent = { blockId: "b1", agentName: "Korp", color: "#ff8800" };

describe("touched files (Files pane §8.4)", () => {
    it("records a file once its write succeeds, not before", () => {
        noteToolCall(agent, { id: "t1", tool: "Write", params: { file_path: "C:\\repo\\src\\a.ts" } });
        expect(touched().size).toBe(0);
        noteToolResult("t1", "success", 1000);
        expect([...touched().values()]).toEqual([{ blockId: "b1", agentName: "Korp", color: "#ff8800", tool: "Write", at: 1000 }]);
    });

    it("ignores failed, denied and unrelated calls, and relative paths", () => {
        noteToolCall(agent, { id: "t1", tool: "Edit", params: { file_path: "C:\\repo\\a.ts" } });
        noteToolResult("t1", "failed");
        noteToolCall(agent, { id: "t2", tool: "Bash", params: { command: "rm -rf x" } });
        noteToolResult("t2", "success");
        noteToolCall(agent, { id: "t3", tool: "Write", params: { file_path: "a.ts" } });
        noteToolResult("t3", "success");
        noteToolResult("never-called", "success");
        expect(touched().size).toBe(0);
    });

    it("knows NotebookEdit's parameter", () => {
        noteToolCall(agent, { id: "t1", tool: "NotebookEdit", params: { notebook_path: "/home/a/n.ipynb" } });
        noteToolResult("t1", "success", 5);
        expect(touchesUnder("/home/a", 5).get("n.ipynb")?.tool).toBe("NotebookEdit");
    });

    it("gives each direct child of a folder its latest change, folders included", () => {
        noteToolCall(agent, { id: "a", tool: "Write", params: { file_path: "C:\\repo\\src\\deep\\x.ts" } });
        noteToolResult("a", "success", 100);
        noteToolCall({ ...agent, agentName: "Lark" }, { id: "b", tool: "Edit", params: { file_path: "C:\\repo\\src\\y.ts" } });
        noteToolResult("b", "success", 200);
        noteToolCall(agent, { id: "c", tool: "Edit", params: { file_path: "C:\\repo\\README.md" } });
        noteToolResult("c", "success", 150);
        const under = touchesUnder("C:\\Repo", 300);
        expect(under.get(childKey("C:\\Repo", "src"))?.agentName).toBe("Lark");
        expect(under.get(childKey("C:\\Repo", "README.md"))?.agentName).toBe("Korp");
        expect(touchesUnder("C:\\repo\\src", 300).get("deep")?.at).toBe(100);
    });

    it("forgets a change after half an hour", () => {
        noteToolCall(agent, { id: "a", tool: "Write", params: { file_path: "/home/a/x" } });
        noteToolResult("a", "success", 0);
        expect(touchesUnder("/home/a", TOUCH_TTL_MS).size).toBe(1);
        expect(touchesUnder("/home/a", TOUCH_TTL_MS + 1).size).toBe(0);
    });

    it("keeps case on POSIX paths and ignores it on Windows ones", () => {
        expect(childKey("/home/a", "README.md")).toBe("README.md");
        expect(childKey("C:\\Users\\a", "README.md")).toBe("readme.md");
    });
});
