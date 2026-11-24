// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { isPlanProblem, planDestination, type PanePlan } from "./destination-meta";
import { parseLink } from "./parse";

function plan(link: string, now = 7): PanePlan {
    const read = parseLink(link);
    if (read.ok === false) throw new Error(`${link}: ${read.code}`);
    const p = planDestination(read.link, now);
    if (isPlanProblem(p)) throw new Error(`${link}: ${p.text}`);
    return p;
}

describe("a link's place is the pane's own meta (R14)", () => {
    // Each row: the link, and the meta the pane's own opener (or a saved layout) holds for the same place.
    const rows: [string, Record<string, unknown>][] = [
        ["agentmux://settings/recording", { view: "settings", "settings:section": "recording" }],
        // openSettingsAt("kanban", "work"): the board and its ask stamp.
        ["agentmux://settings/kanban?board=work", { view: "settings", "settings:section": "kanban", "settings:kanban-board": "work", "settings:kanban-ask": 7 }],
        // openSettingsAt("kanban"): forgets the board an earlier cog asked for.
        ["agentmux://settings/kanban", { view: "settings", "settings:section": "kanban", "settings:kanban-board": null, "settings:kanban-ask": null }],
        ["agentmux://settings/recording#recording.engine", { view: "settings", "settings:section": "recording", "settings:reveal": "recording.engine", "settings:reveal-ask": 7 }],
        // openVault("cards").
        ["agentmux://vault/cards", { view: "vault", "vault:section": "cards" }],
        // openVault(undefined, "r1").
        ["agentmux://vault/request/r1", { view: "vault", "vault:request": "r1" }],
        // openConnectors("mcp").
        ["agentmux://connectors/mcp", { view: "connectors", "connectors:section": "mcp" }],
        // openMemory("personal", { agentId: "sorrel" }).
        ["agentmux://memory/personal?agent=sorrel", { view: "memory", "memory:section": "personal", "memory:agent": "sorrel" }],
        // openKanbanTask("12").
        ["agentmux://kanban/task/12", { view: "kanban", "kanban:open": "12" }],
        ["agentmux://kanban/board/work", { view: "kanban", "kanban:board": "work" }],
        // The Kanban's own ref for a board's task.
        ["agentmux://kanban/board/work/task/12", { view: "kanban", "kanban:open": "work#12" }],
        // The Reviews pane is part of the Kanban: its link is an alias of the open-pull-requests place.
        ["agentmux://kanban/pulls", { view: "kanban", "kanban:waits": "open" }],
        ["agentmux://reviews", { view: "kanban", "kanban:waits": "open" }],
        ["agentmux://swarm/supervisor", { view: "swarm", "swarm:section": "supervisor" }],
        ["agentmux://audit/agent/sorrel", { view: "audit", "audit:agent": "sorrel", "audit:section": "events" }],
        ["agentmux://audit/typed", { view: "audit", "audit:section": "typed" }],
        // The History pane is the Audit pane's Conversations section: its link is an alias.
        ["agentmux://history", { view: "audit", "audit:section": "conversations" }],
    ];
    it.each(rows)("%s", (link, meta) => {
        expect(plan(link).meta).toEqual(meta);
    });

    it("merges only the place into a pane that is already open", () => {
        expect(plan("agentmux://vault/cards").focusMeta).toEqual({ "vault:section": "cards" });
        expect(plan("agentmux://history").focusMeta).toEqual({ "audit:section": "conversations" });
    });

    it("says the setting to reveal", () => {
        expect(plan("agentmux://settings/recording#recording.engine").revealSetting).toBe("recording.engine");
    });

    it("opens the Vault on an entry by its opaque id", () => {
        const p = plan("agentmux://vault/entry/e_ab12cd34");
        expect(p.view).toBe("vault");
        expect(p.vaultEntry).toBe("e_ab12cd34");
        expect(p.focusMeta).toMatchObject({ "vault:entry-link": "e_ab12cd34" });
        expect(p.missing).toBeUndefined();
    });

    it("is one pane: never more than one view in the plan (G10)", () => {
        for (const [link] of rows) expect(typeof plan(link).view).toBe("string");
    });
});
