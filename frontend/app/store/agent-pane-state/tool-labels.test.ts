// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { foldActivities, toolActivity } from "@/app/store/agent-pane-state/tool-labels";

describe("toolActivity", () => {
    it("uses the description the model wrote for a command, in progressive form", () => {
        expect(toolActivity("Bash", { command: "cargo test -p srv", description: "Run the srv test suite" }).label).toBe(
            "Running the srv test suite",
        );
        expect(toolActivity("Bash", { description: "Check the CI status" }).label).toBe("Checking the CI status");
        expect(toolActivity("Bash", { description: "Write the changeset" }).label).toBe("Writing the changeset");
        expect(toolActivity("Bash", { description: "Building the app" }).label).toBe("Building the app");
        expect(toolActivity("Bash", { description: "List files" }).label).toBe("Listing files");
    });

    it("falls back to the command's program, never the whole command", () => {
        expect(toolActivity("Bash", { command: "npm run build -- --watch" }).label).toBe("Running npm");
        expect(toolActivity("Bash", {}).label).toBe("Running a command");
    });

    it("names a subagent by its kind and task", () => {
        expect(toolActivity("Agent", { subagent_type: "Explore", description: "find turn-origin code" }).label).toBe(
            "Explore agent: find turn-origin code",
        );
        expect(toolActivity("Task", { subagent_type: "general-purpose", description: "audit" }).label).toBe("Subagent: audit");
    });

    it("reads, edits and searches by file name or pattern", () => {
        expect(toolActivity("Read", { file_path: "C:\\x\\frontend\\AgentFooter.tsx" })).toEqual({ family: "read", label: "Reading AgentFooter.tsx" });
        expect(toolActivity("Edit", { file_path: "/repo/health.rs" }).label).toBe("Editing health.rs");
        expect(toolActivity("Write", { file_path: "/repo/new.ts" }).label).toBe("Writing new.ts");
        expect(toolActivity("Grep", { pattern: "turn_active" }).label).toBe("Searching for turn_active");
        expect(toolActivity("WebFetch", { url: "https://docs.anthropic.com/x" }).label).toBe("Reading docs.anthropic.com");
        expect(toolActivity("WebSearch", { query: "claude code stream-json" }).label).toBe("Searching the web: claude code stream-json");
    });

    it("an MCP tool by its own short name", () => {
        expect(toolActivity("mcp__agentmux__SendMessage", {}).label).toBe("Using SendMessage");
    });

    it("keeps a long description to one short line", () => {
        const label = toolActivity("Bash", { description: "Run " + "x".repeat(200) }).label;
        expect(label.length).toBe(80);
        expect(label.endsWith("…")).toBe(true);
    });
});

describe("foldActivities", () => {
    const read = (f: string) => toolActivity("Read", { file_path: f });
    it("one call is its own label; one kind is counted; a mix is counted", () => {
        expect(foldActivities([])).toBeNull();
        expect(foldActivities([read("a.ts")])).toBe("Reading a.ts");
        expect(foldActivities([read("a.ts"), read("b.ts"), read("c.ts")])).toBe("Reading 3 files");
        expect(foldActivities([read("a.ts"), toolActivity("Grep", { pattern: "x" })])).toBe("2 tools running");
    });

    it("a subagent among others is the one named", () => {
        const agent = toolActivity("Agent", { subagent_type: "Explore", description: "map the code" });
        expect(foldActivities([read("a.ts"), agent])).toBe("Explore agent: map the code (+1)");
    });
});
