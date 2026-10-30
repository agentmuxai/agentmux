// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { archiveAt } from "./audit-history-target";

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
