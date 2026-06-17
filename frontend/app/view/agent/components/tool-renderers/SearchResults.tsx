// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SearchResults — render a WebSearch-style tool result as a list of result
 * cards (title → opens in the system browser, host, snippet) instead of a JSON
 * blob. The first rich result-kind to prove the renderer registry.
 *
 * Registered (below) for the `WebSearch` tool by name. The renderer is graceful:
 * if the result isn't actually search-shaped it falls back to `CompactResult`
 * (today's terminal-or-JSON view), so registering by name can never break an
 * unexpected payload.
 *
 * See SPEC_TOOL_RESULT_RENDERER_REGISTRY_2026_06_17.md (Phase 2).
 */

import { For, Show, type JSX } from "solid-js";
import { getApi } from "@/store/global";
import type { ToolNode } from "../../types";
import { CompactResult } from "../CompactResult";
import { MAX_TOOL_OUTPUT_LINES } from "../output-cap";
import { OutputHiddenMarker } from "../OutputHiddenMarker";
import { extractSearchResults, type SearchResultItem } from "./search-results";
import { byName, registerToolRenderer } from "./registry";

/** Compact host display for the card's URL line (e.g. "example.com/path"). */
function prettyUrl(url: string): string {
    try {
        const u = new URL(url);
        const path = u.pathname === "/" ? "" : u.pathname;
        return `${u.host}${path}`.replace(/\/$/, "");
    } catch {
        return url;
    }
}

export function SearchResults(props: { node: ToolNode }): JSX.Element {
    const items = extractSearchResults(props.node.result);
    return (
        <Show
            when={items}
            fallback={
                <CompactResult
                    tool={props.node.tool}
                    params={props.node.params as any}
                    result={props.node.result}
                />
            }
        >
            <SearchResultCards items={items!} />
        </Show>
    );
}

function SearchResultCards(props: { items: SearchResultItem[] }): JSX.Element {
    const visible = (): SearchResultItem[] => props.items.slice(0, MAX_TOOL_OUTPUT_LINES);
    const hidden = (): number => Math.max(0, props.items.length - MAX_TOOL_OUTPUT_LINES);
    const open = (url: string): void => {
        try {
            getApi().openExternal(url);
        } catch {
            /* best-effort — opening external is non-critical */
        }
    };
    return (
        <div class="agent-tool-search-results">
            <For each={visible()}>
                {(it) => (
                    <div
                        class="agent-search-card"
                        role="link"
                        tabindex="0"
                        title={it.url}
                        onClick={() => open(it.url)}
                        onKeyDown={(e) => {
                            if (e.key === "Enter" || e.key === " ") {
                                e.preventDefault();
                                open(it.url);
                            }
                        }}
                    >
                        <div class="agent-search-card-title">{it.title}</div>
                        <div class="agent-search-card-url">{prettyUrl(it.url)}</div>
                        <Show when={it.snippet}>
                            <div class="agent-search-card-snippet">{it.snippet}</div>
                        </Show>
                    </div>
                )}
            </For>
            <Show when={hidden() > 0}>
                <OutputHiddenMarker hidden={hidden()} noun="result" from="head" />
            </Show>
        </div>
    );
}

SearchResults.displayName = "SearchResults";

// Register for WebSearch by name (priority above the coarse-kind built-ins).
// `web_search` covers providers that lower-snake-case the tool name.
registerToolRenderer({
    priority: 10,
    label: "web:search",
    match: byName("WebSearch", "web_search"),
    render: (node) => <SearchResults node={node} />,
});
