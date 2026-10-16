// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { SupervisorState } from "@/types/rpc/SupervisorState";
import {
    META_SUPERVISOR_MODE,
    META_SUPERVISOR_STATE,
    parseSupervisorHeader,
    readSupervisorMode,
    readSupervisorState,
    supervisorChipDetail,
    supervisorPauseReason,
    supervisorSupported,
} from "./supervisor";

const state = (over: Partial<SupervisorState>): SupervisorState => ({
    mode: "drive",
    status: "watching",
    continuations: 0,
    limit: 15,
    run_started_at_ms: 0,
    no_progress: 0,
    updated_at_ms: 0,
    history: [],
    ...over,
});

describe("supervisor meta", () => {
    it("reads the mode as srv does: anything unknown is off", () => {
        expect(readSupervisorMode(undefined)).toBe("off");
        expect(readSupervisorMode({ [META_SUPERVISOR_MODE]: "drive" })).toBe("drive");
        expect(readSupervisorMode({ [META_SUPERVISOR_MODE]: "steady" })).toBe("steady");
        expect(readSupervisorMode({ [META_SUPERVISOR_MODE]: "yolo" })).toBe("off");
        expect(readSupervisorMode({ [META_SUPERVISOR_MODE]: true })).toBe("off");
    });

    it("reads the state only when it has srv's shape", () => {
        expect(readSupervisorState({ [META_SUPERVISOR_STATE]: state({}) })?.mode).toBe("drive");
        expect(readSupervisorState({ [META_SUPERVISOR_STATE]: "paused" })).toBeNull();
        expect(readSupervisorState({ [META_SUPERVISOR_STATE]: { status: "paused" } })).toBeNull();
    });

    it("is offered for Claude agents only", () => {
        expect(supervisorSupported({ agentProvider: "claude" })).toBe(true);
        expect(supervisorSupported({ agentProvider: "codex" })).toBe(false);
        expect(supervisorSupported(undefined)).toBe(false);
    });

    it("gives the pause reason only for a paused, idle, supervised agent", () => {
        const meta = {
            [META_SUPERVISOR_MODE]: "steady",
            [META_SUPERVISOR_STATE]: state({ mode: "steady", status: "paused", reason: "Pick a cache." }),
        };
        expect(supervisorPauseReason(meta, false)).toBe("Pick a cache.");
        expect(supervisorPauseReason(meta, true)).toBeNull();
        expect(supervisorPauseReason({ ...meta, [META_SUPERVISOR_MODE]: undefined }, false)).toBeNull();
        const blank = { ...meta, [META_SUPERVISOR_STATE]: state({ mode: "steady", status: "paused", reason: " " }) };
        expect(supervisorPauseReason(blank, false)).toBe("The Supervisor stopped for you.");
    });

    it("says what the chip shows for each state", () => {
        expect(supervisorChipDetail(state({ status: "judging" }), "drive", false)).toBe("judging");
        expect(supervisorChipDetail(state({ status: "continued", continuations: 3 }), "drive", true)).toBe("3/15");
        expect(supervisorChipDetail(state({ status: "paused" }), "drive", false)).toBe("paused");
        expect(supervisorChipDetail(state({ status: "paused" }), "drive", true)).toBeNull();
        expect(supervisorChipDetail(state({ status: "watching" }), "drive", false)).toBeNull();
        // A state left from another mode, or the mode off, shows nothing.
        expect(supervisorChipDetail(state({ status: "judging" }), "steady", false)).toBeNull();
        expect(supervisorChipDetail(state({ status: "judging" }), "off", false)).toBeNull();
    });
});

describe("parseSupervisorHeader", () => {
    it("splits srv's header off a continuation", () => {
        expect(parseSupervisorHeader("[SUPERVISOR:MODE=drive N=3 OF=15]\nKeep going.\nWhy: it asked.")).toEqual({
            mode: "drive",
            n: 3,
            of: 15,
            body: "Keep going.\nWhy: it asked.",
        });
    });

    it("reads nothing but the exact shape", () => {
        expect(parseSupervisorHeader("Keep going.")).toBeNull();
        expect(parseSupervisorHeader("[SUPERVISOR:MODE=off N=1 OF=5]\nx")).toBeNull();
        expect(parseSupervisorHeader("[SUPERVISOR-QUOTED:MODE=drive N=1 OF=5]\nx")).toBeNull();
        expect(parseSupervisorHeader("note\n[SUPERVISOR:MODE=drive N=1 OF=5]\nx")).toBeNull();
    });
});
