// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { createSessionStartDetector } from "./session-start";

const init = { type: "system", subtype: "init", session_id: "s" };
const boundary = { type: "system", subtype: "compact_boundary" };

describe("createSessionStartDetector", () => {
    it("an init is a new session", () => {
        expect(createSessionStartDetector()(init)).toBe(true);
    });

    it("the init a compaction writes after its boundary is not, and the next one is again", () => {
        const isNewSession = createSessionStartDetector();
        expect(isNewSession(boundary)).toBe(false);
        expect(isNewSession({ type: "assistant" })).toBe(false);
        expect(isNewSession(init)).toBe(false);
        expect(isNewSession(init)).toBe(true);
    });

    it("ignores a subagent's lines and anything that isn't system/init", () => {
        const isNewSession = createSessionStartDetector();
        expect(isNewSession({ ...init, parent_tool_use_id: "toolu_1" })).toBe(false);
        expect(isNewSession({ type: "system", subtype: "task_notification" })).toBe(false);
        expect(isNewSession({ type: "result" })).toBe(false);
    });

    it("a subagent's compaction doesn't hide the main agent's next session", () => {
        const isNewSession = createSessionStartDetector();
        expect(isNewSession({ ...boundary, parent_tool_use_id: "toolu_1" })).toBe(false);
        expect(isNewSession(init)).toBe(true);
    });
});

describe("the stream ends a turn's live tokens on a new session", () => {
    it("useAgentStream dispatches StreamSessionStarted from the detector", () => {
        const src = readFileSync(join(process.cwd(), "frontend/app/view/agent/useAgentStream.ts"), "utf8");
        expect(src).toMatch(/if \(isNewSession\(rawEvent\)\) model\.dispatchPane\(\{ type: "StreamSessionStarted" \}\)/);
    });
});
