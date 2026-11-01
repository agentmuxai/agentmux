// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const setMetaCommand = vi.fn().mockResolvedValue(undefined);
const setAgentContentCommand = vi.fn().mockResolvedValue(undefined);
const setConfigCommand = vi.fn().mockResolvedValue(undefined);
let paneColors: Record<string, string | number | null> | undefined;

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        SetMetaCommand: (...args: unknown[]) => setMetaCommand(...args),
        SetAgentContentCommand: (...args: unknown[]) => setAgentContentCommand(...args),
        SetConfigCommand: (...args: unknown[]) => setConfigCommand(...args),
    },
}));
vi.mock("@/app/store/block-atom-cache", () => ({
    getSettingsKeyAtom: (key: string) => () => (key === "pane:colors" ? paneColors : undefined),
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/global", () => ({
    MOS: { makeORef: (t: string, id: string) => `${t}:${id}` },
}));

import { applyColorToAllPanes, setPaneColor, setWidgetColor } from "./pane-color-menu";
import { contrastRatio, paneRoleColor } from "./pane-color-scheme";
import { registerPaneTab } from "./pane-tab-registry";
import { slotById } from "@/app/theme/palette";

// 2026-10-02 (REPORT_PANE_TAB_COLOR_BEST_PRACTICES_2026_10_02.md §6 P3): the
// header is a subtle OKLCH tint of the identity in BOTH themes.
describe("paneRoleColor headerTint — dark theme (default)", () => {
    it("uses the explicit pick when one is present — regardless of any identity hex", () => {
        expect(paneRoleColor(120, "#3535d4", false, "headerTint")).toBe(paneRoleColor(120, undefined, false, "headerTint"));
    });

    it("gives an agent's persisted identity hex the same header as an explicit pick of its slot — one system for both sources", () => {
        for (const id of ["red", "green", "indigo"]) {
            expect(paneRoleColor(undefined, slotById(id)!.hex, false, "headerTint")).toBe(
                paneRoleColor(undefined, id, false, "headerTint"),
            );
        }
    });

    it("is a subtle tint at the neutral header's lightness, not a saturated fill", () => {
        // The neutral dark header is hsl(220, 12%, 16%) = #24272e.
        expect(contrastRatio(paneRoleColor(undefined, "red", false, "headerTint")!, "#24272e")).toBeLessThan(1.15);
    });

    it("returns undefined when there is no color source at all", () => {
        expect(paneRoleColor(undefined, undefined, false, "headerTint")).toBeUndefined();
    });
});

describe("paneRoleColor headerTint — light theme", () => {
    it("is a subtle tint near white, not the identity at full strength", () => {
        const header = paneRoleColor(undefined, "green", true, "headerTint")!;
        expect(header).not.toBe(paneRoleColor(undefined, "green", true, "identity"));
        expect(contrastRatio(header, "#ffffff")).toBeLessThan(1.2);
    });

    it("returns undefined when there is no color source at all, same as the dark-theme case", () => {
        expect(paneRoleColor(undefined, undefined, true, "headerTint")).toBeUndefined();
    });
});

describe("setPaneColor", () => {
    beforeEach(() => {
        setMetaCommand.mockClear();
        setAgentContentCommand.mockClear();
    });

    it("writes frame:color and clears the older frame:hue via SetMetaCommand", () => {
        setPaneColor("block-1", "azure");
        expect(setMetaCommand).toHaveBeenCalledWith(
            {},
            { oref: "block:block-1", meta: { "frame:color": "azure", "frame:hue": null } },
        );
    });

    it("persists the slot's stored hex as the agent's identity color when an agentId is given", () => {
        setPaneColor("block-1", "azure", "agent-42");
        expect(setAgentContentCommand).toHaveBeenCalledWith(
            {},
            { agent_id: "agent-42", content_type: "ui:color", content: slotById("azure")!.hex },
        );
    });

    it("does NOT touch agent identity when no agentId is given (non-agent pane)", () => {
        setPaneColor("block-1", "azure");
        expect(setAgentContentCommand).not.toHaveBeenCalled();
    });

    it("does NOT touch agent identity on 'Default' (null), even with an agentId — clearing one pane's color must not reset the agent's color everywhere else", () => {
        setPaneColor("block-1", null, "agent-42");
        expect(setMetaCommand).toHaveBeenCalledWith(
            {},
            { oref: "block:block-1", meta: { "frame:color": null, "frame:hue": null } },
        );
        expect(setAgentContentCommand).not.toHaveBeenCalled();
    });

    it("treats a name that is not a slot as Default, never storing it", () => {
        setPaneColor("block-1", "mauve", "agent-42");
        expect(setMetaCommand).toHaveBeenCalledWith(
            {},
            { oref: "block:block-1", meta: { "frame:color": null, "frame:hue": null } },
        );
        expect(setAgentContentCommand).not.toHaveBeenCalled();
    });
});

// SPEC_WIDGET_DEFAULT_PANE_COLORS_2026_10_05.md §3.4 and §3.7.
describe("setWidgetColor", () => {
    beforeEach(() => {
        setConfigCommand.mockClear();
        paneColors = undefined;
    });
    const written = () => setConfigCommand.mock.calls.at(-1)?.[1];

    it("adds a widget's color to the others already set", () => {
        paneColors = { browser: "green" };
        setWidgetColor("term", "indigo");
        expect(written()).toEqual({ "pane:colors": { browser: "green", term: "indigo" } });
    });

    it("keeps an older number the user still has for another widget", () => {
        paneColors = { browser: 90 };
        setWidgetColor("term", "indigo");
        expect(written()).toEqual({ "pane:colors": { browser: 90, term: "indigo" } });
    });

    it("writes null for None, which is not the same as unset", () => {
        setWidgetColor("term", null);
        expect(written()).toEqual({ "pane:colors": { term: null } });
    });

    it("removes the key on undefined, returning the widget to its built-in color", () => {
        paneColors = { browser: "green", term: "indigo" };
        setWidgetColor("term", undefined);
        expect(written()).toEqual({ "pane:colors": { browser: "green" } });
    });

    it("removes a color saved under the widget's former id, so a reset sticks", () => {
        const unregister = registerPaneTab({ apiVersion: 1, view: "t-new", aliases: ["t-old"], label: "N", icon: "square", create: () => ({}) as never });
        try {
            paneColors = { "t-old": 90, browser: "amber" };
            setWidgetColor("t-new", undefined);
            expect(written()).toEqual({ "pane:colors": { browser: "amber" } });
            paneColors = { "t-old": 90 };
            setWidgetColor("t-new", "indigo");
            expect(written()).toEqual({ "pane:colors": { "t-new": "indigo" } });
        } finally {
            unregister();
        }
    });

    it("deletes the whole setting when the last key goes", () => {
        paneColors = { term: "indigo" };
        setWidgetColor("term", undefined);
        expect(written()).toEqual({ "pane:colors": null });
    });
});

describe("applyColorToAllPanes", () => {
    beforeEach(() => {
        setConfigCommand.mockClear();
        setMetaCommand.mockClear();
        paneColors = undefined;
    });

    it("makes the pick the widget's color and clears the pane's own pick", () => {
        applyColorToAllPanes("block-1", "term", "violet");
        expect(setConfigCommand).toHaveBeenCalledWith({}, { "pane:colors": { term: "violet" } });
        expect(setMetaCommand).toHaveBeenCalledWith(
            {},
            { oref: "block:block-1", meta: { "frame:color": null, "frame:hue": null } },
        );
    });
});
