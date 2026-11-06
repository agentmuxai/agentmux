// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * TokenBreakdownPopover — click-opened popover anchored under the
 * TokenUsageIndicator. Lists per-agent totals (tokens, turns, cost), a
 * collapsed "AgentMux internal" row for ambient/background usage, a
 * grand total row, and a destructive-gated "Reset counter" action.
 * Clicking a real agent row focuses that agent's pane.
 *
 * Renders through AnchoredPopover (positioning, chrome zoom, dismiss, the
 * airspace cut). Spec: SPEC_STATUSBAR_TOKEN_USAGE_2026_04_24.md §4.2,
 * SPEC_STATUSBAR_TOKEN_PANEL_BY_AGENT_2026_08_30.md.
 */

import { type JSX } from "solid-js";
import { AnchoredPopover } from "@/app/element/anchored-popover";
import { ConfirmModal } from "@/element/confirm-modal";
import { BreakdownFooter, BreakdownHeader, BreakdownRows } from "./token-breakdown-parts";
import { createTokenBreakdown, type TokenBreakdownPopoverProps } from "./token-breakdown-state";

// Opens upward, right-aligned to the indicator (it sits at the right of
// the status bar). Positioning, zoom, dismiss and the airspace cut are
// AnchoredPopover's.
const POPOVER_WIDTH = 320;

export const TokenBreakdownPopover = (props: TokenBreakdownPopoverProps): JSX.Element => {
    const s = createTokenBreakdown(props);

    return (
        <>
            <AnchoredPopover
                anchor={props.anchor}
                placement="top-end"
                onDismiss={props.onClose}
                class="token-usage-breakdown"
                role="dialog"
                aria-label="Token usage breakdown"
                style={{ width: `${POPOVER_WIDTH}px` }}
            >
                <BreakdownHeader />
                <BreakdownRows s={s} />
                <BreakdownFooter s={s} />
            </AnchoredPopover>
            <ConfirmModal
                open={s.confirmingReset()}
                title="Reset token counter?"
                description="This clears the running total for the current session. Per-pane Worked stats stay unchanged."
                confirmLabel="Reset"
                destructive={true}
                onConfirm={s.handleConfirmReset}
                onCancel={() => s.setConfirmingReset(false)}
            />
        </>
    );
};

TokenBreakdownPopover.displayName = "TokenBreakdownPopover";
