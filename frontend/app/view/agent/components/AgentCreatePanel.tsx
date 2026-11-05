// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentCreatePanel — the panel that opens below a harness card in the agent
 * picker's New Agent grid, pushing the cards below it down
 * (SPEC_AGENT_PICKER_INLINE_CREATE_2026_10_18.md §2.3; the pattern is P1 in
 * SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md §5.7). It replaces the
 * three modals a new agent used to take, and shows only the steps that apply:
 *
 *  1. Tools: system tools the harness needs that aren't on PATH.
 *  2. Install: the harness's CLI, when it isn't found. AgentMux installs an
 *     npm-managed CLI here (the run outlives the panel, `install-sessions.ts`);
 *     for a CLI AgentMux doesn't install, it shows the vendor's installer.
 *     Steps 1 and 2 are for a Native agent: a Container agent's CLI is in its
 *     container, so they are skipped when the runtime is Container.
 *  3. Create: the agent's name and settings (`AgentCreateForm`).
 */

import type { JSX } from "solid-js";

import { useFreeGuide } from "./create-panel-guide";
import { useInstallStep } from "./create-panel-install";
import type { AgentCreatePanelProps } from "./create-panel-model";
import { createPanelState } from "./create-panel-state";
import { useSteps } from "./create-panel-steps";
import { useToolsCheck } from "./create-panel-tools";
import { PanelView } from "./create-panel-view";
import { useToolInstallInfo } from "./use-tool-install-info";

import "./AgentCreatePanel.scss";

export type { MissingPrereq } from "./create-panel-model";

/**
 * The panel is a state object (create-panel-state.ts: the provider and the runtime choice) and a run of hook
 * groups that act on it, called in the order their resources, effects and memos must be created: the free
 * sign-up guide, the tools check, the install, then which step applies.
 */
export const AgentCreatePanel = (props: AgentCreatePanelProps): JSX.Element => {
    const s = createPanelState(props);
    const free = useFreeGuide(s);
    const tools = useToolsCheck(s);
    const install = useInstallStep(s);
    const flow = useSteps(s, tools, install);
    const info = useToolInstallInfo(tools.missing);

    return <PanelView p={{ s, free, tools, install, flow, info }} />;
};

AgentCreatePanel.displayName = "AgentCreatePanel";
