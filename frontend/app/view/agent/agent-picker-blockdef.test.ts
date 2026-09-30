// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * agentPickerBlockDef — the block "+" → Agent opens, reused by a split of an
 * agent pane and by closing an agent tab.
 * SPEC_AGENT_PANE_SPLIT_OPENS_PICKER_2026_09_30.md.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

let config: any;
vi.mock("@/app/store/global", () => ({ atoms: { fullConfigAtom: () => config } }));

import { agentPickerBlockDef } from "./agent-picker-blockdef";

const WIDGET_META = {
    view: "agent",
    controller: "cmd",
    cmd: "",
    "cmd:args": [],
    "cmd:interactive": true,
    "cmd:runonstart": false,
};

beforeEach(() => {
    config = { widgets: { "defwidget@agent": { label: "Agent", blockdef: { meta: WIDGET_META } } } };
});

describe("agentPickerBlockDef", () => {
    it("is the configured Agent widget's blockdef", () => {
        expect(agentPickerBlockDef()).toEqual({ meta: WIDGET_META });
    });

    it("returns a copy: changing it leaves the config alone", () => {
        const def = agentPickerBlockDef();
        def.meta!["agentId"] = "lark";
        (def.meta!["cmd:args"] as string[]).push("--x");
        expect(config.widgets["defwidget@agent"].blockdef.meta).toEqual(WIDGET_META);
        expect(WIDGET_META["cmd:args"]).toEqual([]);
    });

    it("falls back to a bare agent pane when the widget isn't configured", () => {
        config = { widgets: {} };
        expect(agentPickerBlockDef()).toEqual({ meta: { view: "agent" } });
        config = undefined;
        expect(agentPickerBlockDef()).toEqual({ meta: { view: "agent" } });
    });
});
