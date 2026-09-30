// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The TS slug rules against the Rust ones they mirror: both run over the same
// table, read from crates/common/src/slug.rs's `the_rules_side_by_side`.

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { deriveSlug, jsAsciiSlug } from "./agent-config-builder";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../../..");

/** The (input, definition_slug, path_slug, file_stem, js_ascii_slug) rows. */
function rustTable(): string[][] {
    const src = readFileSync(resolve(repoRoot, "crates/common/src/slug.rs"), "utf8");
    const block = /fn the_rules_side_by_side\(\) \{[\s\S]*?&\[([\s\S]*?)\n\s*\];/.exec(src);
    if (!block) throw new Error("the_rules_side_by_side table not found in crates/common/src/slug.rs");
    const rows = [...block[1].matchAll(/\(\s*((?:"(?:[^"\\]|\\.)*"\s*,?\s*){5})\)/g)];
    return rows.map((r) => [...r[1].matchAll(/"(?:[^"\\]|\\.)*"/g)].map((q) => JSON.parse(q[0]) as string));
}

describe("TS slug rules match the Rust table", () => {
    const table = rustTable();

    it("finds the Rust table", () => {
        expect(table.length).toBeGreaterThanOrEqual(7);
        expect(table[0]).toEqual(["Zed Bot", "zed-bot", "zed-bot", "zed_bot", "zed-bot"]);
    });

    it.each(table)("deriveSlug(%j) is definition_slug", (input, definition) => {
        expect(deriveSlug(input)).toBe(definition);
    });

    it.each(table)("jsAsciiSlug(%j) is js_ascii_slug", (input, _d, _p, _f, js) => {
        expect(jsAsciiSlug(input)).toBe(js);
    });
});
