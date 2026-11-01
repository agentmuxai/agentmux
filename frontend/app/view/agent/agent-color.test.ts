// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { PALETTE } from "@/app/theme/palette";
import { isValidAgentColor, pickAgentColor } from "./agent-color";

describe("pickAgentColor", () => {
    it("is deterministic", () => {
        expect(pickAgentColor("abc-123")).toBe(pickAgentColor("abc-123"));
    });

    it("returns a palette member", () => {
        const hexes = new Set(PALETTE.map((s) => s.hex));
        for (const id of ["a", "b", "c", "0d5b45f1-9b2c-4a7e-8f3d-1234567890ab", ""]) {
            expect(hexes.has(pickAgentColor(id))).toBe(true);
        }
    });

    it("spreads distinct ids over more than one bucket", () => {
        const picked = new Set(Array.from({ length: 100 }, (_, i) => pickAgentColor(`agent-${i}`)));
        expect(picked.size).toBeGreaterThan(5);
    });
});

describe("isValidAgentColor", () => {
    it("accepts every palette color", () => {
        for (const { hex } of PALETTE) {
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

// The palette itself is one JSON file both sides read. What is still written
// twice is the id hash, so this reads the Rust source: a change to one side
// without the other fails here.
describe("picks the colour srv's palette.rs would", () => {
    const rust = readFileSync(resolve(__dirname, "../../../../crates/srv/src/backend/palette.rs"), "utf8");

    it("hashes with the same FNV-1a", () => {
        const start = rust.indexOf("fn fnv1a(");
        expect(start, "fn fnv1a in palette.rs").toBeGreaterThanOrEqual(0);
        const body = rust.slice(start, start + rust.slice(start).search(/\n\}\r?\n/));
        const [offset, prime] = [...body.matchAll(/0x[0-9a-fA-F]+/g)].map((h) => BigInt(h[0]));
        expect(prime, "the FNV offset basis and prime in fnv1a").toBeDefined();
        const mask = (BigInt(1) << BigInt(64)) - BigInt(1);
        // Agent ids are ASCII, where UTF-8 bytes and char codes agree.
        for (const id of ["a", "abc-123", "0d5b45f1-9b2c-4a7e-8f3d-1234567890ab", ""]) {
            let hash = offset;
            for (const byte of Buffer.from(id, "utf8")) {
                hash = ((hash ^ BigInt(byte)) * prime) & mask;
            }
            expect(pickAgentColor(id), id).toBe(PALETTE[Number(hash % BigInt(PALETTE.length))].hex);
        }
    });
});
