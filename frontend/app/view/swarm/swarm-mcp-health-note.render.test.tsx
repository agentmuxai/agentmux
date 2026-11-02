// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Repairing tools" / "Tools down" on a Swarm row: the state of the agent's AgentMux tools that srv
 * stores on its block (SPEC_AGENT_MCP_SELF_HEAL_2026_10_10.md §4.2).
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";

import { McpHealthNote, mcpHealthState } from "./swarm-row-notes";

afterEach(cleanup);

const meta = (state: string) => ({ "agent:mcp_health": { state, step: 1, at: 5 } });

describe("mcpHealthState", () => {
    it("reads a repair under way as repairing and a given-up one as down", () => {
        expect(mcpHealthState(meta("healing"))).toBe("repairing");
        expect(mcpHealthState(meta("restarting"))).toBe("repairing");
        expect(mcpHealthState(meta("gave_up"))).toBe("down");
    });

    it("reads nothing when the tools are fine, the state is unknown or there is no meta", () => {
        expect(mcpHealthState({})).toBeNull();
        expect(mcpHealthState(meta("ok"))).toBeNull();
        expect(mcpHealthState({ "agent:mcp_health": null })).toBeNull();
        expect(mcpHealthState(undefined)).toBeNull();
    });
});

describe("McpHealthNote", () => {
    it("says the tools are being repaired, or are down, and shows nothing otherwise", () => {
        const repairing = render(() => <McpHealthNote meta={meta("healing")} />);
        expect(repairing.container.querySelector(".swarm-mcp-health--repairing")!.textContent).toBe("Repairing tools");
        repairing.unmount();
        const down = render(() => <McpHealthNote meta={meta("gave_up")} />);
        expect(down.container.querySelector(".swarm-mcp-health--down")!.textContent).toBe("Tools down");
        down.unmount();
        const fine = render(() => <McpHealthNote meta={{}} />);
        expect(fine.container.querySelector(".swarm-mcp-health")).toBeNull();
    });
});
