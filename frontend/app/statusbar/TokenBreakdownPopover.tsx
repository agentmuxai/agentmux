// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * TokenBreakdownPopover — click-opened popover anchored under the
 * TokenUsageIndicator. Lists per-service input/output totals, a
 * grand total row, and a destructive-gated "Reset counter" action.
 *
 * Calls `usePaneOverlay` so the popover renders cleanly over any
 * browser pane HWND (same airspace pattern as MoreDropdown and
 * modal-v2). Spec: SPEC_STATUSBAR_TOKEN_USAGE_2026_04_24.md §4.2.
 */

import { createMemo, createSignal, For, Show, type JSX } from "solid-js";
import { usePaneOverlay } from "@/app/platform/pane-overlay";
import { ConfirmModal } from "@/element/modal-v2";
import { getCliCatalogEntry } from "@/app/view/agent/defaults/cli-catalog";
import {
    getBreakdown,
    getSessionStartAt,
    getTotal,
    resetSession,
    tokenUsageState,
    type ServiceRow,
} from "@/store/token-usage";

function formatTokenCount(n: number): string {
    if (n < 1000) return String(n);
    if (n < 10_000) return `${(n / 1000).toFixed(1)}k`;
    return `${Math.round(n / 1000)}k`;
}

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

interface TokenBreakdownPopoverProps {
    anchorRect: DOMRect | null;
    onClose: () => void;
    ref?: (el: HTMLDivElement) => void;
}

export const TokenBreakdownPopover = (props: TokenBreakdownPopoverProps): JSX.Element => {
    let rootRef: HTMLDivElement | undefined;

    // Airspace cut so the popover shows through any browser pane HWND
    // that the status bar overlaps. Same primitive used by modal-v2
    // (PR #544) and MoreDropdown.
    usePaneOverlay(() => rootRef);

    const [confirmingReset, setConfirmingReset] = createSignal(false);

    // Trigger reactivity on the store so breakdown re-renders when
    // a new turn lands while the popover is open.
    const rows = createMemo((): ServiceRow[] => {
        void tokenUsageState.byService;
        return getBreakdown();
    });
    const total = createMemo(() => {
        void tokenUsageState.byService;
        return getTotal();
    });

    // Anchor positioning — popover bottom-edge pins to the status bar's
    // top, horizontally aligned to the right edge of the indicator so
    // it extends leftward into the main pane area. Clamped so it never
    // runs off the viewport.
    const POPOVER_WIDTH = 320;
    const GUTTER = 8;
    const positioning = createMemo(() => {
        const r = props.anchorRect;
        if (!r) return { bottom: GUTTER, right: GUTTER };
        const rightFromViewport = Math.max(GUTTER, window.innerWidth - r.right);
        const bottomFromViewport = Math.max(GUTTER, window.innerHeight - r.top);
        return { bottom: bottomFromViewport, right: rightFromViewport };
    });

    const handleResetClick = () => setConfirmingReset(true);

    const handleConfirmReset = () => {
        resetSession();
        setConfirmingReset(false);
        props.onClose();
    };

    return (
        <>
            <div
                ref={(el) => {
                    rootRef = el;
                    props.ref?.(el);
                }}
                class="token-usage-breakdown"
                role="dialog"
                aria-label="Token usage breakdown"
                style={{
                    position: "fixed",
                    bottom: `${positioning().bottom}px`,
                    right: `${positioning().right}px`,
                    width: `${POPOVER_WIDTH}px`,
                }}
            >
                <div class="token-usage-breakdown-header">
                    <span class="token-usage-breakdown-title">Token Usage</span>
                    <span class="token-usage-breakdown-subtitle">
                        since {formatSessionStartTime(getSessionStartAt())}
                    </span>
                </div>
                <Show
                    when={rows().length > 0}
                    fallback={
                        <div class="token-usage-breakdown-empty">
                            No turns completed yet this session.
                        </div>
                    }
                >
                    <div class="token-usage-breakdown-rows">
                        <For each={rows()}>
                            {(row) => (
                                <div class="token-usage-breakdown-row">
                                    <span class="token-usage-breakdown-row-name">
                                        {serviceDisplayName(row.id)}
                                    </span>
                                    <span class="token-usage-breakdown-row-counts">
                                        <span class="token-usage-indicator-arrow">↑</span>
                                        {formatTokenCount(row.input)}
                                        {" "}
                                        <span class="token-usage-indicator-arrow">↓</span>
                                        {formatTokenCount(row.output)}
                                    </span>
                                </div>
                            )}
                        </For>
                        <div class="token-usage-breakdown-row token-usage-breakdown-total">
                            <span class="token-usage-breakdown-row-name">Total</span>
                            <span class="token-usage-breakdown-row-counts">
                                <span class="token-usage-indicator-arrow">↑</span>
                                {formatTokenCount(total().input)}
                                {" "}
                                <span class="token-usage-indicator-arrow">↓</span>
                                {formatTokenCount(total().output)}
                            </span>
                        </div>
                    </div>
                </Show>
                <div class="token-usage-breakdown-footer">
                    <button
                        type="button"
                        class="token-usage-breakdown-reset"
                        onClick={handleResetClick}
                        disabled={rows().length === 0}
                    >
                        Reset counter
                    </button>
                </div>
            </div>
            <ConfirmModal
                open={confirmingReset()}
                title="Reset token counter?"
                description="This clears the running total for the current session. Per-pane Worked stats stay unchanged."
                confirmLabel="Reset"
                destructive={true}
                onConfirm={handleConfirmReset}
                onCancel={() => setConfirmingReset(false)}
            />
        </>
    );
};

TokenBreakdownPopover.displayName = "TokenBreakdownPopover";
