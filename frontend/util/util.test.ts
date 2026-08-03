// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * `sleep` was exported here but had zero real call sites anywhere in the
 * frontend (reagent P2 on PR #2388: a first draft duplicated it as a new
 * frontend/util/async.ts instead of noticing it already existed and was
 * just never consumed) — no test coverage existed either. Scoped to just
 * `sleep` rather than a full util.ts test file; the rest of this large
 * grab-bag module is out of scope for this PR.
 */

import { describe, expect, it, vi } from "vitest";
import { sleep } from "./util";

describe("sleep", () => {
    it("resolves after the given delay", async () => {
        vi.useFakeTimers();
        const spy = vi.fn();
        sleep(1000).then(spy);
        await vi.advanceTimersByTimeAsync(999);
        expect(spy).not.toHaveBeenCalled();
        await vi.advanceTimersByTimeAsync(1);
        expect(spy).toHaveBeenCalledOnce();
        vi.useRealTimers();
    });

    it("resolves with undefined", async () => {
        vi.useFakeTimers();
        const p = sleep(0);
        await vi.advanceTimersByTimeAsync(0);
        expect(await p).toBeUndefined();
        vi.useRealTimers();
    });
});
