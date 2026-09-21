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

import { headerBgForEffectiveColor, hueToActiveBorder, hueToAgentIdentityColor, hueToHeaderBg, setHue } from "./pane-color-menu";

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

describe("headerBgForEffectiveColor — dark theme (default)", () => {
    it("uses the explicit hue pick, darkened, when one is present — regardless of any identity hex", () => {
        expect(headerBgForEffectiveColor(120, "#3535d4", false)).toBe(hueToHeaderBg(120));
    });

    it("derives the SAME darkened header treatment from an agent's persisted identity hex when no explicit hue is set — the whole point of unifying the two sources (2026-09-21)", () => {
        // "#35d435" is exactly hueToAgentIdentityColor(120)'s output — round
        // tripping it back through hexToHue must recover hue 120, so this
        // agent's header gets the identical darkened background an explicit
        // pick of hue 120 would have produced.
        expect(headerBgForEffectiveColor(undefined, "#35d435", false)).toBe(hueToHeaderBg(120));
        expect(headerBgForEffectiveColor(undefined, "#d43535", false)).toBe(hueToHeaderBg(0)); // red
        expect(headerBgForEffectiveColor(undefined, "#3535d4", false)).toBe(hueToHeaderBg(240)); // blue
    });

    it("returns undefined when there is no color source at all", () => {
        expect(headerBgForEffectiveColor(undefined, undefined, false)).toBeUndefined();
    });
});

describe("headerBgForEffectiveColor — light theme (user request 2026-09-21)", () => {
    it("uses the full-strength hueToActiveBorder color for an explicit hue pick — matching the border instead of darkening", () => {
        expect(headerBgForEffectiveColor(120, "#3535d4", true)).toBe(hueToActiveBorder(120));
    });

    it("uses the agent identity hex directly, at full strength — the pre-2026-09-21 'bright' behavior — instead of darkening it", () => {
        expect(headerBgForEffectiveColor(undefined, "#3b82f6", true)).toBe("#3b82f6");
    });

    it("returns undefined when there is no color source at all, same as the dark-theme case", () => {
        expect(headerBgForEffectiveColor(undefined, undefined, true)).toBeUndefined();
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
