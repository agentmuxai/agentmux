// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { CONTAINER_META_CLEARED_BY_A_HOST_LAUNCH, LEFT_AGENT_META, leavePaneAgent } from "./leave-pane-agent";

describe("leavePaneAgent: an agent taken out of its pane in place", () => {
    it("stops its CLI first, then clears what the pane kept of it", async () => {
        const order: string[] = [];
        await leavePaneAgent("blk", {
            stop: async (id) => void order.push(`stop ${id}`),
            setMeta: async (m) => void order.push(`meta ${Object.keys(m).length} keys`),
        });
        expect(order[0]).toBe("stop blk");
        expect(order[1]).toMatch(/^meta \d+ keys$/);
    });

    it("clears the pane even when the stop fails (a pane that never started has nothing to stop)", async () => {
        const setMeta = vi.fn(async () => {});
        const warn = vi.fn();
        await leavePaneAgent("blk", { stop: async () => { throw new Error("no controller"); }, setMeta, warn });
        expect(setMeta).toHaveBeenCalledTimes(1);
        expect(warn).toHaveBeenCalledWith(expect.stringContaining("no controller"));
    });

    it("clears everything that made the pane a container pane or the previous agent's launch", () => {
        for (const key of [
            "agentId", "agentName", "agentMode", "agentInstanceId", "controller",
            "agent:container_image", "agent:container_volumes", "agent:container_command",
            "cmd", "cmd:args", "cmd:cwd", "cmd:env", "agent:sessionid", "agent:runtime",
        ]) {
            expect(LEFT_AGENT_META, key).toHaveProperty(key, null);
        }
    });

    it("a container agent's pane, after Back, holds no mode, image, command or environment of it (the merge srv does on SetMeta)", async () => {
        // srv's merge_meta: a null deletes the key, anything else is set.
        let meta: Record<string, unknown> = {
            agentId: "jcont-def", agentName: "JCont", agentMode: "container", controller: "subprocess",
            "agent:container_image": "ghcr.io/agentmuxai/agent-claude:latest", "agent:container_volumes": "[]", "agent:container_command": "claude",
            cmd: "claude.cmd", "cmd:args": ["--print"], "cmd:cwd": "C:/agents/jcont", "cmd:env": { CLAUDE_CONFIG_DIR: "C:/shared/providers/claude" },
            "agent:sessionid": "5cd602f7", "agent:runtime": { model: "opus" },
            "frame:title": "Control Center", "agent:stashheight": 240,
        };
        await leavePaneAgent("blk", {
            stop: async () => {},
            setMeta: async (m) => {
                const next = { ...meta };
                for (const [k, v] of Object.entries(m)) v === null ? delete next[k] : (next[k] = v);
                meta = next;
            },
        });
        expect(Object.keys(meta).sort()).toEqual(["agent:stashheight", "frame:title"]);
    });

    it("only deletes: nothing in it sets a value", () => {
        expect(Object.values(LEFT_AGENT_META).every((v) => v === null)).toBe(true);
        expect(Object.values(CONTAINER_META_CLEARED_BY_A_HOST_LAUNCH).every((v) => v === null)).toBe(true);
    });

    it("leaves what is the user's about the pane alone", () => {
        for (const key of ["frame:title", "frame:activebordercolor", "agent:stashheight", "agent:inputhistory", "term:zoom"]) {
            expect(LEFT_AGENT_META, key).not.toHaveProperty(key);
        }
    });
});
