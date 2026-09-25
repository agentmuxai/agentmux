// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Parity guard for Pane Tab contract Phase 2: the built-in manifests must
 * reproduce exactly what the tables they replaced said — blockutil.tsx's
 * icons and VIEW_LABELS, block.tsx's migration aliases, pane-leaf-chrome.tsx's
 * KEEP_ALIVE_TYPES and block-registry.ts's view → class map.
 */

import { describe, expect, it } from "vitest";
import { getBlockViewClass } from "./block-registry";
import { blockViewToIcon, blockViewToName } from "./blockutil";
import { getPaneTab, isKeepAliveView, resolvePaneTabView } from "./pane-tab-registry";

const VIEWS = [
    "term", "cpuplot", "sysinfo", "help", "launcher", "agent", "swarm", "editor", "browser",
    "memory", "media", "identity", "drone", "warden", "toolchain", "armory", "settings",
];

const OLD_ICONS: Record<string, string> = {
    term: "terminal", agent: "sparkles", browser: "globe", sysinfo: "chart-line", editor: "file-lines",
    help: "circle-question", swarm: "diagram-project", drone: "diagram-project", media: "photo-film",
};
const OLD_LABELS: Record<string, string> = {
    agent: "Agent", browser: "Browser", drone: "Drone", editor: "Editor", help: "Help", identity: "Identity",
    media: "Media", memory: "Memory", swarm: "Swarm", sysinfo: "Sysinfo", term: "Terminal", warden: "Warden",
};

describe("built-in pane tabs (block-registry.ts)", () => {
    it("registers a ViewModel class for every view the old map had", () => {
        for (const view of VIEWS) expect(getBlockViewClass(view), view).toBeTypeOf("function");
        expect(getBlockViewClass("cpuplot")).toBe(getBlockViewClass("sysinfo"));
    });

    it("keeps every view's old icon and label", () => {
        for (const view of VIEWS) {
            expect(blockViewToIcon(view), view).toBe(OLD_ICONS[view] ?? "square");
            expect(blockViewToName(view), view).toBe(OLD_LABELS[view] ?? view);
        }
        expect(blockViewToName("")).toBe("(No View)");
    });

    it("keeps exactly the old keep-alive set", () => {
        expect(VIEWS.filter(isKeepAliveView).sort()).toEqual(["agent", "browser", "editor", "term"]);
    });

    it("keeps the old migration aliases", () => {
        expect(resolvePaneTabView("forge")).toBe("agent");
        expect(resolvePaneTabView("workflows")).toBe("drone");
        expect(resolvePaneTabView("trust")).toBe("armory");
    });

    it("carries the term and agent tab descriptors", () => {
        expect(getPaneTab("term")?.tab?.label).toBeTypeOf("function");
        expect(getPaneTab("agent")?.tab?.label).toBeTypeOf("function");
    });
});
