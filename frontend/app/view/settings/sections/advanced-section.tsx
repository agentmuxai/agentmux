// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { type JSX } from "solid-js";

import {
    AgentPaneLimits,
    AgentPaneModels,
    AgentPaneToggles,
    DeveloperGroup,
    DragDropGroup,
    EnvironmentGroup,
    SysinfoGroup,
    TerminalGroup,
} from "./advanced-section-parts";

export { ADVANCED_SETTINGS } from "./advanced-settings-index";

// ── Section: Advanced ─────────────────────────────────────────────────────────

export function AdvancedSection(): JSX.Element {
    return (
        <div class="settings-section-body">
            <TerminalGroup />
            <AgentPaneLimits />
            <AgentPaneModels />
            <AgentPaneToggles />
            <SysinfoGroup />
            <DragDropGroup />
            <EnvironmentGroup />
            <DeveloperGroup />
        </div>
    );
}
