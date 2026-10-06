// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const setMetaCommand = vi.fn().mockResolvedValue(undefined);
const setAgentContentCommand = vi.fn().mockResolvedValue(undefined);

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        SetMetaCommand: (...args: unknown[]) => setMetaCommand(...args),
        SetAgentContentCommand: (...args: unknown[]) => setAgentContentCommand(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/global", () => ({
    MOS: { makeORef: (t: string, id: string) => `${t}:${id}` },
}));

import { hueToActiveBorder, hueToAgentIdentityColor, setHue } from "./pane-color-menu";
import { contrastRatio, paneRoleColor } from "./pane-color-scheme";

describe("hueToAgentIdentityColor", () => {
    it("matches hsl(hue, 65%, 52%) — the same values hueToActiveBorder uses — converted to hex", () => {
        // Hand-verified against the standard HSL->RGB formula (not just
        // re-deriving the same computation the implementation does), so the
        // identity color persisted for an agent is the exact color the user
        // saw when picking it (hueToActiveBorder), not an approximation.
        expect(hueToAgentIdentityColor(0)).toBe("#d43535"); // red
        expect(hueToAgentIdentityColor(120)).toBe("#35d435"); // green
        expect(hueToAgentIdentityColor(240)).toBe("#3535d4"); // blue
    });

    it("always produces a strict #rrggbb string valid per agent-color.ts::isValidAgentColor", () => {
        for (const hue of [0, 30, 60, 90, 120, 150, 180, 210, 240, 270, 300, 330, 359]) {
            expect(hueToAgentIdentityColor(hue)).toMatch(/^#[0-9a-f]{6}$/);
        }
    });
});

// 2026-10-02 (REPORT_PANE_TAB_COLOR_BEST_PRACTICES_2026_10_02.md §6 P3): the
// header is a subtle OKLCH tint of the identity in BOTH themes — no longer
// hsl(h, 28%, 16%) on dark and the identity at full strength on light.
describe("paneRoleColor headerTint — dark theme (default)", () => {
    it("uses the explicit hue pick when one is present — regardless of any identity hex", () => {
        expect(paneRoleColor(120, "#3535d4", false, "headerTint")).toBe(paneRoleColor(120, undefined, false, "headerTint"));
    });

    it("gives an agent's persisted identity hex the same header as an explicit pick of that hue — one system for both sources", () => {
        for (const hue of [0, 120, 240]) {
            expect(paneRoleColor(undefined, hueToAgentIdentityColor(hue), false, "headerTint")).toBe(
                paneRoleColor(hue, undefined, false, "headerTint"),
            );
        }
    });

    it("is a subtle tint at the neutral header's lightness, not a saturated fill", () => {
        // The neutral dark header is hsl(220, 12%, 16%) = #24272e.
        expect(contrastRatio(paneRoleColor(0, undefined, false, "headerTint")!, "#24272e")).toBeLessThan(1.15);
    });

    it("returns undefined when there is no color source at all", () => {
        expect(paneRoleColor(undefined, undefined, false, "headerTint")).toBeUndefined();
    });
});

describe("paneRoleColor headerTint — light theme", () => {
    it("is a subtle tint near white, not the identity at full strength (the old light-theme header)", () => {
        const header = paneRoleColor(120, undefined, true, "headerTint")!;
        expect(header).not.toBe(hueToActiveBorder(120));
        expect(contrastRatio(header, "#ffffff")).toBeLessThan(1.2);
    });

    it("gives an identity hex the same header as the matching explicit hue", () => {
        expect(paneRoleColor(undefined, hueToAgentIdentityColor(240), true, "headerTint")).toBe(
            paneRoleColor(240, undefined, true, "headerTint"),
        );
    });

    it("returns undefined when there is no color source at all, same as the dark-theme case", () => {
        expect(paneRoleColor(undefined, undefined, true, "headerTint")).toBeUndefined();
    });
});

describe("setHue", () => {
    beforeEach(() => {
        setMetaCommand.mockClear();
        setAgentContentCommand.mockClear();
    });

    it("always writes frame:hue via SetMetaCommand", () => {
        setHue("block-1", 120);
        expect(setMetaCommand).toHaveBeenCalledWith(
            {},
            { oref: "block:block-1", meta: { "frame:hue": 120 } },
        );
    });

    it("persists to the agent's identity color when a hue AND an agentId are given", () => {
        setHue("block-1", 120, "agent-42");
        expect(setAgentContentCommand).toHaveBeenCalledWith(
            {},
            { agent_id: "agent-42", content_type: "ui:color", content: hueToAgentIdentityColor(120) },
        );
    });

    it("does NOT touch agent identity when no agentId is given (non-agent pane)", () => {
        setHue("block-1", 120);
        expect(setAgentContentCommand).not.toHaveBeenCalled();
    });

    it("does NOT touch agent identity on 'Default' (hue: null), even with an agentId — clearing one pane's color must not reset the agent's color everywhere else", () => {
        setHue("block-1", null, "agent-42");
        expect(setMetaCommand).toHaveBeenCalledWith(
            {},
            { oref: "block:block-1", meta: { "frame:hue": null } },
        );
        expect(setAgentContentCommand).not.toHaveBeenCalled();
    });
});
