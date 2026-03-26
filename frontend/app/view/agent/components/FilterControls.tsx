// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * FilterControls - Advanced filtering for agent document
 */

import { type Accessor, type JSX } from "solid-js";
import type { SignalPair } from "../state";
import type { DocumentState } from "../types";

interface FilterControlsProps {
    documentStateAtom: SignalPair<DocumentState>;
    documentStatsAtom: Accessor<any>;
    updateFilter: (updates: Partial<DocumentState["filter"]>) => void;
}

export const FilterControls = ({
    documentStateAtom,
    documentStatsAtom,
    updateFilter,
}: FilterControlsProps): JSX.Element => {
    const [documentState] = documentStateAtom;
    const stats = documentStatsAtom;

    const { filter } = documentState();

    return (
        <div class="agent-filter-controls">
            <div class="agent-filter-title">Filters</div>
            <div class="agent-filter-options">
                <label class="agent-filter-option">
                    <input
                        type="checkbox"
                        checked={filter.showThinking}
                        onChange={(e) => updateFilter({ showThinking: (e.target as HTMLInputElement).checked })}
                    />
                    <span>Thinking ({stats().markdownNodes})</span>
                </label>

                <label class="agent-filter-option">
                    <input
                        type="checkbox"
                        checked={filter.showSuccessfulTools}
                        onChange={(e) => updateFilter({ showSuccessfulTools: (e.target as HTMLInputElement).checked })}
                    />
                    <span>Successful Tools ({stats().successfulTools})</span>
                </label>

                <label class="agent-filter-option">
                    <input
                        type="checkbox"
                        checked={filter.showFailedTools}
                        onChange={(e) => updateFilter({ showFailedTools: (e.target as HTMLInputElement).checked })}
                    />
                    <span>Failed Tools ({stats().failedTools})</span>
                </label>

                <label class="agent-filter-option">
                    <input
                        type="checkbox"
                        checked={filter.showIncoming}
                        onChange={(e) => updateFilter({ showIncoming: (e.target as HTMLInputElement).checked })}
                    />
                    <span>Incoming Messages ({stats().agentMessages})</span>
                </label>

                <label class="agent-filter-option">
                    <input
                        type="checkbox"
                        checked={filter.showOutgoing}
                        onChange={(e) => updateFilter({ showOutgoing: (e.target as HTMLInputElement).checked })}
                    />
                    <span>Outgoing Messages ({stats().agentMessages})</span>
                </label>
            </div>
        </div>
    );
};

FilterControls.displayName = "FilterControls";
