// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { recentActivityEntries } from "./ambient-activity";
import type { DocumentNode } from "./types";

const user = (message: string, isStartup = false) =>
    ({ type: "user_message", id: `u-${message}`, message, isStartup }) as unknown as DocumentNode;
const said = (content: string, thinking = false) =>
    ({ type: "markdown", id: `m-${content}`, content, metadata: { thinking } }) as unknown as DocumentNode;
const tool = (toolName: string, status = "success") =>
    ({ type: "tool", id: `t-${toolName}`, tool: "Other", toolName, status, params: {} }) as unknown as DocumentNode;
const jekt = (message: string, direction: "incoming" | "outgoing") =>
    ({ type: "jekt_message", id: `j-${message}`, message, direction }) as unknown as DocumentNode;

describe("recentActivityEntries", () => {
    it("renders the conversation in the digest's form, oldest first", () => {
        const nodes = [
            user("fix the build"),
            said("Let me look.", true),
            tool("shell"),
            said("Fixed the import path."),
        ];
        expect(recentActivityEntries(nodes)).toEqual([
            "[user] fix the build",
            "[tool] shell",
            "[assistant] Fixed the import path.",
        ]);
    });

    it("leaves out the startup payload, outgoing jekts and empty messages", () => {
        const nodes = [user("# Session Context ...", true), jekt("to someone", "outgoing"), said("   "), jekt("review done", "incoming")];
        expect(recentActivityEntries(nodes)).toEqual(["[user] review done"]);
    });

    it("follows a failed tool with an error entry", () => {
        expect(recentActivityEntries([tool("Bash", "failed")])).toEqual(["[tool] Bash", "[error] Bash failed"]);
    });

    it("keeps the newest 40 entries and the end of a long one", () => {
        const nodes = Array.from({ length: 60 }, (_, i) => user(`msg ${i}`));
        const entries = recentActivityEntries(nodes);
        expect(entries).toHaveLength(40);
        expect(entries[0]).toBe("[user] msg 20");
        expect(entries[39]).toBe("[user] msg 59");

        const [long] = recentActivityEntries([said(`${"x".repeat(5000)} Should I ship it?`)]);
        expect(long.length).toBe(1000);
        expect(long.startsWith("[assistant] …")).toBe(true);
        expect(long.endsWith("Should I ship it?")).toBe(true);
    });
});
