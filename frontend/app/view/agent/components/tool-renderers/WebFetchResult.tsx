// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * WebFetchResult — render a WebFetch tool result as a structured content view
 * (URL header with HTTP status badge, optional title, scrollable body) instead
 * of a raw JSON blob.
 *
 * Registered (below) for the `WebFetch` and `web_fetch` tool names. Gracefully
 * falls back to `CompactResult` when the result isn't fetch-shaped.
 *
 * See SPEC_WEBFETCH_CONTENT_VIEW_2026_06_22.md and
 *     SPEC_TOOL_RESULT_RENDERER_REGISTRY_2026_06_17.md.
 */

import { Show, createSignal, type JSX } from "solid-js";
import { getApi } from "@/store/global";
import type { ToolNode } from "../../types";
import { CompactResult } from "../CompactResult";
import {
    extractFetchResult,
    httpStatusText,
    looksLikeJson,
    statusClass,
    type FetchResultData,
} from "./web-fetch-result";
import { byName, registerToolRenderer } from "./registry";

function prettyUrl(url: string): string {
    try {
        const u = new URL(url);
        const path = u.pathname === "/" ? "" : u.pathname.replace(/\/$/, "");
        return `${u.host}${path}`;
    } catch {
        return url;
    }
}

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

export function WebFetchResult(props: { node: ToolNode }): JSX.Element {
    const data = extractFetchResult(props.node.result);
    return (
        <Show
            when={data}
            fallback={
                <CompactResult
                    tool={props.node.tool}
                    params={props.node.params as any}
                    result={props.node.result}
                />
            }
        >
            <FetchResultView data={data!} />
        </Show>
    );
}

function FetchResultView(props: { data: FetchResultData }): JSX.Element {
    const [faviconOk, setFaviconOk] = createSignal(true);
    const host = () => (props.data.url ? hostname(props.data.url) : "");
    const faviconSrc = () => `https://www.google.com/s2/favicons?domain=${host()}&sz=16`;
    const isJson = () => looksLikeJson(props.data.content);
    const sClass = () =>
        props.data.status != null ? statusClass(props.data.status) : null;

    return (
        <div class="agent-tool-fetch-result">
            <Show when={props.data.url}>
                <div
                    class="agent-fetch-header"
                    role="link"
                    tabindex="0"
                    title={props.data.url}
                    onClick={() => props.data.url && openUrl(props.data.url)}
                    onKeyDown={(e) => {
                        if (e.key === "Enter" || e.key === " ") {
                            e.preventDefault();
                            props.data.url && openUrl(props.data.url);
                        }
                    }}
                >
                    <div class="agent-fetch-header-left">
                        <Show when={faviconOk() && host()}>
                            <img
                                class="agent-fetch-favicon"
                                src={faviconSrc()}
                                alt=""
                                width="14"
                                height="14"
                                onError={() => setFaviconOk(false)}
                            />
                        </Show>
                        <span class="agent-fetch-domain">{prettyUrl(props.data.url!)}</span>
                    </div>
                    <Show when={props.data.status != null}>
                        <span class={`agent-fetch-status agent-fetch-status-${sClass()}`}>
                            {props.data.status} {httpStatusText(props.data.status!)}
                        </span>
                    </Show>
                </div>
                <Show when={props.data.title}>
                    <div class="agent-fetch-title">{props.data.title}</div>
                </Show>
            </Show>

            <div class={`agent-fetch-content${isJson() ? " agent-fetch-content-json" : ""}`}>
                {props.data.content}
            </div>

            <Show when={props.data.truncated}>
                <div class="agent-fetch-truncated">
                    ⚠ Content truncated
                </div>
            </Show>
        </div>
    );
}

WebFetchResult.displayName = "WebFetchResult";

// Register for WebFetch by name (priority above the coarse-kind built-ins).
registerToolRenderer({
    priority: 10,
    label: "web:fetch",
    match: byName("WebFetch", "web_fetch"),
    render: (node) => <WebFetchResult node={node} />,
});
