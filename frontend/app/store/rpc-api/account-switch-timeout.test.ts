// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * `agent.account.switch` restarts the agent's panes, which for a container agent can take longer than
 * the RPC client's default 5 s (7.6 s was seen when a host agent's CLI auth check overlapped), and the
 * pane then reported "Couldn't bind: EC-TIME timeout (5000ms)" for a switch srv went on to finish.
 * The call waits for the switch itself, with a cap far above any real one; a caller can still choose.
 */

import { describe, expect, it, vi } from "vitest";
import { GeneratedApi } from "./generated";

const req = { agent_id: "a1", account: "acc", when: "after_turn" } as any;
const client = () => ({ rpcCall: vi.fn().mockResolvedValue({ failed: [], status: "switched" }) }) as any;

describe("AgentAccountSwitchCommand's timeout", () => {
    it("waits well past the client's 5 s default and past a switch that took 7.6 s", async () => {
        const c = client();
        await GeneratedApi.AgentAccountSwitchCommand(c, req);
        const [command, data, opts] = c.rpcCall.mock.calls[0];
        expect(command).toBe("agent.account.switch");
        expect(data).toEqual(req);
        expect(opts.timeout).toBeGreaterThanOrEqual(60_000);
    });

    it("lets a caller choose its own", async () => {
        const c = client();
        await GeneratedApi.AgentAccountSwitchCommand(c, req, { timeout: 1000 });
        expect(c.rpcCall.mock.calls[0][2].timeout).toBe(1000);
    });
});
