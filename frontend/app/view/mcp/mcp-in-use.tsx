// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// "In use now" in Connectors → MCP servers: every server running agents
// report having, wherever it comes from, with each agent's status
// (mcp-observed.ts; SPEC_CONNECTORS_MCP_WHAT_AGENTS_RUN_2026_10_10.md §4).

import { For, Show, type JSX } from "solid-js";
import "./mcp-in-use.scss";
import { rowSummary, sourceLabel, type McpObservedModel } from "./mcp-observed";

const STATUS_TEXT: Record<string, string> = {
    connected: "connected",
    failed: "failed",
    "needs-auth": "needs sign-in",
    pending: "starting",
};

export function McpInUse(props: { model: McpObservedModel }): JSX.Element {
    const model = props.model;
    return (
        <section class="mcp-in-use" aria-label="MCP servers in use now">
            <h4 class="mcp-in-use-heading">In use now</h4>
            <Show
                when={model.rowsAtom().length > 0}
                fallback={
                    <p class="mcp-in-use-empty">
                        {model.loadedAtom()
                            ? "No running agent has reported its MCP servers since AgentMux started."
                            : "Loading…"}
                    </p>
                }
            >
                <ul class="mcp-in-use-list">
                    <For each={model.rowsAtom()}>
                        {(row) => (
                            <li class="mcp-in-use-row">
                                <div class="mcp-in-use-row-head">
                                    <span class="mcp-in-use-name">{row.name}</span>
                                    <span class="mcp-in-use-source">{sourceLabel(row.source)}</span>
                                    <span class="mcp-in-use-summary">{rowSummary(row)}</span>
                                </div>
                                <ul class="mcp-in-use-agents">
                                    <For each={row.uses}>
                                        {(use) => (
                                            <li
                                                class="mcp-in-use-agent"
                                                classList={{ [`is-${use.status}`]: true }}
                                                title={`${use.agent}: ${STATUS_TEXT[use.status] ?? use.status}, ${use.tools} ${use.tools === 1 ? "tool" : "tools"}`}
                                            >
                                                <span class="mcp-in-use-dot" aria-hidden="true" />
                                                {use.agent}
                                                <Show when={use.status !== "connected"}>
                                                    <span class="mcp-in-use-status">
                                                        {STATUS_TEXT[use.status] ?? use.status}
                                                    </span>
                                                </Show>
                                            </li>
                                        )}
                                    </For>
                                </ul>
                            </li>
                        )}
                    </For>
                </ul>
            </Show>
            <h4 class="mcp-in-use-heading">Catalog</h4>
        </section>
    );
}
