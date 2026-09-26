// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SearchResults — render a WebSearch-style tool result.
 *
 * Two shapes:
 *  - Claude Code's result is one prose string (see `parseWebSearchText`): it
 *    renders as the search answer first (markdown), then a compact strip of
 *    source chips, one per domain. The answer is the content; the sources are
 *    its citations, and there are no snippets to fill a card with.
 *    SPEC_TOOL_PREVIEW_CONTENT_FIRST_2026_09_26.md §3.3–3.4.
 *  - An array of result objects (other providers / look-alike tools) renders
 *    as result cards (favicon + domain, citation number, title → opens in
 *    system browser, snippet, date). SPEC_WEBSEARCH_RICH_VIEW_2026_06_19.md.
 *
 * Registered (below) for the `WebSearch` and `web_search` tool names. The
 * renderer is graceful: a result of neither shape falls back to
 * `CompactResult`, so registering by name can never break an unexpected payload.
 * See SPEC_TOOL_RESULT_RENDERER_REGISTRY_2026_06_17.md (Phase 2).
 */

import { Markdown } from "@/app/element/markdown";
import { For, Show, createSignal, type JSX } from "solid-js";
import type { ToolNode } from "../../types";
import { CompactResult } from "../CompactResult";
import { MAX_TOOL_OUTPUT_LINES } from "../output-cap";
import { OutputHiddenMarker } from "../OutputHiddenMarker";
import {
    extractSearchResults,
    extractWebSearch,
    groupLinksByDomain,
    type ParsedWebSearch,
    type SearchResultItem,
    type SourceGroup,
} from "./search-results";
import { byName, registerToolRenderer } from "./registry";
import { faviconSrc, hostname, openUrl, prettyUrl } from "./url";

export function SearchResults(props: { node: ToolNode }): JSX.Element {
    const parsed = extractWebSearch(props.node.result);
    const items = parsed ? null : extractSearchResults(props.node.result);
    if (parsed) return <WebSearchAnswer parsed={parsed} />;
    if (items) return <SearchResultCards node={props.node} items={items} />;
    return <CompactResult tool={props.node.tool} params={props.node.params as any} result={props.node.result} />;
}

/** Enter/Space activate, like a native button or link. */
const onActivateKey = (activate: () => void) => (e: KeyboardEvent) => {
    if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        activate();
    }
};

function Favicon(props: { url: string; class: string }): JSX.Element {
    const [ok, setOk] = createSignal(true);
    const host = () => hostname(props.url);
    return (
        <Show when={ok() && host()}>
            <img class={props.class} src={faviconSrc(host())} alt="" width="14" height="14" onError={() => setOk(false)} />
        </Show>
    );
}

function WebSearchAnswer(props: { parsed: ParsedWebSearch }): JSX.Element {
    const groups = groupLinksByDomain(props.parsed.links);
    // The domain whose links are listed under the strip (a grouped chip).
    const [openDomain, setOpenDomain] = createSignal<string | null>(null);
    const openGroup = () => groups.find((g) => g.domain === openDomain());
    return (
        <div class="agent-websearch">
            <Show when={props.parsed.summary} fallback={<LinkList links={props.parsed.links} />}>
                <div class="agent-search-summary">
                    {/* scrollable={false}: this lives inside the virtualized
                        document, which owns the scroll — see renderRead in
                        ToolOverlayLog.tsx. */}
                    <Markdown text={props.parsed.summary!} scrollable={false} />
                </div>
                <Show when={groups.length > 0}>
                    <div class="agent-search-sources">
                        <div class="agent-search-sources-label">Sources</div>
                        <div class="agent-search-source-chips">
                            <For each={groups}>
                                {(g) => (
                                    <SourceChip
                                        group={g}
                                        open={openDomain() === g.domain}
                                        onToggle={() => setOpenDomain(openDomain() === g.domain ? null : g.domain)}
                                    />
                                )}
                            </For>
                        </div>
                        <Show when={openGroup()}>{(g) => <LinkList links={g().links} />}</Show>
                    </div>
                </Show>
            </Show>
        </div>
    );
}

/** One domain. A single link opens directly; several list inline under the
 *  strip rather than guessing which one was meant. */
function SourceChip(props: { group: SourceGroup; open: boolean; onToggle: () => void }): JSX.Element {
    const single = () => props.group.links.length === 1;
    const activate = () => (single() ? openUrl(props.group.links[0].url) : props.onToggle());
    return (
        <div
            class="agent-search-source"
            classList={{ "agent-search-source--open": props.open }}
            role={single() ? "link" : "button"}
            aria-expanded={single() ? undefined : props.open}
            tabindex="0"
            title={props.group.links.map((l) => `${l.title}\n${l.url}`).join("\n\n")}
            onClick={activate}
            onKeyDown={onActivateKey(activate)}
        >
            <Favicon url={props.group.links[0].url} class="agent-search-card-favicon" />
            <span class="agent-search-source-domain">{props.group.domain}</span>
            <Show when={!single()}>
                <span class="agent-search-source-count">×{props.group.links.length}</span>
            </Show>
        </div>
    );
}

/** One line per link: favicon · title · domain. */
function LinkList(props: { links: readonly SearchResultItem[] }): JSX.Element {
    return (
        <div class="agent-search-links">
            <For each={props.links}>
                {(link) => (
                    <div
                        class="agent-search-link"
                        role="link"
                        tabindex="0"
                        title={link.url}
                        onClick={() => openUrl(link.url)}
                        onKeyDown={onActivateKey(() => openUrl(link.url))}
                    >
                        <Favicon url={link.url} class="agent-search-card-favicon" />
                        <span class="agent-search-link-title">{link.title}</span>
                        <span class="agent-search-link-domain">{prettyUrl(link.url)}</span>
                    </div>
                )}
            </For>
        </div>
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
    return (
        <div
            class="agent-search-card"
            role="link"
            tabindex="0"
            title={props.item.url}
            onClick={() => openUrl(props.item.url)}
            onKeyDown={onActivateKey(() => openUrl(props.item.url))}
        >
            <div class="agent-search-card-meta">
                <div class="agent-search-card-source">
                    <Favicon url={props.item.url} class="agent-search-card-favicon" />
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
