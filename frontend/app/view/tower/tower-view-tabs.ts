// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The Tower toolbar's view tabs: which views this machine has.

import type { TabItem } from "@/app/element/ui";
import type { TowerViewModel } from "./tower-model";
import type { TowerView as TowerViewKind } from "./tower-util";

type ViewTab = TabItem<TowerViewKind>;

const MEMORY: ViewTab = {
    id: "memory",
    label: "Memory",
    icon: "memory",
    tooltip: "AgentMux's memory, and what it is made of",
};
const AGENTS: ViewTab = { id: "agents", label: "Agents", icon: "robot", tooltip: "Each agent and what it runs" };
const PROCESSES: ViewTab = {
    id: "processes",
    label: "Processes",
    icon: "server",
    tooltip: "Every process on this machine",
};
const PANES: ViewTab = {
    id: "panes",
    label: "Panes",
    icon: "table-columns",
    tooltip: "Each window, tab and pane, and what it costs",
};

export function towerViewTabs(m: TowerViewModel): ViewTab[] {
    if (!m.hasTasks()) return [{ ...PROCESSES, tooltip: "Every process on that machine" }];
    // Memory and Panes are this computer's AgentMux only (SPEC_TOWER_PANES_VIEW_2026_10_21.md §6.4).
    return m.remote() ? [AGENTS, PROCESSES] : [MEMORY, AGENTS, PROCESSES, PANES];
}
