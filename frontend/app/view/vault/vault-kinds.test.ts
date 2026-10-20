// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

import { VAULT_KINDS } from "./vault-kinds";

// srv's catalog is the source: it refuses a part or field a kind doesn't have.
const RUST = readFileSync(join(__dirname, "../../../../crates/srv/src/backend/vault/kinds.rs"), "utf8");

function rustKinds(): Map<string, { section: string; parts: string[]; fields: string[] }> {
    const ids = /const ID_FIELDS: &\[&str\] = &\[([^\]]*)\]/.exec(RUST)![1];
    const list = (s: string) => (s.trim() === "ID_FIELDS" ? ids : s).match(/"([^"]+)"/g)?.map((x) => x.slice(1, -1)) ?? [];
    const out = new Map();
    const re = /name: "([^"]+)",\s*section: VaultSection::(\w+),\s*parts: &\[([^\]]*)\],\s*fields: (&\[[^\]]*\]|ID_FIELDS)/g;
    for (const m of RUST.matchAll(re)) {
        const section = m[2].replace(/([a-z])([A-Z])/g, "$1-$2").toLowerCase();
        out.set(m[1], { section, parts: list(m[3]), fields: list(m[4].replace(/^&\[|\]$/g, "")) });
    }
    return out;
}

describe("the Vault's kinds", () => {
    it("are srv's kinds, with the same sections, parts and fields", () => {
        const rust = rustKinds();
        expect(rust.size).toBeGreaterThan(10);
        expect(VAULT_KINDS.map((k) => k.name).sort()).toEqual([...rust.keys()].sort());
        for (const k of VAULT_KINDS) {
            const r = rust.get(k.name)!;
            expect({ kind: k.name, section: k.section }).toEqual({ kind: k.name, section: r.section });
            expect(k.parts.map((p) => p.name)).toEqual(r.parts);
            expect(k.fields.map((f) => f.name)).toEqual(r.fields);
        }
    });
});
