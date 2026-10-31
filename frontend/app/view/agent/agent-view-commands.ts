// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The agent pane's view as palette commands: "Agent view: Full / Standard / Replies / Follow
 * default" act on the focused agent pane (docs/specs/SPEC_AGENT_PANE_VERBOSITY_MODES_2026_10_29.md
 * §4.5). They are available only while an agent pane is focused.
 */

import { commandRegistry } from "@/app/store/command-registry";
import { getFocusedBlockId } from "@/app/store/block-component-registry";
import { agentViewControls } from "./agent-view-controls";
import { PRESET_LABEL, PRESET_NAMES } from "./virtualization/verbosity";

export function registerAgentViewCommands(): () => void {
    const focused = () => agentViewControls(getFocusedBlockId());
    const offs = [
        commandRegistry.register({
            id: "agent:view:open",
            label: "Agent view…",
            category: "Agent",
            icon: "eye",
            keywords: "verbosity verbose show hide thinking tool calls jekts replies standard full",
            when: () => focused() !== undefined,
            execute: () => focused()?.setPopoverOpen(true),
        }),
    ];
    offs.push(...PRESET_NAMES.map((name) =>
        commandRegistry.register({
            id: `agent:view:${name}`,
            label: `Agent view: ${PRESET_LABEL[name]}`,
            category: "Agent",
            icon: "eye",
            keywords: "verbosity verbose show hide thinking tool calls jekts replies standard full",
            when: () => focused() !== undefined,
            execute: () => focused()?.setPreset(name),
        }),
    ));
    offs.push(
        commandRegistry.register({
            id: "agent:view:follow-default",
            label: "Agent view: Follow default",
            category: "Agent",
            icon: "eye",
            keywords: "verbosity verbose show hide reset",
            when: () => focused() !== undefined,
            execute: () => focused()?.followDefault(),
        }),
    );
    return () => offs.forEach((off) => off());
}
