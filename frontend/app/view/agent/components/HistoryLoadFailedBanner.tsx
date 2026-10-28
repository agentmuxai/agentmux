// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * HistoryLoadFailedBanner: says so when the pane couldn't load the
 * conversation's earlier messages (`InitFailed`). The pane recorded that
 * failure for years without showing it, so an unreadable transcript (after the
 * 2026-10-22 shared-store repair, one missing its last block) looked like an
 * agent with no history at all. New messages still work after it (the reducer
 * fails open), and a fresh pane tries again, so the banner says both.
 * It also says so when the load succeeded but from the block's own copy of a damaged
 * history (`InitReady` with `historyDamaged`): that copy holds only what this block
 * wrote, so a conversation that began before it opens empty or short, not "new"
 * (the 2026-10-21 pane that came back blank with no word why, #125). The agent's own
 * conversation is intact: it lives in the CLI's session, not in the saved history.
 * Same pane-level notice as ForkProviderFallbackBanner: no close button.
 */

import { Show, type Accessor, type JSX } from "solid-js";
import type { InitPhase } from "@/app/store/agent-pane-state/types";

interface HistoryLoadFailedBannerProps {
    /** The pane's init phase, e.g. `() => paneModel.state.initPhase`. */
    phase: Accessor<InitPhase>;
}

export const HistoryLoadFailedBanner = (props: HistoryLoadFailedBannerProps): JSX.Element => {
    const failed = () => {
        const phase = props.phase();
        return phase.kind === "InitFailed" ? phase : undefined;
    };
    const damaged = () => {
        const phase = props.phase();
        return phase.kind === "InitReady" && phase.historyDamaged === true;
    };
    return (
        <>
            <Show when={failed()}>
                {(phase) => (
                    <div class="fork-provider-fallback-banner history-load-failed-banner" role="status" aria-live="polite">
                        <span class="fork-provider-fallback-banner-icon" aria-hidden="true">
                            {"⚠"}
                        </span>
                        <span class="fork-provider-fallback-banner-message">
                            Couldn't load this conversation's earlier messages ({phase().reason}). New messages still
                            work. Close and reopen the pane to try again.
                        </span>
                    </div>
                )}
            </Show>
            <Show when={damaged()}>
                <div class="fork-provider-fallback-banner history-load-failed-banner" role="status" aria-live="polite">
                    <span class="fork-provider-fallback-banner-icon" aria-hidden="true">
                        {"⚠"}
                    </span>
                    <span class="fork-provider-fallback-banner-message">
                        Earlier history couldn't be loaded: the saved copy is damaged. The agent's own conversation is
                        intact, and new messages are saved.
                    </span>
                </div>
            </Show>
        </>
    );
};

HistoryLoadFailedBanner.displayName = "HistoryLoadFailedBanner";
