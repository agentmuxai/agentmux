// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * An agent's colour on the Kanban is its name's text colour, with no square beside it, and that
 * colour stays readable on every theme's surface. The colour is the theme's `identity` variant of
 * the agent's hue (pane-color-scheme.ts), the one made for text, so a hue too faint to read as text
 * in one theme is already lifted or darkened there.
 */
import { cleanup, render } from "@solidjs/testing-library";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { ListAgentDefinitionsCommand: vi.fn().mockResolvedValue([]) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));

import { contrastRatio, paneRoleColor } from "@/app/block/pane-color-scheme";
import { LIGHT_THEME_IDS } from "@/app/menu/base-menus";
import { AgentChip } from "./kanban-card";
import type { KanbanAgent } from "./kanban-types";

const agent = (color?: string): KanbanAgent => ({ name: "Naki-2", block_id: "", state: "working", color });

describe("an agent's name in its colour", () => {
    afterEach(() => cleanup());

    it("colours the name text and draws no square", () => {
        const { container } = render(() => <AgentChip name="Naki-2" agent={agent("#3b82f6")} />);
        const name = container.querySelector<HTMLElement>(".kanban-agent-chip__name")!;
        expect(name.textContent?.trim()).toBe("Naki-2");
        expect(name.style.getPropertyValue("--kanban-agent-text")).toMatch(/^#[0-9a-f]{6}$/);
        expect(container.querySelector(".kanban-agent-chip__swatch")).toBeNull();
    });

    it("colours a name from the colour srv keeps for an agent that isn't running here", () => {
        const { container } = render(() => (
            <AgentChip name="Camper" uid="u1" colors={{ u1: "#16a34a" }} agent={null} />
        ));
        const name = container.querySelector<HTMLElement>(".kanban-agent-chip__name")!;
        expect(name.style.getPropertyValue("--kanban-agent-text")).toMatch(/^#[0-9a-f]{6}$/);
    });

    it("leaves an agent with no colour in the normal text colour", () => {
        const { container } = render(() => <AgentChip name="Free" agent={agent()} />);
        const name = container.querySelector<HTMLElement>(".kanban-agent-chip__name")!;
        expect(name.getAttribute("style") ?? "").not.toContain("--kanban-agent-text");
        expect(name.textContent?.trim()).toBe("Free");
    });
});

describe("the name colour is readable on every theme", () => {
    const themes = join(__dirname, "..", "..", "themes");
    const ids = [
        "catppuccin-latte",
        "catppuccin",
        "dracula",
        "gruvbox-light",
        "gruvbox",
        "high-contrast",
        "light",
        "midnight",
        "monokai",
        "nord",
        "solarized-light",
        "tokyo-night",
    ];
    /** Text at the chip's size (`--text-xs`): WCAG 1.4.3 AA. */
    const MIN_CONTRAST = 4.5;

    it.each(ids)("%s: every hue reads against the theme's surface", (id) => {
        const m = readFileSync(join(themes, `${id}.scss`), "utf8").match(
            /--main-bg-color:\s*rgba\((\d+),\s*(\d+),\s*(\d+)/
        );
        expect(m, `${id} sets --main-bg-color`).not.toBeNull();
        const surface = "#" + [1, 2, 3].map((i) => Number(m![i]).toString(16).padStart(2, "0")).join("");
        let worst = { ratio: Infinity, hue: 0 };
        for (let hue = 0; hue < 360; hue += 5) {
            const colour = paneRoleColor(hue, undefined, LIGHT_THEME_IDS.has(id), "identity")!;
            const ratio = contrastRatio(colour, surface);
            if (ratio < worst.ratio) worst = { ratio, hue };
        }
        expect(worst.ratio, `${id} (${surface}) is lowest at hue ${worst.hue}`).toBeGreaterThanOrEqual(MIN_CONTRAST);
    });
});
