// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A profile's badge is its initial on its colour. A Black or White profile's badge would vanish on a menu of its own
// colour, and a white initial on white could not be read
// (docs/specs/SPEC_SHARED_PALETTE_12_HUES_BLACK_WHITE_2026_11_16.md §3.4).

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";

import { slotById } from "@/app/theme/palette";
import { ProfileBadge } from "./browser-profile-menu";

afterEach(() => cleanup());

const badge = (color: string) => render(() => <ProfileBadge name="Work" color={color} />).container.querySelector(".browser-profile-badge") as HTMLElement;

describe("ProfileBadge", () => {
    it("is the profile's initial on its colour", () => {
        const el = badge(slotById("teal")!.hex);
        expect(el.textContent).toBe("W");
        expect(el.style.backgroundColor).not.toBe("");
        expect(el.className).toBe("browser-profile-badge");
    });

    it("marks a Black and a White badge, so each gets a hairline and the white one a dark initial", () => {
        expect(badge("#000000").classList.contains("browser-profile-badge--black")).toBe(true);
        expect(badge("#ffffff").classList.contains("browser-profile-badge--white")).toBe(true);
        expect(badge("#FFFFFF").style.backgroundColor).toBe("rgb(255, 255, 255)");
    });

    it("does not mark a hue, an older colour or a colour it cannot read", () => {
        for (const color of ["#3b82f6", "#53a3f2", "not-a-colour"]) expect(badge(color).className, color).toBe("browser-profile-badge");
    });

    // A menu's icon slot mutes its glyph to 70%. A badge there is the profile's colour, not a glyph: at 70% a
    // White badge read light grey on a dark menu and a Black one dark grey on a light menu.
    it("is at full strength in a menu's icon slot", () => {
        const scss = readFileSync(resolve(process.cwd(), "frontend/app/view/browser/browser-view.scss"), "utf8");
        const rule = scss.match(/\.menu-item-icon:has\(> \.browser-profile-badge\)\s*\{([^}]*)\}/);
        expect(rule?.[1]).toMatch(/opacity:\s*1;/);
    });
});
