// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ForkProviderFallbackBanner — SPEC_AGENT_QUICK_FORK_NEW_TAB_2026_08_21.md
 * §4.4's "visible, non-dismissable-by-accident note" that a fork started a
 * fresh conversation instead of carrying history forward, because the
 * forked definition's provider has no equivalent to Claude's
 * `--fork-session`.
 *
 * The spec frames this as a note in the new tab's first turn, but there's
 * no seam to push a synthesized message into the conversation before the
 * pane's own stream even mounts (`SessionOutcomeNode`/`DocumentNode` are
 * only ever pushed from inside the live stream-processing loop). This
 * reuses `AgentDisconnectedBanner`'s pattern instead: a pane-level notice
 * with no close button, so it can't be swiped away by accident the way a
 * toast could — it reflects `quick-fork.ts`'s own
 * `FORK_NO_HISTORY_FALLBACK_META_KEY` block meta, set once right after the
 * fork lands, and stays for the life of the pane.
 */

import { Show, type Accessor, type JSX } from "solid-js";
import { FORK_NO_HISTORY_FALLBACK_META_KEY } from "@/app/tab/quick-fork";

interface ForkProviderFallbackBannerProps {
    /** The pane's own block meta, e.g. `block()?.meta`. */
    meta: Accessor<MetaType | undefined>;
}

export const ForkProviderFallbackBanner = (
    props: ForkProviderFallbackBannerProps,
): JSX.Element => {
    return (
        <Show when={props.meta()?.[FORK_NO_HISTORY_FALLBACK_META_KEY]}>
            <div
                class="fork-provider-fallback-banner"
                role="status"
                aria-live="polite"
            >
                <span class="fork-provider-fallback-banner-icon" aria-hidden="true">
                    {"⚠"}
                </span>
                <span class="fork-provider-fallback-banner-message">
                    This provider doesn't support forking mid-conversation — started a fresh
                    conversation instead of carrying history forward.
                </span>
            </div>
        </Show>
    );
};

ForkProviderFallbackBanner.displayName = "ForkProviderFallbackBanner";
