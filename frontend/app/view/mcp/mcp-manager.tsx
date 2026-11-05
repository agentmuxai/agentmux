// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * McpManager — Connectors → MCP servers. The global MCP server catalog
 * (mcp.catalog.* MuxAPI via McpCatalogModel) and the servers running agents
 * report (McpObservedModel), as cards: an Add gallery of presets, "Your
 * servers" (the catalog) and "Also in use" (what agents run that isn't in
 * it), with a card's details and forms opening below its row
 * (docs/specs/SPEC_CONNECTORS_CARDS_2026_10_13.md §3.5). Per-agent private
 * servers are not built.
 */

import type { JSX } from "solid-js";

import { createMcpActions } from "./mcp-manager-actions";
import { McpDeleteConfirm } from "./mcp-manager-delete";
import { createMcpState } from "./mcp-manager-state";
import { McpView } from "./mcp-manager-view";
import "./mcp-cards.scss";

/**
 * The page is a state object (mcp-manager-state.ts: the two models, the signals and the memos), the actions
 * over it (mcp-manager-actions.ts), and the view parts that read both.
 */
export const McpManager = (): JSX.Element => {
    const s = createMcpState();
    const act = createMcpActions(s);

    return (
        <div class="ui-card-section ui-density-compact mcp-cards">
            <McpView s={s} act={act} />
            <McpDeleteConfirm s={s} />
        </div>
    );
};

McpManager.displayName = "McpManager";
