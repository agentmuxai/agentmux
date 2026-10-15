// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { LaunchCost } from "@/types/rpc/LaunchCost";
import { bandWords, launchBudget, launchGroups, viaLabel, windowFromMetas } from "./agent-launch-cost";
import { launchLine } from "./MemoryAgentCard";

const part = (kind: string, tokens: number, via = "startup_file") => ({
    kind,
    name: kind,
    tokens_est: tokens,
    size_bytes: tokens * 4,
    lines: 1,
    via,
});

describe("agent-launch-cost", () => {
    it("groups the parts in launch order and leaves out the empty groups", () => {
        const cost = {
            parts: [part("operator_config", 4500), part("soul", 300), part("skills_index", 100), part("personal", 900, "session_hook")],
        } as LaunchCost;
        expect(launchGroups(cost)).toEqual([
            { key: "operator", label: "Operator Config", tokens: 4500 },
            { key: "agent", label: "Agent", tokens: 400 },
            { key: "personal", label: "Personal Memory", tokens: 900 },
        ]);
    });

    it("names how each part arrives", () => {
        expect(viaLabel({ via: "startup_file" })).toBe("Startup file");
        expect(viaLabel({ via: "session_hook" })).toBe("Session hook");
        expect(viaLabel({ via: "cli" })).toBe("Loaded by Claude Code");
    });

    it("takes the window from an open pane's reading, and none from a reading without one", () => {
        const reading = (window?: number) => ({
            "agent:context": { tokens: 46_000, model: "opencode/big-pickle", window, windowSource: "reported", source: "live" },
        });
        expect(windowFromMetas([undefined, reading(), reading(200_000)])).toEqual({ window: 200_000, model: "opencode/big-pickle" });
        expect(windowFromMetas([reading()])).toBeNull();
        expect(windowFromMetas([])).toBeNull();
    });

    it("bands a launch against a tenth of the window and says so in words when it's large", () => {
        expect(launchBudget(5_000, 200_000)).toEqual({ share: "3% of 200k", band: "low", fill: 0.25, room: 162_000 });
        expect(launchBudget(40_000, 200_000).fill).toBe(1);
        expect(launchBudget(16_000, 200_000).band).toBe("high");
        expect(launchBudget(19_000, 200_000).band).toBe("critical");
        expect(bandWords("low")).toBe("");
        expect(bandWords("high")).toBe("large");
        expect(bandWords("critical")).toBe("very large: trim it or split it");
    });

    it("writes the card's launch line with a share only when the window is known", () => {
        expect(launchLine({ total: 4_840, window: null })).toBe("Starts with ~4.8k tok");
        expect(launchLine({ total: 19_000, window: { window: 200_000, model: null } })).toBe(
            "Starts with ~19k tok · 10% of 200k · very large: trim it or split it"
        );
    });
});
