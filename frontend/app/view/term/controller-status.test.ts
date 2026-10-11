// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { isNewerControllerStatus } from "./controller-status";
import type { BlockControllerRuntimeStatus } from "@/types/rpc/BlockControllerRuntimeStatus";

const st = (shellprocstatus: string, version: number, spawn_ts_ms?: number): BlockControllerRuntimeStatus =>
    ({ blockid: "b", shellprocstatus, version, spawn_ts_ms }) as BlockControllerRuntimeStatus;

describe("isNewerControllerStatus", () => {
    it("takes the first status, and later versions of one spawn", () => {
        expect(isNewerControllerStatus(null, st("running", 1, 100))).toBe(true);
        expect(isNewerControllerStatus(st("running", 1, 100), st("done", 2, 100))).toBe(true);
        expect(isNewerControllerStatus(st("done", 2, 100), st("running", 1, 100))).toBe(false);
    });

    // A restart replaces the controller, whose versions start again: its
    // "running" (v1) lost to the old one's "done" (v2), and every Enter in the
    // pane restarted it again instead of reaching the shell (found live on an
    // SSH pane that stays open when its connection drops, #4513).
    it("takes a newer spawn's status whatever its version", () => {
        expect(isNewerControllerStatus(st("done", 2, 100), st("running", 1, 200))).toBe(true);
    });

    // The replaced controller's own "done" can arrive after the new one is
    // running: it must not put the pane back to done.
    it("never takes an older spawn's late status", () => {
        expect(isNewerControllerStatus(st("running", 1, 200), st("done", 3, 100))).toBe(false);
    });

    // A status from before any spawn (init, or a start refused) has no spawn
    // time to compare: the version decides, as before.
    it("compares versions when a spawn time is missing", () => {
        expect(isNewerControllerStatus(st("init", 0), st("running", 1, 100))).toBe(true);
        expect(isNewerControllerStatus(st("running", 1, 100), st("done", 2))).toBe(true);
        expect(isNewerControllerStatus(st("running", 3, 100), st("done", 2))).toBe(false);
    });
});
