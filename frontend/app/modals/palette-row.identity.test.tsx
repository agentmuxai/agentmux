// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// An agent's palette row carries its colour: the same identity tone its pane's ring and its Swarm row draw, in the
// theme the window is in. A row with no colour has no mark.

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ light: false }));
vi.mock("@/app/block/pane-identity", () => ({ isLightThemeActive: () => h.light }));
vi.mock("@/app/keybindings", () => ({ shortcutFor: () => "" }));

import { paneRoleColor } from "@/app/block/pane-color-scheme";
import type { CommandEntry } from "@/app/store/command-registry";
import { PaletteRow } from "./palette-row";

const STORED = "#4f8cff";
const entry = (identity?: string, over: Partial<CommandEntry> = {}): CommandEntry => ({
    id: "agent:a",
    label: "Camper",
    category: "Agent",
    icon: "robot",
    identity,
    ...over,
    execute: () => {},
});

function renderRow(identity?: string, over: Partial<CommandEntry> = {}): HTMLElement {
    const { container } = render(() => (
        <PaletteRow
            row={{ cmd: entry(identity, over), index: 0, divider: false }}
            id="r0"
            selected={false}
            groupColumn={false}
            onPick={() => {}}
        />
    ));
    return container.querySelector<HTMLElement>(".command-palette-item")!;
}

afterEach(() => {
    cleanup();
    h.light = false;
});

describe("an agent's palette row", () => {
    it("is marked in the agent's identity tone for a dark theme", () => {
        const row = renderRow(STORED);
        expect(row.classList.contains("has-identity")).toBe(true);
        expect(row.style.getPropertyValue("--palette-identity")).toBe(paneRoleColor(undefined, STORED, false, "identity"));
    });

    it("takes the light theme's tone of the same colour in a light theme", () => {
        h.light = true;
        const row = renderRow(STORED);
        const light = paneRoleColor(undefined, STORED, true, "identity");
        expect(row.style.getPropertyValue("--palette-identity")).toBe(light);
        expect(light).not.toBe(paneRoleColor(undefined, STORED, false, "identity"));
    });

    it("has no mark when the agent has no colour yet, and still shows its name", () => {
        const row = renderRow(undefined);
        expect(row.classList.contains("has-identity")).toBe(false);
        expect(row.style.getPropertyValue("--palette-identity")).toBe("");
        expect(row.textContent).toContain("Camper");
    });
});

describe("an agent's icon", () => {
    it("is the icon-font glyph for a font name, and the text itself for a provider glyph", () => {
        const named = renderRow(STORED);
        expect(named.querySelector("i.fa-robot")).not.toBeNull();
        cleanup();
        const { container } = render(() => (
            <PaletteRow
                row={{ cmd: { ...entry(STORED), icon: "✦" }, index: 0, divider: false }}
                id="r1"
                selected={false}
                groupColumn={false}
                onPick={() => {}}
            />
        ));
        const icon = container.querySelector(".command-palette-item-icon")!;
        expect(icon.tagName).toBe("SPAN");
        expect(icon.textContent).toBe("✦");
        expect(container.querySelector("i.fa-solid")).toBeNull();
    });
});

describe("an agent's palette row icon", () => {
    it("draws the provider's logo, not a text glyph, when the agent has no icon of its own", () => {
        const row = renderRow(STORED, { icon: undefined, provider: "claude" });
        expect(row.querySelector(".command-palette-item-logo .provider-logo")).not.toBeNull();
        expect(row.querySelector("i.command-palette-item-icon")).toBeNull();
        expect(row.textContent).not.toContain("✖");
    });

    it("draws an icon-font icon the agent has, and no logo", () => {
        const row = renderRow(STORED, { icon: "shield", provider: "claude" });
        expect(row.querySelector("i.fa-shield")).not.toBeNull();
        expect(row.querySelector(".command-palette-item-logo")).toBeNull();
    });
});
