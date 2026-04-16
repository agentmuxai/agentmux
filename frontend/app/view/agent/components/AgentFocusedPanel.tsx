// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentFocusedPanel — half-pane overlay shown when the user clicks the gear
 * in the presentation-view title bar.
 *
 * Thin wrapper around AgentCardSettingsPanel. The rename flow moved to the
 * inline name editor in the block frame header (v0.33.197+).
 */

import { type JSX } from "solid-js";
import type { BlockNodeModel } from "@/app/block/blocktypes";
import { AgentCardSettingsPanel, type SettingsTab } from "./AgentCardSettingsPanel";
import type { OverlayTab } from "../agent-model";

interface AgentFocusedPanelProps {
    blockId: string;
    nodeModel: BlockNodeModel;
    agent: ForgeAgent;
    initialTab: OverlayTab;
    onClose: () => void;
    onTabChange?: (tab: SettingsTab) => void;
}

export const AgentFocusedPanel = (props: AgentFocusedPanelProps): JSX.Element => {
    // Click-outside: close when clicking the backdrop, not the panel itself.
    const handleBackdropClick = (e: MouseEvent) => {
        if (e.target === e.currentTarget) props.onClose();
    };

    return (
        <div class="agent-focused-overlay" onClick={handleBackdropClick}>
            <div class="agent-focused-panel">
                <AgentCardSettingsPanel
                    blockId={props.blockId}
                    nodeModel={props.nodeModel}
                    agent={props.agent}
                    initialTab={props.initialTab}
                    onClose={props.onClose}
                    onTabChange={props.onTabChange}
                />
            </div>
        </div>
    );
};

AgentFocusedPanel.displayName = "AgentFocusedPanel";
