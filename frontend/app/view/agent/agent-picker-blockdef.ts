// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms } from "@/app/store/global";

const AGENT_WIDGET_KEY = "defwidget@agent";

/**
 * The block definition "+" → Agent uses: a fresh agent pane, which opens the
 * picker (My Agents, then New Agent). A split of an agent pane
 * (SPEC_AGENT_PANE_SPLIT_OPENS_PICKER_2026_09_30.md) and closing an agent tab
 * (close-agent-tab.ts) both open it, so the new pane can't be told apart from
 * a freshly added one.
 *
 * Returns a copy: callers get a blockdef they may change without touching the
 * config. It's plain JSON from widgets.json, so a JSON round trip copies it.
 */
export function agentPickerBlockDef(): BlockDef {
    const def = atoms.fullConfigAtom()?.widgets?.[AGENT_WIDGET_KEY]?.blockdef;
    return def ? (JSON.parse(JSON.stringify(def)) as BlockDef) : { meta: { view: "agent" } };
}
