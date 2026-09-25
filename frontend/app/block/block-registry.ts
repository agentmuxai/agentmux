// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The built-in pane tab types, registered through the ONE pane-tab registry
// (`pane-tab-registry.ts`, Pane Tab contract v1 Phase 2 —
// docs/specs/SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md §3/§4). To add a view:
// register its manifest below — block.tsx, blockutil.tsx and
// pane-leaf-chrome.tsx read everything from the registry. Labels and icons
// reproduce the tables they replace exactly; a view with none there keeps
// the defaults (its name, a square).

import { AgentViewModel } from "@/app/view/agent";
import { ArmoryViewModel } from "@/app/view/armory/armory";
import { BrowserViewModel } from "@/app/view/browser/browser";
import { DroneViewModel } from "@/app/view/drone/drone";
import { EditorViewModel } from "@/app/view/editor/editor";
import { IdentityPaneViewModel } from "@/app/view/identity/identity-pane";
import { LauncherViewModel } from "@/app/view/launcher/launcher";
import { MediaViewModel } from "@/app/view/media/media";
import { BundleViewModel } from "@/app/view/bundle/bundle";
import { SettingsViewModel } from "@/app/view/settings/settings";
import { SwarmViewModel } from "@/app/view/swarm/swarm";
import { SysinfoViewModel } from "@/app/view/sysinfo/sysinfo";
import { ToolchainViewModel } from "@/app/view/toolchain/toolchain";
import { WardenViewModel } from "@/app/view/warden/warden";
import { helpPaneTab } from "@/view/helpview/helpview";
import { TermViewModel } from "@/view/term/term";
import { AGENT_SPLIT_DROPPED_META, agentPaneTab } from "@/app/view/agent/agent-pane-tab";
import { buildAgentPaneChromeModel } from "@/app/view/agent/agent-view";
import { buildTermPaneChromeModel } from "@/view/term/term";
import { termPaneTab } from "@/view/term/term-pane-tab";
import { getPaneTab, legacyAdapter, registerPaneTab } from "./pane-tab-registry";

const builtins = [
    // Keep-alive (term, agent, browser, editor): remounting would lose real
    // state — the PTY/xterm instance, the parsed agent document, the page
    // (scroll, form input), the editor's cursor and undo. Agent per
    // SPEC_AGENT_PANE_TAB_KEEPALIVE_2026_09_18.md; browser and editor per the
    // repo owner's decision, SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md §5.
    legacyAdapter("term", TermViewModel as any, {
        label: "Terminal",
        icon: "terminal",
        lifecycle: "keepAlive",
        capabilities: {
            headerMic: { title: "Speak into this terminal (Ctrl+Shift+V)" },
            statsBadgeSetting: "term:showstatsbadge",
            hueBorder: true,
            paneZoom: {},
            acceptsInput: true,
            shellKeys: true,
            sharesCwd: true,
        },
        tab: termPaneTab,
        chrome: buildTermPaneChromeModel,
    }),
    // "forge" was folded into the agent pane in v0.33.197.
    legacyAdapter("agent", AgentViewModel as any, {
        label: "Agent",
        icon: "sparkles",
        aliases: ["forge"],
        lifecycle: "keepAlive",
        capabilities: { header: "surface", paneZoom: {}, splitDropsMeta: AGENT_SPLIT_DROPPED_META },
        tab: agentPaneTab,
        chrome: buildAgentPaneChromeModel,
    }),
    legacyAdapter("browser", BrowserViewModel as any, {
        label: "Browser",
        icon: "globe",
        lifecycle: "keepAlive",
        capabilities: { nativeSurface: true },
    }),
    legacyAdapter("editor", EditorViewModel as any, {
        label: "Editor",
        icon: "file-lines",
        lifecycle: "keepAlive",
        capabilities: { paneZoom: { baseFontSize: 13 } },
    }),
    legacyAdapter("sysinfo", SysinfoViewModel as any, { label: "Sysinfo", icon: "chart-line" }),
    legacyAdapter("cpuplot", SysinfoViewModel as any),
    helpPaneTab, // native (create(ctx)) — the Phase 2b pilot
    legacyAdapter("launcher", LauncherViewModel as any),
    // Swarm, Armory and Warden apply `term:zoom` as CSS zoom.
    legacyAdapter("swarm", SwarmViewModel as any, { label: "Swarm", icon: "diagram-project", capabilities: { paneZoom: {} } }),
    legacyAdapter("memory", BundleViewModel as any, { label: "Memory" }),
    legacyAdapter("media", MediaViewModel as any, { label: "Media", icon: "photo-film" }),
    legacyAdapter("identity", IdentityPaneViewModel as any, { label: "Identity" }),
    // Workflows was renamed to Drone (SPEC_RENAME_WORKFLOWS_TO_DRONE_2026_05_18);
    // persisted blocks still say "workflows".
    legacyAdapter("drone", DroneViewModel as any, { label: "Drone", icon: "diagram-project", aliases: ["workflows"] }),
    legacyAdapter("warden", WardenViewModel as any, { label: "Warden", capabilities: { paneZoom: {} } }),
    legacyAdapter("toolchain", ToolchainViewModel as any),
    // The Trust Center was renamed to Armory
    // (docs/specs/archive/SPEC_RENAME_TRUST_CENTER_TO_ARMORY_2026_07_02.md);
    // persisted blocks still say "trust".
    legacyAdapter("armory", ArmoryViewModel as any, { aliases: ["trust"], capabilities: { paneZoom: {} } }),
    legacyAdapter("settings", SettingsViewModel as any),
];
const unregisterBuiltins = builtins.map(registerPaneTab);
// A hot reload re-runs this module but not the registry; without this the
// re-registration would throw "already registered".
import.meta.hot?.dispose(() => unregisterBuiltins.forEach((unregister) => unregister()));

export function getBlockViewClass(viewType: string): ViewModelClass | undefined {
    return getPaneTab(viewType)?.viewModelClass;
}
