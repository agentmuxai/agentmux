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
        expect(shellStatusCorrection({ known: true, running: true }, 1000)).toBeNull();
    });

    // Reagent P1 round 2 on PR #2770: `known: false` means the backend has
    // no registry entry yet — this is the routine race window for a
    // genuinely live, freshly-spawned shell (shell_node_create publishes
    // BEFORE the runner registers), not a confirmed exit. Must never be
    // treated as "exited," or a real `task dev` gets misreported as failed
    // for its whole run.
    it("returns null when the backend doesn't know this shell yet (registration race)", () => {
        expect(shellStatusCorrection({ known: false, running: false }, 1000)).toBeNull();
    });

    it("maps a clean exit (code 0) to exited-ok, using the fallback timestamp", () => {
        expect(shellStatusCorrection({ known: true, running: false, exit_code: 0 }, 1000)).toEqual({
            status: "exited-ok",
            exitCode: 0,
            exitedAt: 1000,
        });
    });

    it("maps a nonzero exit code to exited-err", () => {
        expect(shellStatusCorrection({ known: true, running: false, exit_code: 1 }, 1000)).toEqual({
            status: "exited-err",
            exitCode: 1,
            exitedAt: 1000,
        });
    });

    it("maps a known-but-missing exit_code to exited-err with -1", () => {
        expect(shellStatusCorrection({ known: true, running: false }, 1000)).toEqual({
            status: "exited-err",
            exitCode: -1,
            exitedAt: 1000,
        });
    });
});
