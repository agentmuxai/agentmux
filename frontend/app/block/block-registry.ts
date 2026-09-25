// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// View-type → ViewModel class registry.
// To add a new block view: add its class to the map below — block.tsx never
// needs to change. A runtime registration API (for user-installed pane-tab
// widgets) is planned: SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md §3/§4.

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
import { HelpViewModel } from "@/view/helpview/helpview";
import { TermViewModel } from "@/view/term/term";

const blockViewRegistry = new Map<string, ViewModelClass>();

blockViewRegistry.set("term", TermViewModel as any);
blockViewRegistry.set("cpuplot", SysinfoViewModel as any);
blockViewRegistry.set("sysinfo", SysinfoViewModel as any);
blockViewRegistry.set("help", HelpViewModel as any);
blockViewRegistry.set("launcher", LauncherViewModel as any);
blockViewRegistry.set("agent", AgentViewModel as any);
blockViewRegistry.set("swarm", SwarmViewModel as any);
blockViewRegistry.set("editor", EditorViewModel as any);
blockViewRegistry.set("browser", BrowserViewModel as any);
blockViewRegistry.set("memory", BundleViewModel as any);
blockViewRegistry.set("media", MediaViewModel as any);
blockViewRegistry.set("identity", IdentityPaneViewModel as any);
blockViewRegistry.set("drone", DroneViewModel as any);
blockViewRegistry.set("warden", WardenViewModel as any);
blockViewRegistry.set("toolchain", ToolchainViewModel as any);
blockViewRegistry.set("armory", ArmoryViewModel as any);
blockViewRegistry.set("settings", SettingsViewModel as any);

export function getBlockViewClass(viewType: string): ViewModelClass | undefined {
    return blockViewRegistry.get(viewType);
}
