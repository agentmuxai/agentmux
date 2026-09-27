// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
    agentOpenRevealed,
    beginAgentOpen,
    beginAgentOpenOnMount,
    finishAgentOpen,
    formatAgentOpenLine,
    markAgentOpen,
    noteAgentOpen,
    OPEN_TIMEOUT_MS,
    openTraceEnv,
    QUIET_CAP_MS,
    QUIET_GAP_MS,
    QUIET_HOLD_MS,
    QuietDetector,
    resetAgentOpenTracesForTests,
} from "./open-trace";

describe("open-trace", () => {
    let clock = 0;
    let lines: string[] = [];
    let frames: Array<(t: number) => void> = [];
    const saved = { ...openTraceEnv };

    /** Run the pending animation frame at `t`. */
    const frameAt = (t: number) => {
        clock = t;
        const pending = frames;
        frames = [];
        for (const cb of pending) cb(t);
    };

    beforeEach(() => {
        vi.useFakeTimers();
        clock = 0;
        lines = [];
        frames = [];
        openTraceEnv.now = () => clock;
        openTraceEnv.log = (l) => lines.push(l);
        openTraceEnv.requestFrame = (cb) => frames.push(cb);
        openTraceEnv.cancelFrame = () => {
            frames = [];
        };
    });

    afterEach(() => {
        resetAgentOpenTracesForTests();
        Object.assign(openTraceEnv, saved);
        vi.useRealTimers();
    });

    it("logs one line with each phase's time since the click, stopping once quiet after the reveal", () => {
        beginAgentOpen("1a2b3c4d-rest-of-id", "Lazo", "my-agents");
        clock = 180;
        markAgentOpen("1a2b3c4d-rest-of-id", "cli", { cli_source: "local_install" });
        clock = 400;
        markAgentOpen("1a2b3c4d-rest-of-id", "config");
        clock = 610;
        markAgentOpen("1a2b3c4d-rest-of-id", "history_read", { history_lines: 5000 });
        clock = 812;
        markAgentOpen("1a2b3c4d-rest-of-id", "painted");
        noteAgentOpen("1a2b3c4d-rest-of-id", { auth: "authenticated" });
        clock = 815;
        agentOpenRevealed("1a2b3c4d-rest-of-id");
        // A long frame (jank), then regular frames for QUIET_HOLD_MS.
        frameAt(830);
        frameAt(1000);
        for (let t = 1016; t <= 1000 + QUIET_HOLD_MS; t += 16) frameAt(t);
        frameAt(1000 + QUIET_HOLD_MS + 16);

        expect(lines).toHaveLength(1);
        expect(lines[0]).toBe(
            `[agent-open] agent="Lazo" block=1a2b3c4d source=my-agents outcome=quiet total=${1000 + QUIET_HOLD_MS + 16} ` +
                "cli=180 cli_source=local_install config=400 history_read=610 history_lines=5000 painted=812 " +
                "revealed=815 quiet=1000 auth=authenticated"
        );
    });

    it("keeps the first time for a phase marked twice (ResolveCli runs on launch and again on mount)", () => {
        beginAgentOpen("b1", "A", "my-agents");
        clock = 100;
        markAgentOpen("b1", "cli", { cli_source: "installed" });
        clock = 900;
        markAgentOpen("b1", "cli", { cli_source: "local_install" });
        finishAgentOpen("b1", "closed");
        expect(lines[0]).toContain("cli=100 cli_source=installed");
    });

    it("a mount doesn't restart a trace the click started", () => {
        beginAgentOpen("b1", "A", "my-agents");
        clock = 50;
        beginAgentOpenOnMount("b1", "A");
        markAgentOpen("b1", "history_start");
        finishAgentOpen("b1", "closed");
        expect(lines[0]).toContain("source=my-agents");
        expect(lines[0]).toContain("history_start=50");
    });

    it("a pane mounting without a click traces from its mount", () => {
        clock = 1000;
        beginAgentOpenOnMount("b2", "Restored agent");
        clock = 1040;
        markAgentOpen("b2", "history_start");
        finishAgentOpen("b2", "closed");
        expect(lines[0]).toBe(
            '[agent-open] agent="Restored agent" block=b2 source=mount outcome=closed total=40 history_start=40'
        );
    });

    it("marks without a trace are no-ops, and finishing twice logs once", () => {
        markAgentOpen("nobody", "cli");
        agentOpenRevealed("nobody");
        beginAgentOpen("b1", "A", "my-agents");
        finishAgentOpen("b1", "failed");
        finishAgentOpen("b1", "closed");
        expect(lines).toEqual(['[agent-open] agent="A" block=b1 source=my-agents outcome=failed total=0']);
    });

    it("a second click on the same block logs the first open as superseded", () => {
        beginAgentOpen("b1", "A", "my-agents");
        clock = 30;
        beginAgentOpen("b1", "B", "my-agents");
        expect(lines).toEqual(['[agent-open] agent="A" block=b1 source=my-agents outcome=superseded total=30']);
    });

    it("an open that never reveals is logged as a timeout", () => {
        beginAgentOpen("b1", "A", "my-agents");
        clock = OPEN_TIMEOUT_MS;
        vi.advanceTimersByTime(OPEN_TIMEOUT_MS);
        expect(lines[0]).toContain("outcome=timeout");
    });

    it("no frames after the reveal (hidden window) ends as unsettled at the cap", () => {
        beginAgentOpen("b1", "A", "my-agents");
        clock = 500;
        agentOpenRevealed("b1");
        clock = 500 + QUIET_CAP_MS;
        vi.advanceTimersByTime(QUIET_CAP_MS);
        expect(lines).toHaveLength(1);
        expect(lines[0]).toContain("outcome=unsettled");
        expect(lines[0]).toContain("revealed=500");
        expect(lines[0]).not.toContain("quiet=");
    });

    it("quotes note values with spaces", () => {
        const line = formatAgentOpenLine(
            { blockId: "b", agent: "A", source: "mount", start: 0, marks: {}, notes: { error: "no cli found" } },
            "failed",
            3
        );
        expect(line).toBe('[agent-open] agent="A" block=b source=mount outcome=failed total=3 error="no cli found"');
    });
});

describe("QuietDetector", () => {
    it("reports when regular frames began, once they've held long enough", () => {
        const d = new QuietDetector();
        expect(d.frame(0)).toBeNull();
        expect(d.frame(QUIET_GAP_MS + 1)).toBeNull(); // a gap: quiet restarts here
        let t = QUIET_GAP_MS + 1;
        let result: number | null = null;
        while (result === null) {
            t += 16;
            result = d.frame(t);
        }
        expect(result).toBe(QUIET_GAP_MS + 1);
        expect(t - (QUIET_GAP_MS + 1)).toBeGreaterThanOrEqual(QUIET_HOLD_MS);
    });
});
