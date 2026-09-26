// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The built-in pane tab types, registered through the ONE pane-tab registry
// (`pane-tab-registry.ts`, Pane Tab contract v1 Phase 2 —
// docs/specs/SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md §3/§4). To add a view:
// register its manifest below — block.tsx, blockutil.tsx and
// pane-leaf-chrome.tsx read everything from the registry. Labels and icons
// reproduce the tables they replace exactly; a view with none there keeps
// the defaults (its name, a square).

import { armoryPaneTab } from "@/app/view/armory/armory";
import { browserPaneTab } from "@/app/view/browser/browser";
import { dronePaneTab } from "@/app/view/drone/drone";
import { editorPaneTab } from "@/app/view/editor/editor";
import { identityPaneTab } from "@/app/view/identity/identity-pane";
import { launcherPaneTab } from "@/app/view/launcher/launcher";
import { mediaPaneTab } from "@/app/view/media/media";
import { memoryPaneTab } from "@/app/view/bundle/bundle";
import { settingsPaneTab } from "@/app/view/settings/settings";
import { swarmPaneTab } from "@/app/view/swarm/swarm";
import { sysinfoPaneTab } from "@/app/view/sysinfo/sysinfo";
import { toolchainPaneTab } from "@/app/view/toolchain/toolchain";
import { wardenPaneTab } from "@/app/view/warden/warden";
import { helpPaneTab } from "@/view/helpview/helpview";
import { terminalPaneTab } from "@/view/term/term";
import { agentPaneTabManifest } from "@/app/view/agent/agent-manifest";
import { getPaneTab, registerPaneTab } from "./pane-tab-registry";

const builtins = [
    // Keep-alive (term, agent, browser, editor): remounting would lose real
    // state — the PTY/xterm instance, the parsed agent document, the page
    // (scroll, form input), the editor's cursor and undo. Agent per
    // SPEC_AGENT_PANE_TAB_KEEPALIVE_2026_09_18.md; browser and editor per the
    // repo owner's decision, SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md §5.
    terminalPaneTab, // native — Phase 2c (keep-alive)
    agentPaneTabManifest, // native — Phase 2c (keep-alive)
    browserPaneTab, // native — Phase 2c (keep-alive, native surface)
    editorPaneTab, // native — Phase 2c (keep-alive, zoom base 13)
    // Native (create(ctx)) — Phase 2c. "cpuplot" is the same view under an
    // older name.
    sysinfoPaneTab("sysinfo"),
    sysinfoPaneTab("cpuplot"),
    helpPaneTab, // native (create(ctx)) — the Phase 2b pilot
    launcherPaneTab, // native — Phase 2c (no header)
    swarmPaneTab, // native — Phase 2c
    memoryPaneTab, // native — Phase 2c
    mediaPaneTab, // native — Phase 2c
    identityPaneTab, // native — Phase 2c
    dronePaneTab, // native — Phase 2c (keeps the "workflows" alias)
    wardenPaneTab, // native — Phase 2c
    toolchainPaneTab, // native — Phase 2c
    armoryPaneTab, // native — Phase 2c (keeps the "trust" alias)
    settingsPaneTab, // native — Phase 2c
];
const unregisterBuiltins = builtins.map(registerPaneTab);
// A hot reload re-runs this module but not the registry; without this the
// re-registration would throw "already registered".
import.meta.hot?.dispose(() => unregisterBuiltins.forEach((unregister) => unregister()));

export function getBlockViewClass(viewType: string): ViewModelClass | undefined {
    return getPaneTab(viewType)?.viewModelClass;
}
