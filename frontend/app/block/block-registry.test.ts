// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Parity guard for Pane Tab contract Phase 2: the built-in manifests must
 * reproduce exactly what the tables they replaced said — blockutil.tsx's
 * icons and VIEW_LABELS, block.tsx's migration aliases, pane-leaf-chrome.tsx's
 * KEEP_ALIVE_TYPES and block-registry.ts's view → class map.
 */

import { describe, expect, it } from "vitest";
import "./block-registry";
import { blockViewToIcon, blockViewToName } from "./blockutil";
import { PANE_HUE_OPTIONS } from "./pane-color-menu";
import { widgetHueFor } from "./pane-identity";
import { getPaneTab, isKeepAliveView, paneTabCapability, resolvePaneTabView } from "./pane-tab-registry";

const VIEWS = [
    "term", "cpuplot", "sysinfo", "help", "launcher", "agent", "swarm", "editor", "browser",
    "memory", "media", "identity", "drone", "warden", "toolchain", "armory", "settings",
    "connectors",
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
// Warden and Armory likewise always showed their own icon in the header
// (`shield-halved`, `vault`) but not in their pills; Armory had no label.
// So did the last five legacy views, whose pills fell back to "square" and
// three of them to their lowercase view name.
const NEW_ICONS: Record<string, string> = {
    cpuplot: "chart-line", warden: "shield-halved", armory: "vault", connectors: "plug",
    launcher: "shapes", memory: "brain", identity: "user", toolchain: "wrench", settings: "cog",
};
const NEW_LABELS: Record<string, string> = {
    cpuplot: "Sysinfo", armory: "Armory", connectors: "Connectors", launcher: "Launcher", toolchain: "Toolchain", settings: "Settings",
};

describe("built-in pane tabs (block-registry.ts)", () => {
    it("registers an instance factory for every view the old map had", () => {
        for (const view of VIEWS) {
            expect(getPaneTab(view)!.create, view).toBeTypeOf("function");
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
        expect(resolvePaneTabView("knowledge")).toBe("memory");
    });

    // Phase 5: each capability replaces a view-name check in shared code, so
    // exactly the view types those checks named declare it.
    it("declares exactly the capabilities the old view-name checks encoded", () => {
        const holders = (key: string) => VIEWS.filter((v) => paneTabCapability(v, key as any) != null).sort();
        expect(holders("header")).toEqual(["agent", "launcher"]);
        expect(paneTabCapability("agent", "header")).toBe("surface");
        // The launcher's old `noHeader`.
        expect(paneTabCapability("launcher", "header")).toBe("none");
        expect(holders("headerMic")).toEqual(["term"]);
        expect(paneTabCapability("term", "headerMic")?.title).toBe("Speak into this terminal");
        expect(holders("statsBadgeSetting")).toEqual(["term"]);
        expect(paneTabCapability("term", "statsBadgeSetting")).toBe("term:showstatsbadge");
        expect(holders("hueBorder")).toEqual(["term"]);
        expect(holders("nativeSurface")).toEqual(["browser"]);
        // 5b: zoom.ts's allowlist and editor's base size, paste, Ctrl+F, cwd.
        // The Armory's zoom went to the two panes that replaced it; the Armory
        // shim renders one of them until its block remounts.
        expect(holders("paneZoom")).toEqual(["agent", "armory", "connectors", "editor", "memory", "swarm", "term", "warden"]);
        expect(paneTabCapability("editor", "paneZoom")?.baseFontSize).toBe(13);
        expect(paneTabCapability("files", "paneZoom")?.baseFontSize).toBe(12);
        expect(paneTabCapability("term", "paneZoom")?.baseFontSize).toBeUndefined();
        expect(holders("acceptsInput")).toEqual(["term"]);
        expect(holders("shellKeys")).toEqual(["term"]);
        expect(holders("sharesCwd")).toEqual(["term"]);
        expect(holders("splitBlockDef")).toEqual(["agent"]);
        // An alias carries its view's capabilities.
        expect(paneTabCapability("forge", "header")).toBe("surface");
    });

    it("carries the term and agent tab descriptors", () => {
        expect(getPaneTab("term")?.tab?.label).toBeTypeOf("function");
        expect(getPaneTab("agent")?.tab?.label).toBeTypeOf("function");
    });
});

// SPEC_WIDGET_DEFAULT_PANE_COLORS_2026_10_05.md §3.3: every built-in widget has
// a color, and it is one of the Pane Color swatches, so the user can pick it
// back after trying another.
describe("built-in widget colors", () => {
    const swatches = new Set(PANE_HUE_OPTIONS.map((o) => o.hue));
    // Through widgetHueFor, so a legacy view (cpuplot, armory) is checked by
    // the color it takes from the view it stands in for.
    it.each([...VIEWS, "files", "remotes"])("%s has a default hue from the Pane Color palette", (view) => {
        expect(swatches.has(widgetHueFor(view) as number)).toBe(true);
    });

    it("gives cpuplot Sysinfo's color", () => {
        expect(getPaneTab("cpuplot")?.legacyOf).toBe("sysinfo");
        expect(widgetHueFor("cpuplot")).toBe(widgetHueFor("sysinfo"));
    });

    it("gives the most used widgets distinct colors", () => {
        const common = ["term", "agent", "browser", "editor", "files", "sysinfo", "swarm", "media", "help", "launcher", "remotes", "memory"];
        expect(new Set(common.map((v) => getPaneTab(v)?.defaultHue)).size).toBe(common.length);
    });
});
