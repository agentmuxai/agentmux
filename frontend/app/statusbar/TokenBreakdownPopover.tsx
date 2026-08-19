// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * TokenBreakdownPopover — click-opened popover anchored under the
 * TokenUsageIndicator. Lists per-service input/output totals, a
 * grand total row, and a destructive-gated "Reset counter" action.
 *
 * Calls `usePaneOverlay` so the popover renders cleanly over any
 * browser pane HWND (same airspace pattern as MoreDropdown and the
 * canonical `<Modal>`). Spec: SPEC_STATUSBAR_TOKEN_USAGE_2026_04_24.md §4.2.
 */

import { createMemo, createSignal, For, onCleanup, Show, type JSX } from "solid-js";
import { autoUpdate } from "@floating-ui/dom";
import { usePaneOverlay } from "@/app/platform/pane-overlay";
import { computeMenuPosition } from "@/app/util/menu-position";
import { ConfirmModal } from "@/element/modal";
import { getCliCatalogEntry } from "@/app/view/agent/defaults/cli-catalog";
import { formatCompactNumber } from "@/util/format-count";
import {
    getBreakdown,
    getCacheHitRate,
    getSessionStartAt,
    getTotal,
    resetSession,
    tokenUsageState,
    type ServiceRow,
} from "@/store/token-usage";

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
    // that the status bar overlaps. Same primitive used by `<Modal>`
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
    // null until at least one turn has reported a cache breakdown (see
    // getCacheHitRate's doc comment) — render nothing rather than a
    // misleading "0%" in that window.
    const cacheHitRate = createMemo(() => {
        void tokenUsageState.byService;
        return getCacheHitRate();
    });

    // Positioning routes through the shared primitive (Phase 3): anchored to
    // the TokenUsageIndicator rect, preferred placement top-end so the popover
    // opens upward and right-aligns to the indicator (it lives in the status
    // bar at the bottom of the window). flip/shift/size + the paintable-area
    // boundary replace the old bespoke 8px-GUTTER viewport clamp.
    const POPOVER_WIDTH = 320;
    const [floatingStyle, setFloatingStyle] = createSignal<JSX.CSSProperties>({
        position: "fixed",
        left: "0px",
        top: "0px",
    });
    let cleanupAutoUpdate: (() => void) | null = null;

    const registerFloating = (el: HTMLDivElement) => {
        rootRef = el;
        props.ref?.(el);
        requestAnimationFrame(() => {
            const r = props.anchorRect;
            if (!r || !(el instanceof Element)) return;
            const update = async () => {
                const cur = props.anchorRect;
                if (!cur) return;
                const pos = await computeMenuPosition(
                    { anchor: cur, placement: "top-end", avoidNativePanes: false },
                    el,
                );
                setFloatingStyle(pos.style);
            };
            cleanupAutoUpdate?.();
            // anchorRect is a static DOMRect → virtual reference element.
            cleanupAutoUpdate = autoUpdate(
                { getBoundingClientRect: () => props.anchorRect ?? r },
                el,
                update,
            );
            // assertMenuInPaintableArea omitted: this popover uses usePaneOverlay
            // (airspace transparency cut-out), so intentional native-pane overlap
            // would produce a false-positive [menu-guard] warning.
        });
    };

    onCleanup(() => cleanupAutoUpdate?.());

    const handleResetClick = () => setConfirmingReset(true);

    const handleConfirmReset = () => {
        resetSession();
        setConfirmingReset(false);
        props.onClose();
    };

    return (
        <>
            <div
                ref={registerFloating}
                class="token-usage-breakdown"
                role="dialog"
                aria-label="Token usage breakdown"
                data-pane-overlay
                style={{ ...floatingStyle(), width: `${POPOVER_WIDTH}px` }}
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
                                        {formatCompactNumber(row.input)}
                                        {" "}
                                        <span class="token-usage-indicator-arrow">↓</span>
                                        {formatCompactNumber(row.output)}
                                    </span>
                                </div>
                            )}
                        </For>
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
                                title="Share of input tokens served from Claude's prompt cache this session (~0.1x the cost of a fresh token). Only available for providers that report a cache breakdown."
                            >
                                {Math.round((cacheHitRate() as number) * 100)}% of input served from cache
                            </div>
                        </Show>
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
