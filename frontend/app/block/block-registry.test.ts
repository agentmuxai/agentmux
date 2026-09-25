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
import { getPaneTab, isKeepAliveView, paneTabCapability, resolvePaneTabView } from "./pane-tab-registry";

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

// Deliberate differences since the tables: "cpuplot" is sysinfo under an older
// name, and as a native tab (Phase 2c) it names and icons itself like sysinfo
// — its header always showed sysinfo's chart icon; only its tab pill didn't.
const NEW_ICONS: Record<string, string> = { cpuplot: "chart-line" };
const NEW_LABELS: Record<string, string> = { cpuplot: "Sysinfo" };

describe("built-in pane tabs (block-registry.ts)", () => {
    // Native views (create(ctx), Phase 2b) have no ViewModel class.
    const NATIVE = ["help", "sysinfo", "cpuplot", "swarm", "drone"];

    it("registers an instance factory for every view the old map had", () => {
        for (const view of VIEWS) {
            const m = getPaneTab(view)!;
            if (NATIVE.includes(view)) {
                expect(m.create, view).toBeTypeOf("function");
                expect(getBlockViewClass(view), view).toBeUndefined();
            } else {
                expect(getBlockViewClass(view), view).toBeTypeOf("function");
            }
        }
        expect(getPaneTab("cpuplot")?.capabilities).toEqual(getPaneTab("sysinfo")?.capabilities);
    });

    it("keeps every view's old icon and label", () => {
        for (const view of VIEWS) {
            expect(blockViewToIcon(view), view).toBe(NEW_ICONS[view] ?? OLD_ICONS[view] ?? "square");
            expect(blockViewToName(view), view).toBe(NEW_LABELS[view] ?? OLD_LABELS[view] ?? view);
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

    // Phase 5: each capability replaces a view-name check in shared code, so
    // exactly the view types those checks named declare it.
    it("declares exactly the capabilities the old view-name checks encoded", () => {
        const holders = (key: string) => VIEWS.filter((v) => paneTabCapability(v, key as any) != null).sort();
        expect(holders("header")).toEqual(["agent"]);
        expect(paneTabCapability("agent", "header")).toBe("surface");
        expect(holders("headerMic")).toEqual(["term"]);
        expect(paneTabCapability("term", "headerMic")?.title).toBe("Speak into this terminal (Ctrl+Shift+V)");
        expect(holders("statsBadgeSetting")).toEqual(["term"]);
        expect(paneTabCapability("term", "statsBadgeSetting")).toBe("term:showstatsbadge");
        expect(holders("hueBorder")).toEqual(["term"]);
        expect(holders("nativeSurface")).toEqual(["browser"]);
        // 5b: zoom.ts's allowlist and editor's base size, paste, Ctrl+F, cwd.
        expect(holders("paneZoom")).toEqual(["agent", "armory", "editor", "swarm", "term", "warden"]);
        expect(paneTabCapability("editor", "paneZoom")?.baseFontSize).toBe(13);
        expect(paneTabCapability("term", "paneZoom")?.baseFontSize).toBeUndefined();
        expect(holders("acceptsInput")).toEqual(["term"]);
        expect(holders("shellKeys")).toEqual(["term"]);
        expect(holders("sharesCwd")).toEqual(["term"]);
        expect(holders("splitDropsMeta")).toEqual(["agent"]);
        expect(paneTabCapability("agent", "splitDropsMeta")).toEqual(
            expect.arrayContaining(["agentId", "agentName", "agentProvider", "cmd", "cmd:args"])
        );
        // An alias carries its view's capabilities.
        expect(paneTabCapability("forge", "header")).toBe("surface");
    });

    it("carries the term and agent tab descriptors", () => {
        expect(getPaneTab("term")?.tab?.label).toBeTypeOf("function");
        expect(getPaneTab("agent")?.tab?.label).toBeTypeOf("function");
    });
});
