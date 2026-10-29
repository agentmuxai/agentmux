// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The agent pane's header has no Stash button: the Stash drawer is gone
 * (SPEC_DROP_THE_AGENT_STASH_2026_10_24.md, D6), and nothing replaces the
 * button. Seat…, Bundles… and Personal memory are in the right-click menu.
 *
 * Also: `agent:stashheight`, the drawer's saved height, is now only an ignored
 * block-meta key. A block that still carries it must open like any other, so no
 * agent-view source may read it.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { createRoot } from "solid-js";
import { describe, expect, it } from "vitest";
import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { agentPaneTabManifest } from "./agent-manifest";
import { AGENT_VIEW_SOURCES } from "./agent-view-sources";

function makeCtx(meta: Record<string, unknown>): PaneTabHostContext {
    return {
        blockId: "block-1",
        meta: () => meta,
        setMeta: async () => {},
        isFocused: () => true,
        visibility: () => "active",
    };
}

describe("agent pane header", () => {
    it("has no header buttons for a live agent pane", () => {
        createRoot((dispose) => {
            const instance = agentPaneTabManifest.create(makeCtx({ agentId: "uid-x", agentName: "AgentX" }));
            expect(instance.headerActions?.()).toEqual([]);
            dispose();
        });
    });

    it("has no Stash button, even on a block saved with the drawer's height", () => {
        createRoot((dispose) => {
            const instance = agentPaneTabManifest.create(
                makeCtx({ agentId: "uid-x", agentName: "AgentX", "agent:stashheight": 320 }),
            );
            const titles = (instance.headerActions?.() ?? []).map((b) => (b as { title?: string }).title);
            expect(titles).not.toContain("Stash");
            expect(instance.headerActions?.()).toHaveLength(0);
            dispose();
        });
    });

    it("has none before an agent is loaded or on a history tab", () => {
        createRoot((dispose) => {
            expect(agentPaneTabManifest.create(makeCtx({})).headerActions?.()).toEqual([]);
            const history = agentPaneTabManifest.create(
                makeCtx({ agentId: "uid-x", "agent:historyTabFor": "session-1" }),
            );
            expect(history.headerActions?.()).toEqual([]);
            dispose();
        });
    });
});

describe("agent:stashheight", () => {
    it("is not read by any agent-view source, so a saved block opens normally", () => {
        const readers = AGENT_VIEW_SOURCES.filter((rel) =>
            readFileSync(join(__dirname, rel), "utf8").includes("stashheight"),
        );
        expect(readers).toEqual([]);
    });
});
