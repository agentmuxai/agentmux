// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { AGENT_COLOR_PALETTE, dimAgentColor, isValidAgentColor, pickAgentColor } from "./agent-color";

describe("pickAgentColor", () => {
    it("is deterministic", () => {
        expect(pickAgentColor("abc-123")).toBe(pickAgentColor("abc-123"));
    });

    it("returns a palette member", () => {
        const palette = new Set(AGENT_COLOR_PALETTE);
        for (const id of ["a", "b", "c", "0d5b45f1-9b2c-4a7e-8f3d-1234567890ab", ""]) {
            expect(palette.has(pickAgentColor(id))).toBe(true);
        }
    });

    it("spreads distinct ids over more than one bucket", () => {
        const picked = new Set(Array.from({ length: 100 }, (_, i) => pickAgentColor(`agent-${i}`)));
        expect(picked.size).toBeGreaterThan(5);
    });
});

describe("isValidAgentColor", () => {
    it("accepts every palette color", () => {
        for (const hex of AGENT_COLOR_PALETTE) {
            expect(isValidAgentColor(hex)).toBe(true);
        }
    });

    it("rejects malformed values", () => {
        for (const bad of [undefined, "", "#fff", "red", "#gggggg", "#3b82f6; }", "3b82f6", "#3B82F66"]) {
            expect(isValidAgentColor(bad)).toBe(false);
        }
    });

    it("accepts uppercase hex", () => {
        expect(isValidAgentColor("#3B82F6")).toBe(true);
    });
});

describe("dimAgentColor", () => {
    it("scales channels down and stays valid", () => {
        expect(dimAgentColor("#ffffff")).toBe("#8c8c8c");
        expect(dimAgentColor("#000000")).toBe("#000000");
        for (const hex of AGENT_COLOR_PALETTE) {
            const dimmed = dimAgentColor(hex);
            expect(isValidAgentColor(dimmed)).toBe(true);
            expect(dimmed).not.toBe(hex);
        }
    });

    it("passes invalid input through unchanged", () => {
        expect(dimAgentColor("junk")).toBe("junk");
    });
});

// agent-color.ts mirrors crates/srv/src/backend/agent_color.rs by hand (no
// shared code). These read the Rust source, so a change to one side without
// the other fails here.
describe("mirrors agent_color.rs", () => {
    const rust = readFileSync(resolve(__dirname, "../../../../crates/srv/src/backend/agent_color.rs"), "utf8");

    /** The body of Rust fn `name`, up to the next top-level item. */
    function rustFn(name: string): string {
        const start = rust.indexOf(`pub fn ${name}(`);
        expect(start, `fn ${name} in agent_color.rs`).toBeGreaterThanOrEqual(0);
        const end = rust.slice(start).search(/\n\}\r?\n/);
        return rust.slice(start, start + end);
    }

    it("has the same palette, in the same order", () => {
        const m = rust.match(/pub const AGENT_COLOR_PALETTE: \[&str; (\d+)\] = \[([\s\S]*?)\];/);
        expect(m, "AGENT_COLOR_PALETTE in agent_color.rs").not.toBeNull();
        const hexes = [...m[2].matchAll(/"(#[0-9a-fA-F]{6})"/g)].map((h) => h[1]);
        expect(hexes).toHaveLength(Number(m[1]));
        expect(AGENT_COLOR_PALETTE).toEqual(hexes);
    });

    it("picks the color the Rust FNV-1a would", () => {
        const body = rustFn("pick_agent_color");
        const [offset, prime] = [...body.matchAll(/0x[0-9a-fA-F]+/g)].map((h) => BigInt(h[0]));
        expect(prime, "the FNV offset basis and prime in pick_agent_color").toBeDefined();
        const mask = (BigInt(1) << BigInt(64)) - BigInt(1);
        // Agent ids are ASCII, where UTF-8 bytes and char codes agree.
        for (const id of ["a", "abc-123", "0d5b45f1-9b2c-4a7e-8f3d-1234567890ab", ""]) {
            let hash = offset;
            for (const byte of Buffer.from(id, "utf8")) {
                hash = ((hash ^ BigInt(byte)) * prime) & mask;
            }
            expect(pickAgentColor(id), id).toBe(AGENT_COLOR_PALETTE[Number(hash % BigInt(AGENT_COLOR_PALETTE.length))]);
        }
    });

    it("dims by the same factor", () => {
        const factors = [...rustFn("dim_agent_color").matchAll(/\* ([0-9.]+)\)/g)].map((f) => Number(f[1]));
        expect(factors).toHaveLength(3);
        // Rust truncates (`as u8`) where this rounds, so a channel may differ by
        // one. Colors are stored, not re-derived, so that is not a drift.
        for (const hex of AGENT_COLOR_PALETTE) {
            const dimmed = dimAgentColor(hex);
            for (const [i, at] of [1, 3, 5].entries()) {
                const want = Math.trunc(parseInt(hex.slice(at, at + 2), 16) * factors[i]);
                expect(Math.abs(parseInt(dimmed.slice(at, at + 2), 16) - want), `${hex} channel ${i}`).toBeLessThanOrEqual(1);
            }
        }
    });
});
