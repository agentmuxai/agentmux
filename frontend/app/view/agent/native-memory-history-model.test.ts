// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { orderVersionsOldestFirst } from "./native-memory-history-model";

function meta(id: string): NativeMemoryVersionMeta {
    return {
        id,
        content_hash: "",
        parent_version_id: null,
        source: "human",
        source_detail: "{}",
        session_id: "",
        created_at: 0,
    };
}

describe("orderVersionsOldestFirst", () => {
    // versionsAtom() is newest-first: index 0 = newest, higher index = older.
    const versions = [meta("v3-newest"), meta("v2"), meta("v1-oldest")];

    it("returns [oldest, newest] regardless of click order — first click newer", () => {
        const [from, to] = orderVersionsOldestFirst("v3-newest", "v1-oldest", versions);
        expect(from).toBe("v1-oldest");
        expect(to).toBe("v3-newest");
    });

    it("returns [oldest, newest] regardless of click order — first click older", () => {
        // Regression for reagent P1: an earlier revision had this branch's
        // output backwards, putting the newer id in `from`.
        const [from, to] = orderVersionsOldestFirst("v1-oldest", "v3-newest", versions);
        expect(from).toBe("v1-oldest");
        expect(to).toBe("v3-newest");
    });

    it("handles two adjacent versions in either click order — v2 is older than v3-newest", () => {
        expect(orderVersionsOldestFirst("v2", "v3-newest", versions)).toEqual(["v2", "v3-newest"]);
        expect(orderVersionsOldestFirst("v3-newest", "v2", versions)).toEqual(["v2", "v3-newest"]);
    });

    it("falls back to input order when an id is not found", () => {
        expect(orderVersionsOldestFirst("unknown-a", "unknown-b", versions)).toEqual(["unknown-a", "unknown-b"]);
    });
});
