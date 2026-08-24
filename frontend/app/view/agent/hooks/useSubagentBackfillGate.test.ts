// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { resolveBackfillStatus } from "./useSubagentBackfillGate";

// docs/retro/retro-activity-dock-flicker-survives-debounce-fix-2026-08-24.md §5.

describe("resolveBackfillStatus", () => {
    it("resolves a started status", () => {
        expect(resolveBackfillStatus({ status: "started" })).toBe("started");
    });

    it("resolves a done status", () => {
        expect(resolveBackfillStatus({ status: "done" })).toBe("done");
    });

    it("rejects an unrecognized status", () => {
        expect(resolveBackfillStatus({ status: "pending" })).toBeNull();
    });

    it("rejects a missing status", () => {
        expect(resolveBackfillStatus({})).toBeNull();
    });

    it("rejects a non-object payload", () => {
        expect(resolveBackfillStatus(null)).toBeNull();
        expect(resolveBackfillStatus(undefined)).toBeNull();
        expect(resolveBackfillStatus("not-an-object")).toBeNull();
    });
});
