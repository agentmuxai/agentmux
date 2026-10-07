// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const src = () => readFileSync(join(process.cwd(), "frontend/app/view/agent/useAgentStream.ts"), "utf8");

// A turn's live tokens end only in stream order (agent-pane-state types.ts,
// `turnTokens`): the turn's own session_end, or a new CLI session's `init`.
describe("the stream ends a turn's live tokens", () => {
    it("on a main-agent `system/init`, the start of a new CLI session", () => {
        expect(src()).toMatch(
            /rawEvent\.type === "system" && rawEvent\.subtype === "init" && !rawEvent\.parent_tool_use_id\) \{\s*model\.dispatchPane\(\{ type: "StreamSessionStarted" \}\)/,
        );
    });
});
