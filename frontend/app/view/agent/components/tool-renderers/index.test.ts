// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { TOOL_RENDERERS, registerToolRenderers } from ".";
import { _registeredLabels } from "./registry";

// SPEC_AGENT_PANE_TOOL_DESCRIPTORS_2026_09_26.md §2.5: one explicit list,
// no side-effect registration, no DispatchCard ↔ ToolOverlayLog cycle.
describe("tool-renderers/index", () => {
    it("registers exactly the explicit list, idempotently", () => {
        registerToolRenderers();
        registerToolRenderers();
        const labels = _registeredLabels();
        for (const e of TOOL_RENDERERS) expect(labels.filter((l) => l === e.label)).toHaveLength(1);
        expect(new Set(TOOL_RENDERERS.map((e) => e.label)).size).toBe(TOOL_RENDERERS.length);
    });

    it("renderer modules don't import ToolOverlayLog (the old cycle)", () => {
        for (const f of ["DispatchCard.tsx", "builtins.tsx", "index.ts"]) {
            const src = readFileSync(join(__dirname, f), "utf8");
            expect(src).not.toMatch(/from ["']\.\.\/ToolOverlayLog["']/);
        }
    });

    it("renderer modules no longer register themselves on import", () => {
        for (const f of ["SearchResults.tsx", "WebFetchResult.tsx", "RecordTable.tsx", "DispatchCard.tsx", "ToolReferences.tsx", "builtins.tsx"]) {
            const src = readFileSync(join(__dirname, f), "utf8");
            expect(src).not.toMatch(/registerToolRenderer\(/);
        }
    });
});
