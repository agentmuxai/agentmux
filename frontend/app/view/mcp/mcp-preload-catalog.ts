// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Preloaded MCP Server catalog — Phase B of
 * docs/specs/SPEC_MCP_INTEGRATION_PARITY_ABLETON_PILOT_2026_07_08.md §4.6.
 * A small built-in manifest list shown in the Armory's "+ Browse catalog"
 * picker: selecting an entry pre-fills the existing add-server form instead
 * of requiring the user to hand-type a command/args JSON blob.
 *
 * `prereqNote` is deliberately NOT a `db_mcp_servers` column — §5's schema
 * migration for that (plus `config_hash`/`disabled_tools`) never shipped in
 * #2030 (that PR was scoped to just the probe + mcp-capabilities.ts). This
 * file is the source of truth for prereq text instead; the Armory joins a
 * created server back to its catalog entry by exact `name` match to show it
 * (see McpManager.tsx). That join breaks if a user renames the server after
 * creating it from the catalog — acceptable known limitation for Phase B,
 * not silently hidden.
 *
 * Scope per spec §7/§8 Q1: ship Ableton MCP first, expand only after that
 * pattern validates. TouchDesigner (researched as a candidate in
 * docs/specs/SPEC_ARMORY_PRELOADED_CREATIVE_MCP_CONNECTORS_2026_07_10.md)
 * is the first expansion, added after end-to-end manual validation against
 * a real TouchDesigner + Ableton Live instance (router up with all 12
 * routes, AbletonOSC bridge live). ComfyUI remains an unshipped candidate.
 */

export interface McpPreloadEntry {
    id: string;
    name: string;
    transport: string;
    config: Record<string, unknown>;
    /** Static remediation text — shown in the picker, and next to a created
     *  server's probe status when that status isn't "connected" (spec §6's
     *  acceptance bar: a user who hasn't opened the app yet should see this
     *  sentence in the Armory, not a bare "Error"). */
    prereqNote: string;
    /** Set only for Tier B entries with real code-exec risk inside the
     *  third-party app (SPEC_ARMORY_PRELOADED_CREATIVE_MCP_CONNECTORS
     *  _2026_07_10.md §4 policy #2). Rendered as a persistent callout
     *  regardless of connection status — unlike prereqNote, this must NOT
     *  disappear once the server is connected and actually usable, since
     *  that's exactly when the risk is live. Not fine print. */
    riskNote?: string;
    docsUrl: string;
}

export const MCP_PRELOAD_CATALOG: McpPreloadEntry[] = [
    {
        id: "ableton-live",
        name: "Ableton Live",
        transport: "stdio",
        config: { command: "uvx", args: ["ableton-mcp"] },
        prereqNote:
            "Requires Ableton Live 10+ already running, with the AbletonMCP Remote Script installed " +
            "(Live 10.1.13–12: copy it into %USERPROFILE%\\Documents\\Ableton\\User Library\\Remote Scripts\\AbletonMCP\\) " +
            "and selected as the active Control Surface (Preferences → Link, Tempo & MIDI). " +
            "This bridge cannot launch Live or install the Remote Script for you.",
        docsUrl: "https://github.com/ahujasid/ableton-mcp",
    },
    {
        id: "touchdesigner",
        name: "TouchDesigner",
        transport: "stdio",
        // Pinned per spec §4 policy #1 — validated against 1.5.0 this session
        // (real TD + Ableton instance, router up with all 12 routes). Bump
        // this deliberately and re-validate; never widen back to @latest.
        // (Spec §4 policy #3 also calls for a mandatory `verifiedAgainst`
        // field with a review cadence — not added here, out of scope for
        // this fix; neither catalog entry has it yet.)
        config: { command: "npx", args: ["-y", "touchdesigner-mcp-server@1.5.0", "--stdio"] },
        prereqNote:
            "Requires TouchDesigner already running with mcp_webserver_base.tox imported into the " +
            "project (download touchdesigner-mcp-td.zip from the v1.5.0 release at " +
            "github.com/8beeeaaat/touchdesigner-mcp/releases/tag/v1.5.0, import mcp_webserver_base.tox — " +
            "/project1/mcp_webserver_base is recommended — and keep its modules/ folder alongside " +
            "it; the component references those files by relative path. This bridge cannot launch " +
            "TouchDesigner or import the component for you.",
        riskNote:
            "This connector's tool list includes exec_python_script — arbitrary Python execution " +
            "inside the running TouchDesigner instance — as a first-class, non-opt-in tool. More " +
            "exposed than a typically-scoped bridge (e.g. Ableton's, above).",
        docsUrl: "https://github.com/8beeeaaat/touchdesigner-mcp",
    },
];

export function findPreloadEntryByName(name: string): McpPreloadEntry | undefined {
    return MCP_PRELOAD_CATALOG.find((e) => e.name === name);
}
