// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * TokenUsageIndicator — compact running-total readout in the status
 * bar. Clicking opens the TokenBreakdownPopover with per-service
 * detail. Zero state stays visible (muted) as an affordance — see
 * SPEC_STATUSBAR_TOKEN_USAGE_2026_04_24.md §4.1.
 */

import { createMemo, createSignal, Show, type JSX } from "solid-js";
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
                aria-label="Token usage, click for breakdown"
                data-tip="Total tokens this session"
            >
                <span class="token-usage-indicator-counts">
                    <span class="token-usage-indicator-arrow">↑</span>
                    {formatCompactNumber(total().input)}
                    {" "}
                    <span class="token-usage-indicator-arrow">↓</span>
                    {formatCompactNumber(total().output)}
                </span>
            </button>
            <Show when={open()}>
                <TokenBreakdownPopover anchor={indicatorRef} onClose={() => setOpen(false)} />
            </Show>
        </>
    );
};

TokenUsageIndicator.displayName = "TokenUsageIndicator";
