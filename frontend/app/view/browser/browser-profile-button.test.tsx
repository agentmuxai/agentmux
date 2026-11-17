// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The toolbar's Profile button is a glyph in the profile's colour, drawn straight on the toolbar. A Black
// profile's would vanish on a dark toolbar and a White one's on a light toolbar, so those two take the neutral's
// line tone (docs/specs/SPEC_SHARED_PALETTE_12_HUES_BLACK_WHITE_2026_11_16.md §3.4).

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("./browser-profiles", async (original) => ({
    ...(await original<typeof import("./browser-profiles")>()),
    followBrowserProfiles: () => {},
    loadBrowserProfiles: async () => {},
    userProfiles: () => [],
    profileById: (id: string) =>
        ({
            work: { id: "work", name: "Work", color: "#3b82f6" },
            night: { id: "night", name: "Night", color: "#000000" },
            paper: { id: "paper", name: "Paper", color: "#ffffff" },
        })[id],
}));

import { paneRoleColor } from "@/app/block/pane-color-scheme";
import type { BrowserViewModel } from "./browser-model";
import { BrowserProfileButton } from "./browser-profile-menu";

afterEach(() => cleanup());

/** A colour as the DOM reports it back. */
const asStyle = (color: string): string => {
    const probe = document.createElement("i");
    probe.style.color = color;
    return probe.style.color;
};

const button = (identity?: string): HTMLElement => {
    const model = {
        blockId: "b1",
        meta: () => (identity ? { "browser:identity": identity } : {}),
        urlAtom: () => "https://example.com/",
    } as unknown as BrowserViewModel;
    const { container } = render(() => <BrowserProfileButton model={model} home={() => "https://example.com/"} />);
    return container.querySelector<HTMLElement>(".browser-profile-btn")!;
};

describe("the toolbar's Profile button", () => {
    it("is drawn in a hue profile's own colour", () => {
        expect(button("profile:work").style.color).toBe(asStyle("#3b82f6"));
    });

    it("is drawn in the neutral's line tone for a Black and a White profile, never plain black or white", () => {
        const night = button("profile:night").style.color;
        expect(night).toBe(asStyle(paneRoleColor(undefined, "black", false, "identity")!));
        expect(night).not.toBe(asStyle("#000000"));
        cleanup();
        const paper = button("profile:paper").style.color;
        expect(paper).toBe(asStyle(paneRoleColor(undefined, "white", false, "identity")!));
    });

    it("takes the toolbar's own colour for Personal", () => {
        expect(button().style.color).toBe("");
    });
});
