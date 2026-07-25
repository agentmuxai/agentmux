// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * McpCatalogPicker — the "+ Browse catalog" overlay for the Armory's MCP
 * Servers tab (SPEC_MCP_INTEGRATION_PARITY_ABLETON_PILOT_2026_07_08.md §4.6).
 * Lists MCP_PRELOAD_CATALOG; picking an entry pre-fills the existing
 * add-server form (McpCatalogModel.startFromCatalog) instead of requiring
 * the user to hand-type command/args JSON.
 */

import { For, type JSX } from "solid-js";
import { MCP_PRELOAD_CATALOG } from "./mcp-preload-catalog";
import type { McpCatalogModel } from "./mcp-model";
import "./mcp-catalog-picker.scss";

export function McpCatalogPicker(props: { model: McpCatalogModel }): JSX.Element {
    const model = props.model;

    return (
        <div class="mcp-picker-overlay" onClick={(e) => e.target === e.currentTarget && model.closeCatalogPicker()}>
            <div class="mcp-picker" role="dialog" aria-label="Browse MCP server catalog">
                <div class="mcp-picker-header">
                    <span class="mcp-picker-title">Browse catalog</span>
                    <button type="button" class="mcp-picker-close" onClick={() => model.closeCatalogPicker()} aria-label="Close">
                        ✕
                    </button>
                </div>
                <div class="mcp-picker-list">
                    <For each={MCP_PRELOAD_CATALOG}>
                        {(entry) => (
                            <button type="button" class="mcp-picker-entry" onClick={() => model.startFromCatalog(entry)}>
                                <span class="mcp-picker-entry-name">{entry.name}</span>
                                <span class="mcp-picker-entry-note">{entry.prereqNote}</span>
                                {entry.riskNote && <span class="mcp-picker-entry-risk">⚠ {entry.riskNote}</span>}
                                <span class="mcp-picker-entry-docs">{entry.docsUrl}</span>
                            </button>
                        )}
                    </For>
                </div>
            </div>
        </div>
    );
}

McpCatalogPicker.displayName = "McpCatalogPicker";
