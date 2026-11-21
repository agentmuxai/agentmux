// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { KINDS, kindById } from "./registry";

const KINDS_DIR = join(__dirname, "kinds");
const kindFiles = readdirSync(KINDS_DIR).filter((f) => f.endsWith(".ts") && !f.endsWith(".test.ts"));

describe("the registry", () => {
    it("holds one kind per file in kinds/, each with its own id and a distinct priority", () => {
        expect(KINDS.map((k) => k.id).sort()).toEqual(kindFiles.map((f) => f.replace(".ts", "")).sort());
        expect(new Set(KINDS.map((k) => k.priority)).size).toBe(KINDS.length);
        for (const k of KINDS) expect(kindById(k.id)).toBe(k);
    });

    it("opens only through openers.ts, and never opens a pane itself (spec §4.2)", () => {
        for (const file of kindFiles) {
            const source = readFileSync(join(KINDS_DIR, file), "utf8");
            expect(source, file).not.toMatch(/createBlock|PaneOpenCommand|openLink\b|openKanbanTask|openTaskLink|window\.open|from "@\/app\/store\/(global|misc-utils)"/);
        }
    });
});
