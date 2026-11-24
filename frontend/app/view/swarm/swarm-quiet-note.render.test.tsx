// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "quiet 22 min" on a Swarm row (SPEC_LEADER_TOLD_WHEN_AN_AGENT_GOES_QUIET_2026_11_21.md §6): shown only for an agent
 * whose runtime row carries `quiet_since_ms`, counting up with the row's tick, and nothing otherwise.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const rows = vi.hoisted(() => ({ byId: {} as Record<string, { quiet_since_ms?: number }> }));
vi.mock("@/app/store/agent-runtime-live", () => ({ sharedAgentRuntime: () => ({ rowOf: (id: string) => rows.byId[id] }) }));

import { quietMinutes, QuietNote, quietSinceForRow } from "./swarm-quiet-note";

afterEach(cleanup);

const MIN = 60_000;
const T0 = 1_000_000;

describe("quietMinutes", () => {
    it("counts whole minutes and never says less than one", () => {
        expect(quietMinutes(0, 22 * MIN + 59_000)).toBe(22);
        expect(quietMinutes(0, 5_000)).toBe(1);
        expect(quietMinutes(10 * MIN, 5 * MIN)).toBe(1);
    });
});

describe("quietSinceForRow", () => {
    it("is the runtime row's quiet time for the agent the pane runs, and null for none", () => {
        rows.byId = { ann: { quiet_since_ms: 1234 }, bob: {} };
        expect(quietSinceForRow({ agentId: "ann" })).toBe(1234);
        expect(quietSinceForRow({ agentId: "bob" })).toBeNull();
        expect(quietSinceForRow({ agentId: "nobody" })).toBeNull();
        expect(quietSinceForRow({})).toBeNull();
        expect(quietSinceForRow(undefined)).toBeNull();
    });
});

describe("QuietNote", () => {
    it("says how long an agent has been quiet, and draws nothing for one that is not", () => {
        rows.byId = { ann: { quiet_since_ms: T0 } };
        const quiet = render(() => <QuietNote meta={{ agentId: "ann" }} now={() => T0 + 22 * MIN} />);
        expect(quiet.container.querySelector(".swarm-quiet")!.textContent).toBe("quiet 22 min");
        quiet.unmount();
        const fine = render(() => <QuietNote meta={{ agentId: "bob" }} now={() => 22 * MIN} />);
        expect(fine.container.querySelector(".swarm-quiet")).toBeNull();
    });
});
