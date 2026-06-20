// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SearchResults — render a WebSearch-style tool result as a list of result
 * cards (favicon + domain, citation number, title → opens in system browser,
 * snippet, date) instead of a JSON blob.
 *
 * Registered (below) for the `WebSearch` and `web_search` tool names. The
 * renderer is graceful: if the result isn't search-shaped it falls back to
 * `CompactResult` so registering by name can never break an unexpected payload.
 *
 * See SPEC_WEBSEARCH_RICH_VIEW_2026_06_19.md and
 *     SPEC_TOOL_RESULT_RENDERER_REGISTRY_2026_06_17.md (Phase 2).
 */

import { For, Show, createSignal, type JSX } from "solid-js";
import { getApi } from "@/store/global";
import type { ToolNode } from "../../types";
import { CompactResult } from "../CompactResult";
import { MAX_TOOL_OUTPUT_LINES } from "../output-cap";
import { OutputHiddenMarker } from "../OutputHiddenMarker";
import { extractSearchResults, type SearchResultItem } from "./search-results";
import { byName, registerToolRenderer } from "./registry";

/** Strip protocol; show host + path (e.g. "example.com/page"). */
function prettyUrl(url: string): string {
    try {
        const u = new URL(url);
        const path = u.pathname === "/" ? "" : u.pathname.replace(/\/$/, "");
        return `${u.host}${path}`;
    } catch {
        return url;
    }
}

/** Extract just the hostname for the favicon service. */
function hostname(url: string): string {
    try {
        return new URL(url).hostname;
    } catch {
        return "";
    }
}

function openUrl(url: string): void {
    try {
        getApi().openExternal(url);
    } catch {
        /* best-effort */
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
            <SearchResultCards node={props.node} items={items!} />
        </Show>
    );
}

function SearchResultCards(props: { node: ToolNode; items: SearchResultItem[] }): JSX.Element {
    const visible = (): SearchResultItem[] => props.items.slice(0, MAX_TOOL_OUTPUT_LINES);
    const hidden = (): number => Math.max(0, props.items.length - MAX_TOOL_OUTPUT_LINES);
    const query = (): string | undefined => (props.node.params as any)?.query;

    return (
        <div class="agent-tool-search-results">
            <div class="agent-search-header">
                <span class="agent-search-count">{props.items.length} {props.items.length === 1 ? "result" : "results"}</span>
                <Show when={query()}>
                    <span class="agent-search-separator">·</span>
                    <span class="agent-search-query">"{query()}"</span>
                </Show>
            </div>
            <For each={visible()}>
                {(it) => <SearchCard item={it} />}
            </For>
            <Show when={hidden() > 0}>
                <OutputHiddenMarker hidden={hidden()} noun="result" from="head" />
            </Show>
        </div>
    );
}

function SearchCard(props: { item: SearchResultItem }): JSX.Element {
    const [faviconOk, setFaviconOk] = createSignal(true);
    const host = () => hostname(props.item.url);
    const faviconSrc = () => `https://www.google.com/s2/favicons?domain=${host()}&sz=16`;

    return (
        <div
            class="agent-search-card"
            role="link"
            tabindex="0"
            title={props.item.url}
            onClick={() => openUrl(props.item.url)}
            onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    openUrl(props.item.url);
                }
            }}
        >
            <div class="agent-search-card-meta">
                <div class="agent-search-card-source">
                    <Show when={faviconOk() && host()}>
                        <img
                            class="agent-search-card-favicon"
                            src={faviconSrc()}
                            alt=""
                            width="14"
                            height="14"
                            onError={() => setFaviconOk(false)}
                        />
                    </Show>
                    <span class="agent-search-card-domain">{prettyUrl(props.item.url)}</span>
                </div>
                <Show when={props.item.index != null}>
                    <span class="agent-search-card-index">[{props.item.index}]</span>
                </Show>
            </div>
            <div class="agent-search-card-title">{props.item.title}</div>
            <Show when={props.item.snippet}>
                <div class="agent-search-card-snippet">{props.item.snippet}</div>
            </Show>
            <Show when={props.item.date}>
                <div class="agent-search-card-date">{props.item.date}</div>
            </Show>
        </div>
    );
}

SearchResults.displayName = "SearchResults";

// Register for WebSearch by name (priority above the coarse-kind built-ins).
registerToolRenderer({
    priority: 10,
    label: "web:search",
    match: byName("WebSearch", "web_search"),
    render: (node) => <SearchResults node={node} />,
});
