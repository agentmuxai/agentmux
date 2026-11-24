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
const entry = (identity?: string): CommandEntry => ({
    id: "agent:a",
    label: "Camper",
    category: "Agent",
    icon: "robot",
    identity,
    execute: () => {},
});

function renderRow(identity?: string): HTMLElement {
    const { container } = render(() => (
        <PaletteRow
            row={{ cmd: entry(identity), index: 0, divider: false }}
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
