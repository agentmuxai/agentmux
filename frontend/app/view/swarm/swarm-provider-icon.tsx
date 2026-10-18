// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Show, type JSX } from "solid-js";
import { ProviderLogo } from "@/app/element/ProviderLogo";

/** Why a remote row has no provider: its instance didn't say. */
export const REMOTE_PROVIDER_UNKNOWN =
    "Its AgentMux doesn't say which provider this agent uses (AgentMux before 0.59.31 doesn't)";
/** Why a local row has no provider. */
export const LOCAL_PROVIDER_UNKNOWN = "No provider is recorded for this agent";

/**
 * An agent's provider icon in the Swarm. With none known, a neutral mark of
 * the same size keeps the rows aligned. It is never the AgentMux logo, which
 * would read as the agent's provider
 * (REPORT_SWARM_REMOTE_PROVIDER_ICON_RCA_2026_10_18.md §6.2).
 */
export function SwarmProviderIcon(props: { provider: string | null | undefined; unknownTitle: string }): JSX.Element {
    return (
        <Show
            when={props.provider?.trim()}
            fallback={
                <span class="swarm-provider-unknown" role="img" aria-label="Provider unknown" title={props.unknownTitle}>
                    <i class="fa-regular fa-circle" aria-hidden="true" />
                </span>
            }
        >
            {(p) => <ProviderLogo provider={p()} size={16} />}
        </Show>
    );
}
