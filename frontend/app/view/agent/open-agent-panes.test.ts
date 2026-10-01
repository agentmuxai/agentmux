// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { AgentOpenPane } from "@/app/store/rpc-api";
import { describe, expect, it } from "vitest";
import { mergeOpenDefinitions, openAgentLocations } from "./open-agent-panes";

const pane = (block_id: string, agent_id: string, window_ids: string[], tab_name = ""): AgentOpenPane => ({
    block_id,
    agent_id,
    tab_id: `tab-${block_id}`,
    tab_name,
    window_ids,
});

describe("mergeOpenDefinitions", () => {
    // 2026-10-01: Korp lived in a floating window; the main window's own map
    // didn't have it, so My agents offered a reattach.
    it("includes an agent open only in another (floating) window", () => {
        const merged = mergeOpenDefinitions(new Map(), [pane("korp-block", "korp", ["floating-win"])], "main-win");
        expect(merged.get("korp")).toBe("korp-block");
    });

    it("prefers this window's pane over one elsewhere", () => {
        const merged = mergeOpenDefinitions(
            new Map(),
            [pane("far", "a", ["other-win"]), pane("near", "a", ["main-win"])],
            "main-win"
        );
        expect(merged.get("a")).toBe("near");
    });

    it("keeps a local pane srv hasn't reported yet", () => {
        const merged = mergeOpenDefinitions(new Map([["a", "fresh"]]), [], "main-win");
        expect(merged.get("a")).toBe("fresh");
    });

    it("skips entries without an agent id", () => {
        expect(mergeOpenDefinitions(new Map(), [pane("b", "", ["w"])], "w").size).toBe(0);
    });
});

describe("openAgentLocations", () => {
    it("says another window for a pane elsewhere", () => {
        const loc = openAgentLocations(new Map(), [pane("korp-block", "korp", ["floating-win"])], "main-win");
        expect(loc.get("korp")).toEqual({ blockId: "korp-block", here: false, label: "open in another window" });
    });

    it("names the tab for a pane in this window", () => {
        const loc = openAgentLocations(new Map(), [pane("b", "a", ["main-win"], "Tab 3")], "main-win");
        expect(loc.get("a")).toEqual({ blockId: "b", here: true, label: "open in another pane (Tab 3)" });
    });

    it("treats a local-only pane as here", () => {
        const loc = openAgentLocations(new Map([["a", "b"]]), [], "main-win");
        expect(loc.get("a")?.here).toBe(true);
    });
});
