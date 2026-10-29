// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * TokenUsageIndicator — compact running-total readout in the status
 * bar. Clicking opens the TokenBreakdownPopover with per-service
 * detail. Zero state stays visible (muted) as an affordance — see
 * SPEC_STATUSBAR_TOKEN_USAGE_2026_04_24.md §4.1.
 */

import { createMemo, createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import { AMBIENT_BACKGROUND_MS, ambientFailingCount, watchAmbient } from "@/app/store/ambient-health";
import { getTotal, tokenUsageState } from "@/store/token-usage";
import { formatCompactNumber } from "@/util/format-count";
import { TokenBreakdownPopover } from "./TokenBreakdownPopover";

export const TokenUsageIndicator = (): JSX.Element => {
    // Trigger reactivity by reading the store field; then compute total.
    const total = createMemo(() => {
        void tokenUsageState.byService;
        return getTotal();
    });
    const isZero = () => total().input === 0 && total().output === 0;

    const [open, setOpen] = createSignal(false);
    let indicatorRef: HTMLButtonElement | undefined;

    // AgentMux's own model calls: the count only drives the dot, so a slow background ask is enough
    // while the popover (which asks faster) is closed.
    onMount(() => onCleanup(watchAmbient(AMBIENT_BACKGROUND_MS)));
    const failing = () => ambientFailingCount();

    const handleToggle = () => setOpen(!open());

    const handleKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            handleToggle();
        }
    };

    return (
        <>
            <button
                type="button"
                ref={indicatorRef}
                class="token-usage-indicator"
                classList={{ "token-usage-indicator--idle": isZero() }}
                onClick={handleToggle}
                onKeyDown={handleKeyDown}
                aria-label={failing() > 0 ? "Token usage, AgentMux internal calls failing, click for breakdown" : "Token usage, click for breakdown"}
                data-tip="Total tokens this session"
            >
                <span class="token-usage-indicator-counts">
                    <span class="token-usage-indicator-arrow">↑</span>
                    {formatCompactNumber(total().input)}
                    {" "}
                    <span class="token-usage-indicator-arrow">↓</span>
                    {formatCompactNumber(total().output)}
                </span>
                <Show when={failing() > 0}>
                    <span
                        class="token-usage-indicator-dot"
                        data-testid="ambient-failing-dot"
                        title={`AgentMux's own model calls: ${failing()} purpose${failing() === 1 ? "" : "s"} failing`}
                    />
                </Show>
            </button>
            <Show when={open()}>
                <TokenBreakdownPopover anchor={indicatorRef} onClose={() => setOpen(false)} />
            </Show>
        </>
    );
};

TokenUsageIndicator.displayName = "TokenUsageIndicator";
