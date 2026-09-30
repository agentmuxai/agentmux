// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Regrowth guard for the agent-view split
 * (docs/specs/SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §3). The 04-13
 * split took seven hooks out of `agent-view.tsx` and the file still grew by
 * half, because nothing held the line.
 */

import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { AGENT_VIEW_MAX_LINES, AGENT_VIEW_SOURCES } from "./agent-view-sources";

describe("agent-view.tsx size and its split-out modules", () => {
    it(`agent-view.tsx stays at or under ${AGENT_VIEW_MAX_LINES} lines`, () => {
        const lines = readFileSync(join(__dirname, "agent-view.tsx"), "utf8").split("\n").length - 1;
        expect(
            lines,
            "agent-view.tsx grew. Put the new code in a module (and list it in agent-view-sources.ts), " +
                "or raise AGENT_VIEW_MAX_LINES with a reason in the PR.",
        ).toBeLessThanOrEqual(AGENT_VIEW_MAX_LINES);
    });

    it("every module the guard tests scan exists", () => {
        const missing = AGENT_VIEW_SOURCES.filter((rel) => !existsSync(join(__dirname, rel)));
        expect(missing).toEqual([]);
    });
});
