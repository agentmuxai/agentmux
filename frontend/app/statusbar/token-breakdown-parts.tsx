// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The parts of the token usage popover (TokenBreakdownPopover.tsx lays them out): the header, one row per agent, the
// collapsed "AgentMux internal" bucket and its per-service detail, the total and cache rate, and the reset footer.

import { createMemo, For, Show, type Accessor, type JSX } from "solid-js";
import { Button } from "@/app/element/ui";
import { getCliCatalogEntry } from "@/app/view/agent/defaults/cli-catalog";
import { formatCompactNumber } from "@/util/format-count";
import { ambientSummary } from "@/app/store/ambient-health";
import { AmbientPurposes } from "./AmbientPurposes";
import { getAgentCacheHitRate, getSessionStartAt, type AgentUsage, type ServiceRow } from "@/store/token-usage";
import { tip } from "@/app/element/tip";
import type { TokenBreakdownState } from "./token-breakdown-state";

function formatSessionStartTime(epochMs: number): string {
    const d = new Date(epochMs);
    return d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

function serviceDisplayName(id: string): string {
    const entry = getCliCatalogEntry(id);
    if (entry) return entry.displayName;
    // Titlecase fallback for unknowns.
    return id.slice(0, 1).toUpperCase() + id.slice(1);
}

/** `AgentUsage.byService` as a ServiceRow[], biggest total first (ties by
 *  service id), for one agent row. Used by the "AgentMux internal"
 *  bucket's expanded per-service detail. */
function serviceRowsOf(row: AgentUsage): ServiceRow[] {
    const rows: ServiceRow[] = Object.entries(row.byService).map(([id, u]) => ({
        id,
        input: u.input,
        output: u.output,
        freshInput: u.freshInput,
        cacheCreation: u.cacheCreation,
        cacheRead: u.cacheRead,
    }));
    rows.sort((a, b) => {
        const aTotal = a.input + a.output;
        const bTotal = b.input + b.output;
        if (bTotal !== aTotal) return bTotal - aTotal;
        return a.id.localeCompare(b.id);
    });
    return rows;
}

/** The title and when this session's count started. */
export function BreakdownHeader(): JSX.Element {
    return (
        <div class="token-usage-breakdown-header">
            <span class="token-usage-breakdown-title">Token Usage</span>
            <span class="token-usage-breakdown-subtitle">
                since {formatSessionStartTime(getSessionStartAt())}
            </span>
        </div>
    );
}

/** One agent's row: its name and cache share, turns and cost, and tokens; a press focuses its pane. */
function AgentRowButton(part: { id: string; s: TokenBreakdownState }): JSX.Element {
    const { id } = part;
    const { rowById, handleAgentClick } = part.s;
    // Keeps the last row if the agent drops out while this row is being removed.
    const row = createMemo<AgentUsage>((prev) => rowById().get(id) ?? prev);
    const agentCacheRate = createMemo(() => getAgentCacheHitRate(row()));
    return (
        <Button
            tone="quiet"
            class="token-usage-breakdown-row token-usage-breakdown-agent-row"
            onClick={() => handleAgentClick(row())}
            title={`Click to focus ${row().agentName}'s pane`}
        >
            <span class="token-usage-breakdown-row-name">
                {row().agentName}
                <Show when={agentCacheRate() != null}>
                    <span
                        class="token-usage-breakdown-row-cache"
                        {...tip(`${Math.round((agentCacheRate() as number) * 100)}% of input served from cache`)}
                    >
                        {Math.round((agentCacheRate() as number) * 100)}%
                    </span>
                </Show>
            </span>
            <span class="token-usage-breakdown-row-meta">
                {row().numTurns} {row().numTurns === 1 ? "turn" : "turns"}
                <Show when={row().costUsd > 0}>
                    {" "}·{" "}${row().costUsd.toFixed(3)}
                </Show>
            </span>
            <span class="token-usage-breakdown-row-counts">
                <span class="token-usage-indicator-arrow">↑</span>
                {formatCompactNumber(row().input)}
                {" "}
                <span class="token-usage-indicator-arrow">↓</span>
                {formatCompactNumber(row().output)}
            </span>
        </Button>
    );
}

/** The "AgentMux internal" bucket when it has tokens: a toggle row, and its services and purposes once expanded. */
function AmbientBucket(part: { row: Accessor<AgentUsage>; s: TokenBreakdownState }): JSX.Element {
    const { row } = part;
    const { ambientExpanded, setAmbientExpanded } = part.s;
    return (
        <div class="token-usage-breakdown-ambient">
            <Button
                tone="quiet"
                class="token-usage-breakdown-row token-usage-breakdown-ambient-toggle"
                onClick={() => setAmbientExpanded(!ambientExpanded())}
                aria-expanded={ambientExpanded()}
            >
                <span class="token-usage-breakdown-row-name">
                    {ambientExpanded() ? "▾" : "▸"} {row().agentName}
                </span>
                <span class="token-usage-breakdown-row-counts">
                    <span class="token-usage-indicator-arrow">↑</span>
                    {formatCompactNumber(row().input)}
                    {" "}
                    <span class="token-usage-indicator-arrow">↓</span>
                    {formatCompactNumber(row().output)}
                </span>
            </Button>
            <Show when={ambientExpanded()}>
                <div class="token-usage-breakdown-ambient-detail">
                    <For each={serviceRowsOf(row())}>
                        {(svc) => (
                            <div class="token-usage-breakdown-row token-usage-breakdown-ambient-row">
                                <span class="token-usage-breakdown-row-name">
                                    {serviceDisplayName(svc.id)}
                                </span>
                                <span class="token-usage-breakdown-row-counts">
                                    <span class="token-usage-indicator-arrow">↑</span>
                                    {formatCompactNumber(svc.input)}
                                    {" "}
                                    <span class="token-usage-indicator-arrow">↓</span>
                                    {formatCompactNumber(svc.output)}
                                </span>
                            </div>
                        )}
                    </For>
                    <AmbientPurposes />
                </div>
            </Show>
        </div>
    );
}

/** The bucket with no tokens yet, only the purposes that have something to say. */
function InternalBucket(): JSX.Element {
    return (
        <div class="token-usage-breakdown-ambient">
            <div class="token-usage-breakdown-row">
                <span class="token-usage-breakdown-row-name">AgentMux internal</span>
            </div>
            <div class="token-usage-breakdown-ambient-detail">
                <AmbientPurposes />
            </div>
        </div>
    );
}

export function BreakdownRows(part: { s: TokenBreakdownState }): JSX.Element {
    const { agentRows, realAgentIds, ambientRow, total, cacheHitRate } = part.s;
    return (
        <Show
            // A failing purpose has no tokens, so it can be the only thing there is to explain the dot.
            when={agentRows().length > 0 || ambientSummary() != null}
            fallback={
                <div class="token-usage-breakdown-empty">
                    No turns completed yet this session.
                </div>
            }
        >
            <div class="token-usage-breakdown-rows">
                <For each={realAgentIds()}>{(id) => <AgentRowButton id={id} s={part.s} />}</For>
                <Show when={ambientRow()}>{(row) => <AmbientBucket row={row} s={part.s} />}</Show>
                <Show when={!ambientRow() && ambientSummary()}>
                    <InternalBucket />
                </Show>
                <div class="token-usage-breakdown-row token-usage-breakdown-total">
                    <span class="token-usage-breakdown-row-name">Total</span>
                    <span class="token-usage-breakdown-row-counts">
                        <span class="token-usage-indicator-arrow">↑</span>
                        {formatCompactNumber(total().input)}
                        {" "}
                        <span class="token-usage-indicator-arrow">↓</span>
                        {formatCompactNumber(total().output)}
                    </span>
                </div>
                <Show when={cacheHitRate() != null}>
                    <div
                        class="token-usage-breakdown-cache-rate"
                        {...tip("Share of input tokens served from Claude's prompt cache this session (~0.1x the cost of a fresh token). Only available for providers that report a cache breakdown.")}
                    >
                        {Math.round((cacheHitRate() as number) * 100)}% of input served from cache
                    </div>
                </Show>
            </div>
        </Show>
    );
}

/** The reset button, off while there is nothing to reset. */
export function BreakdownFooter(part: { s: TokenBreakdownState }): JSX.Element {
    const { agentRows, confirmingReset, handleResetClick, handleConfirmReset, handleCancelReset } = part.s;
    let cancelRef: HTMLButtonElement | undefined;
    // The button that was pressed goes away with the question, so keyboard focus moves to the safe answer.
    const ask = () => {
        handleResetClick();
        queueMicrotask(() => cancelRef?.focus());
    };
    return (
        <div class="token-usage-breakdown-footer" data-confirming={confirmingReset() ? "true" : undefined}>
            <Show
                when={confirmingReset()}
                fallback={
                    <Button tone="danger" density="compact" onClick={ask} disabled={agentRows().length === 0}>
                        Reset counter
                    </Button>
                }
            >
                <div class="token-usage-breakdown-confirm" role="group" aria-label="Reset token counter?">
                    <div class="token-usage-breakdown-confirm-text">
                        <strong>Reset token counter?</strong> This clears the running total for the current session.
                        Per-pane Worked stats stay unchanged.
                    </div>
                    <div class="token-usage-breakdown-confirm-actions">
                        <Button ref={cancelRef} tone="quiet" density="compact" onClick={handleCancelReset}>
                            Cancel
                        </Button>
                        <Button tone="danger" density="compact" onClick={handleConfirmReset}>
                            Reset
                        </Button>
                    </div>
                </div>
            </Show>
        </div>
    );
}
