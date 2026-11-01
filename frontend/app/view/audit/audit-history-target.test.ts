// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { archiveAt, archiveKnown } from "./audit-history-target";

const archives = [
    { archive_zoneid: "z2", archived_at_ms: 2000 },
    { archive_zoneid: "z1", archived_at_ms: 1000 },
];

describe("archiveAt", () => {
    it("is the first conversation archived at or after the time", () => {
        expect(archiveAt(archives, 500)).toBe("z1");
        expect(archiveAt(archives, 1000)).toBe("z1");
        expect(archiveAt(archives, 1500)).toBe("z2");
    });

    it("is none after the last archive, where the current conversation holds it", () => {
        expect(archiveAt(archives, 2001)).toBeUndefined();
        expect(archiveAt([], 5)).toBeUndefined();
    });
});

describe("archiveKnown", () => {
    it("is true when the whole archive list came back", () => {
        expect(archiveKnown(archives, 1)).toBe(true);
    });

    it("is false for a time before the oldest archive of a list cut at its cap, whose real archive may be left out", () => {
        const full = Array.from({ length: 100 }, (_, i) => ({ archive_zoneid: `z${i}`, archived_at_ms: 10_000 + i }));
        expect(archiveKnown(full, 5_000)).toBe(false);
        expect(archiveKnown(full, 10_050)).toBe(true);
    });
});
