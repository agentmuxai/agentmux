// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * shellStatusCorrection — the pure decision function useShellNodeStream
 * uses to fast-correct a `shell_node_create`-spawned node whose
 * `ShellStatusCommand` check reveals it already exited (a replay of a
 * long-dead shell, not a live spawn). See
 * docs/retro/retro-activity-dock-stale-shell-flash-on-load-2026-08-22.md.
 */

import { describe, expect, it } from "vitest";
import { shellStatusCorrection } from "./useShellNodeStream";

describe("shellStatusCorrection", () => {
    it("returns null when the shell is still running", () => {
        expect(shellStatusCorrection({ running: true }, 1000)).toBeNull();
    });

    it("returns null when the status check failed (best-effort fallback)", () => {
        expect(shellStatusCorrection(null, 1000)).toBeNull();
    });

    it("maps a clean exit (code 0) to exited-ok, using the fallback timestamp", () => {
        expect(shellStatusCorrection({ running: false, exit_code: 0 }, 1000)).toEqual({
            status: "exited-ok",
            exitCode: 0,
            exitedAt: 1000,
        });
    });

    it("maps a nonzero exit code to exited-err", () => {
        expect(shellStatusCorrection({ running: false, exit_code: 1 }, 1000)).toEqual({
            status: "exited-err",
            exitCode: 1,
            exitedAt: 1000,
        });
    });

    it("maps a missing exit_code (unknown-id case) to exited-err with -1", () => {
        expect(shellStatusCorrection({ running: false }, 1000)).toEqual({
            status: "exited-err",
            exitCode: -1,
            exitedAt: 1000,
        });
    });
});
