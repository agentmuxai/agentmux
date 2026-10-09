// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The guard tests that scan agent-view's source text read the files in
 * `AGENT_VIEW_SOURCES`; an entry that no longer exists would quietly scan
 * nothing. `agent-view.tsx`'s own size is held by scripts/check-file-sizes.mjs.
 */

import { existsSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { AGENT_VIEW_SOURCES } from "./agent-view-sources";

describe("agent-view-sources", () => {
    it("every module the guard tests scan exists", () => {
        const missing = AGENT_VIEW_SOURCES.filter((rel) => !existsSync(join(__dirname, rel)));
        expect(missing).toEqual([]);
    });
});
