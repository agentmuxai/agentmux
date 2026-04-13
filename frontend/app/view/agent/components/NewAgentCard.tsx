// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * NewAgentCard — "+ New agent" tile at the end of the picker list.
 *
 * PR 1 of specs/SPEC_CONSOLIDATE_FORGE_IDENTITY_INTO_AGENT_2026_04_13.md.
 *
 * Placeholder for now — clicking is a no-op until PR 2 wires it to
 * open the ForgePanel in create mode.
 */

import type { JSX } from "solid-js";

interface NewAgentCardProps {
    onClick?: () => void;
    disabled?: boolean;
}

export const NewAgentCard = (props: NewAgentCardProps): JSX.Element => {
    return (
        <button
            class="agent-card agent-card--new"
            onClick={() => props.onClick?.()}
            disabled={props.disabled}
            title="Create a new agent"
        >
            <span class="agent-card-icon">{"\u002B"}</span>
            <span class="agent-card-info">
                <span class="agent-card-name">New agent</span>
                <span class="agent-card-desc">Define a new agent in the Forge</span>
            </span>
        </button>
    );
};

NewAgentCard.displayName = "NewAgentCard";
