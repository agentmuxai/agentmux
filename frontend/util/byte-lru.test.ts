// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { ByteLru } from "./byte-lru";

const bySize = (_k: string, v: string) => v.length;

describe("ByteLru", () => {
    it("evicts the least recently used entries once over its byte budget", () => {
        const c = new ByteLru<string>(10, bySize);
        c.set("a", "xxxx");
        c.set("b", "xxxx");
        expect(c.get("a")).toBe("xxxx"); // a is now the most recent
        c.set("c", "xxxx"); // 12 > 10: b goes
        expect(c.has("b")).toBe(false);
        expect(c.has("a")).toBe(true);
        expect(c.has("c")).toBe(true);
        expect(c.bytes).toBe(8);
    });

    it("replacing a key adjusts the total", () => {
        const c = new ByteLru<string>(10, bySize);
        c.set("a", "xxxxxx");
        c.set("a", "xx");
        expect(c.bytes).toBe(2);
        expect(c.size).toBe(1);
    });

    it("doesn't keep an entry bigger than the whole budget", () => {
        const c = new ByteLru<string>(10, bySize);
        c.set("a", "xxx");
        c.set("huge", "x".repeat(11));
        expect(c.has("huge")).toBe(false);
        expect(c.get("a")).toBe("xxx");
    });
});
