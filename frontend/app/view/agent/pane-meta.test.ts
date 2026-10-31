// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// The pane's block-meta reads and their defaults.

import { createSignal } from "solid-js";
import { describe, expect, it } from "vitest";
import { paneMeta } from "./pane-meta";

describe("paneMeta", () => {
    it("falls back to the agent's id and the Claude stream format when the keys are unset", () => {
        const m = paneMeta(() => ({ meta: {} }), "agent-7");
        expect(m.providerKey()).toBe("agent-7");
        expect(m.agentName()).toBe("agent-7");
        expect(m.outputFormat()).toBe("claude-stream-json");
        expect(m.cwd()).toBe("");
        expect(m.isContainer()).toBe(false);
        expect(m.agentMode()).toBeUndefined();
    });

    it("reads each key from the block's meta", () => {
        const m = paneMeta(
            () => ({
                meta: {
                    agentProvider: "codex",
                    agentName: "Coddy",
                    agentOutputFormat: "codex-json",
                    agentMode: "container",
                    "cmd:cwd": "/work",
                    "agent:sessionid": "s1",
                    "term:shellsubblockid": "sub-1",
                    "term:shellheight": 240,
                },
            }),
            "agent-7",
        );
        expect([m.providerKey(), m.agentName(), m.outputFormat(), m.agentMode(), m.cwd(), m.sessionId(), m.shellSubBlockId(), m.shellHeight()]).toEqual([
            "codex",
            "Coddy",
            "codex-json",
            "container",
            "/work",
            "s1",
            "sub-1",
            240,
        ]);
        expect(m.isContainer()).toBe(true);
    });

    it("follows the block as it changes, and survives it going away", () => {
        const [block, setBlock] = createSignal<{ meta?: Record<string, unknown> } | null>({ meta: { agentName: "A" } });
        const m = paneMeta(block, "agent-7");
        expect(m.agentName()).toBe("A");
        setBlock({ meta: { agentName: "B" } });
        expect(m.agentName()).toBe("B");
        setBlock(null);
        expect(m.agentName()).toBe("agent-7");
    });
});
