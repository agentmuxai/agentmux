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
 * Scope per spec §7/§8 Q1: ship Ableton MCP alone; expand only after this
 * pattern validates. TouchDesigner/ComfyUI research lives in
 * docs/specs/SPEC_ARMORY_PRELOADED_CREATIVE_MCP_CONNECTORS_2026_07_10.md as
 * candidate follow-on entries, not shipped here.
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
];

export function findPreloadEntryByName(name: string): McpPreloadEntry | undefined {
    return MCP_PRELOAD_CATALOG.find((e) => e.name === name);
}
