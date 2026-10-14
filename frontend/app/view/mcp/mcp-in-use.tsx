// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// What running agents report about an MCP server (mcp-observed.ts;
// SPEC_CONNECTORS_MCP_WHAT_AGENTS_RUN_2026_10_10.md §4), as Connectors →
// MCP servers shows it on cards: a status dot for the card, and the
// per-agent list in its details (SPEC_CONNECTORS_CARDS_2026_10_13.md §3.5).

import { For, type JSX } from "solid-js";

import { StatusDot, type StatusTone } from "@/app/element/ui";
import { sourceLabel, type ObservedServerRow, type ServerUse } from "./mcp-observed";

const STATUS_TEXT: Record<string, string> = {
    connected: "connected",
    failed: "failed",
    "needs-auth": "needs sign-in",
    pending: "starting",
};

const USE_TONE: Record<string, StatusTone> = {
    connected: "ok",
    failed: "error",
    "needs-auth": "warn",
    pending: "busy",
};

const statusText = (status: string): string => STATUS_TEXT[status] ?? status;

export interface CardStatus {
    tone: StatusTone;
    label: string;
}

/** "Korp", "AgentY and Korp", "A, B, C and 2 more". */
function nameList(names: string[]): string {
    const unique = [...new Set(names)];
    const shown = unique.slice(0, 3);
    const more = unique.length - shown.length;
    if (more > 0) return `${shown.join(", ")} and ${more} more`;
    return shown.length > 1 ? `${shown.slice(0, -1).join(", ")} and ${shown[shown.length - 1]}` : (shown[0] ?? "");
}

/**
 * A server's card status from what agents report, not from a probe:
 * `error` when any failed; `warn` when any needs sign-in or is starting, or
 * some agent that should have it hasn't loaded it (`notLoaded`, said in the
 * label); `ok` when every agent that reports it is connected; `idle` when
 * none does.
 */
export function observedStatus(uses: ServerUse[], notLoaded: string | null = null): CardStatus {
    const by = (match: (status: string) => boolean) => uses.filter((u) => match(u.status)).map((u) => u.agent);
    const failed = by((s) => s === "failed");
    const other = by((s) => s !== "connected" && s !== "failed");
    const sentences: string[] = [];
    if (failed.length > 0) sentences.push(`Failed for ${nameList(failed)}.`);
    for (const status of new Set(uses.map((u) => u.status))) {
        if (status === "connected" || status === "failed") continue;
        const text = statusText(status);
        sentences.push(`${text[0].toUpperCase()}${text.slice(1)} for ${nameList(by((s) => s === status))}.`);
    }
    if (uses.length > 0 && failed.length === 0 && other.length === 0) {
        const n = new Set(uses.map((u) => u.agent)).size;
        sentences.push(`Connected for ${n} ${n === 1 ? "agent" : "agents"}.`);
    }
    if (uses.length === 0) sentences.push("No running agent reports it.");
    if (notLoaded) sentences.push(notLoaded);
    const tone: StatusTone =
        failed.length > 0 ? "error" : other.length > 0 || notLoaded ? "warn" : uses.length > 0 ? "ok" : "idle";
    return { tone, label: sentences.join(" ") };
}

/** Each agent that reports a server: its status and how many tools it gets. */
export function McpUsesList(props: { uses: ServerUse[] }): JSX.Element {
    return (
        <ul class="mcp-uses" aria-label="Agents that report it">
            <For each={props.uses}>
                {(use) => (
                    <li class="mcp-use" data-status={use.status}>
                        <StatusDot tone={USE_TONE[use.status] ?? "idle"} label={statusText(use.status)} />
                        <span class="mcp-use-agent">{use.agent}</span>
                        <span class="mcp-use-status">{statusText(use.status)}</span>
                        <span class="mcp-use-tools">
                            {use.tools} {use.tools === 1 ? "tool" : "tools"}
                        </span>
                    </li>
                )}
            </For>
        </ul>
    );
}

/** The details of a server agents run that isn't in the catalog: read-only. */
export function McpObservedDetail(props: { row: ObservedServerRow }): JSX.Element {
    return (
        <div class="mcp-detail">
            <p class="mcp-detail-note">
                {sourceLabel(props.row.source)}: not in AgentMux's catalog, so it is read-only here.
            </p>
            <McpUsesList uses={props.row.uses} />
        </div>
    );
}
