// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentPresentationHeader — the top strip of the agent pane: icon,
 * name, and the "back to picker" close button.
 *
 * Step 11 of specs/SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md.
 *
 * Pulled out of agent-view.tsx to keep the presentation component
 * focused on composition and let the header own its own rendering +
 * fallback logic for the display name.
 */

import type { Accessor, JSX } from "solid-js";
import type { ProviderDefinition } from "../providers";

interface AgentPresentationHeaderProps {
    block: Accessor<{ meta?: Record<string, any> } | undefined>;
    provider: Accessor<ProviderDefinition | undefined>;
    agentId: string;
    onBack: () => void;
}

export const AgentPresentationHeader = (props: AgentPresentationHeaderProps): JSX.Element => {
    const icon = () => props.block()?.meta?.["agentIcon"] ?? "\u26A1";
    const name = () => props.block()?.meta?.["agentName"] ?? props.provider()?.displayName ?? props.agentId;

    return (
        <div class="agent-pres-header">
            <span class="agent-pres-icon">{icon()}</span>
            <span class="agent-pres-name">{name()}</span>
            <button class="agent-pres-back" onClick={props.onBack} title="Back to agents">
                {"\u2715"}
            </button>
        </div>
    );
};

AgentPresentationHeader.displayName = "AgentPresentationHeader";
