// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * TokenUsageIndicator — compact running-total readout in the status
 * bar. Clicking opens the TokenBreakdownPopover with per-service
 * detail. Zero state stays visible (muted) as an affordance — see
 * SPEC_STATUSBAR_TOKEN_USAGE_2026_04_24.md §4.1.
 */

import { createEffect, createMemo, createSignal, onCleanup, Show, type JSX } from "solid-js";
import { Portal } from "solid-js/web";
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
    const [anchorRect, setAnchorRect] = createSignal<DOMRect | null>(null);

    let indicatorRef: HTMLButtonElement | undefined;
    let popoverRef: HTMLDivElement | undefined;

    const handleToggle = () => {
        if (open()) {
            setOpen(false);
            return;
        }
        if (indicatorRef) setAnchorRect(indicatorRef.getBoundingClientRect());
        setOpen(true);
    };

    const handleKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            handleToggle();
        }
    };

    // Close on outside click — ignore clicks on the indicator button
    // or inside the popover. Uses the same dual-ref pattern as
    // MoreDropdown in action-widgets.tsx.
    createEffect(() => {
        if (!open()) return;
        const handler = (e: MouseEvent) => {
            const t = e.target as Node;
            if (indicatorRef?.contains(t) || popoverRef?.contains(t)) return;
            setOpen(false);
        };
        document.addEventListener("mousedown", handler, true);
        onCleanup(() => document.removeEventListener("mousedown", handler, true));
    });

    // Close on Esc.
    createEffect(() => {
        if (!open()) return;
        const handler = (e: KeyboardEvent) => {
            if (e.key === "Escape") {
                e.stopPropagation();
                setOpen(false);
            }
        };
        window.addEventListener("keydown", handler, true);
        onCleanup(() => window.removeEventListener("keydown", handler, true));
    });

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
                <span class="token-usage-indicator-icon" aria-hidden="true">🪙</span>
                <span class="token-usage-indicator-counts">
                    <span class="token-usage-indicator-arrow">↑</span>
                    {formatCompactNumber(total().input)}
                    {" "}
                    <span class="token-usage-indicator-arrow">↓</span>
                    {formatCompactNumber(total().output)}
                </span>
            </button>
            <Show when={open()}>
                <Portal>
                    <TokenBreakdownPopover
                        anchorRect={anchorRect()}
                        onClose={() => setOpen(false)}
                        ref={(el) => { popoverRef = el; }}
                    />
                </Portal>
            </Show>
        </>
    );
};

TokenUsageIndicator.displayName = "TokenUsageIndicator";
