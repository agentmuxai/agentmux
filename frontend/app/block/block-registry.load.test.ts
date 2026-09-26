// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * block-registry.ts registers every built-in pane tab as a side effect, and
 * block.tsx is what loads it. Removing the class path removed block.tsx's
 * last named import from it, and with it the only import anywhere: no
 * built-in registered and every pane rendered "No View Component", while
 * every unit test (which imports the registry itself) still passed. Guards
 * the import by source, since block.test.tsx has to mock the registry.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

describe("block.tsx", () => {
    it("imports block-registry, which registers the built-in pane tabs", () => {
        const src = readFileSync(join(__dirname, "block.tsx"), "utf8");
        expect(src).toMatch(/^import "@\/app\/block\/block-registry";$/m);
    });
});
