// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

// A request kind with no registered panel would render nothing. The kinds are the `kind:` literals in
// element/modal-layer.ts; each must be registered in modal-layer-panels.tsx.
describe("modal-layer-panels", () => {
    const read = (rel: string) => readFileSync(resolve(__dirname, rel), "utf8");
    const kinds = [...read("../element/modal-layer.ts").matchAll(/^\s+kind: "([a-z-]+)";/gm)].map((m) => m[1]);
    const registered = [...read("./modal-layer-panels.tsx").matchAll(/registerModalKind\("([a-z-]+)"/g)].map((m) => m[1]);

    it("registers every modal kind", () => {
        expect(kinds.length).toBeGreaterThan(0);
        expect([...registered].sort()).toEqual([...kinds].sort());
    });
});
