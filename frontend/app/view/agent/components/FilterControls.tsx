// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * FilterControls - Advanced filtering for agent document
 */

import { useAtomValue, useSetAtom } from "jotai";
import { Atom, PrimitiveAtom } from "jotai";
import React, { memo } from "react";
import type { DocumentState } from "../types";

interface FilterControlsProps {
    documentStateAtom: PrimitiveAtom<DocumentState>;
    documentStatsAtom: Atom<any>;
    updateFilter: any;
}

export const FilterControls: React.FC<FilterControlsProps> = memo(
    ({ documentStateAtom, documentStatsAtom, updateFilter }) => {
        const documentState = useAtomValue(documentStateAtom);
        const stats = useAtomValue(documentStatsAtom);
        const setFilter = useSetAtom(updateFilter);

    const { filter } = documentState;

    return (
        <div className="agent-filter-controls">
            <div className="agent-filter-title">Filters</div>
            <div className="agent-filter-options">
                <label className="agent-filter-option">
                    <input
                        type="checkbox"
                        checked={filter.showThinking}
                        onChange={(e) => setFilter({ showThinking: e.target.checked })}
                    />
                    <span>Thinking ({stats.markdownNodes})</span>
                </label>

                <label className="agent-filter-option">
                    <input
                        type="checkbox"
                        checked={filter.showSuccessfulTools}
                        onChange={(e) => setFilter({ showSuccessfulTools: e.target.checked })}
                    />
                    <span>Successful Tools ({stats.successfulTools})</span>
                </label>

                <label className="agent-filter-option">
                    <input
                        type="checkbox"
                        checked={filter.showFailedTools}
                        onChange={(e) => setFilter({ showFailedTools: e.target.checked })}
                    />
                    <span>Failed Tools ({stats.failedTools})</span>
                </label>

                <label className="agent-filter-option">
                    <input
                        type="checkbox"
                        checked={filter.showIncoming}
                        onChange={(e) => setFilter({ showIncoming: e.target.checked })}
                    />
                    <span>Incoming Messages ({stats.agentMessages})</span>
                </label>

                <label className="agent-filter-option">
                    <input
                        type="checkbox"
                        checked={filter.showOutgoing}
                        onChange={(e) => setFilter({ showOutgoing: e.target.checked })}
                    />
                    <span>Outgoing Messages ({stats.agentMessages})</span>
                </label>
            </div>
        </div>
    );
    }
);

FilterControls.displayName = "FilterControls";
