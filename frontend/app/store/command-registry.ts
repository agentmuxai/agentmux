// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Global command registry — the single source of truth for all palette commands.
// Both the Ctrl+P UI and the `run_command` IPC endpoint use this registry.

import {
    atoms,
    createBlock,
    createBlockSplitHorizontally,
    createBlockSplitVertically,
    createTab,
    getApi,
    openOrFocusPaneByView,
    setActiveTab,
} from "@/app/store/global";
import { WorkspaceService } from "@/app/store/services";
import { getLayoutModelForStaticTab, NavigateDirection } from "@/layout/index";
import { invokeCommand } from "@/app/platform/ipc";
import { fireAndForget } from "@/util/util";
import { openModal } from "@/app/store/modalmodel";
import { CommandPaletteModal } from "@/app/modals/command-palette";
import { zoomIn, zoomOut, zoomReset } from "@/app/store/zoom";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface CommandEntry {
    id: string;
    label: string;
    category: string;
    icon?: string;
    iconColor?: string;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    execute: () => void | Promise<any>;
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

class CommandRegistry {
    private commands = new Map<string, CommandEntry>();

    register(entry: CommandEntry): void {
        this.commands.set(entry.id, entry);
    }

    get(id: string): CommandEntry | undefined {
        return this.commands.get(id);
    }

    all(): CommandEntry[] {
        return Array.from(this.commands.values());
    }

    run(id: string): boolean {
        const cmd = this.commands.get(id);
        if (!cmd) return false;
        fireAndForget(async () => {
            await cmd.execute();
        });
        return true;
    }
}

export const commandRegistry = new CommandRegistry();

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function getFocusedBlockIdForSplit(): string | null {
    const layoutModel = getLayoutModelForStaticTab();
    const focusedNode = layoutModel.focusedNode?.();
    return focusedNode?.data?.blockId ?? null;
}

function getDefaultSplitBlockDef() {
    return { meta: { view: "term", controller: "shell" } };
}

function getAllTabs(ws: any): string[] {
    return [...(ws.pinnedtabids ?? []), ...(ws.tabids ?? [])];
}

function switchTab(offset: number) {
    const ws = atoms.workspace();
    const curTabId = atoms.activeTabId();
    const tabids = getAllTabs(ws);
    const tabIdx = tabids.indexOf(curTabId);
    if (tabIdx === -1) return;
    const newTabIdx = (tabIdx + offset + tabids.length) % tabids.length;
    setActiveTab(tabids[newTabIdx]);
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

export function registerDefaultCommands(): void {
    // ---- open ----
    commandRegistry.register({
        id: "open:terminal",
        label: "Open Terminal",
        category: "Open",
        icon: "square-terminal",
        execute: () => createBlock({ meta: { view: "term", controller: "shell" } }),
    });
    commandRegistry.register({
        id: "open:agent",
        label: "Open Agent",
        category: "Open",
        icon: "sparkles",
        iconColor: "#cc785c",
        execute: () =>
            createBlock({ meta: { view: "agent", controller: "cmd", cmd: "", "cmd:args": [], "cmd:interactive": true, "cmd:runonstart": false } }),
    });
    commandRegistry.register({
        id: "open:sysinfo",
        label: "Open System Info",
        category: "Open",
        icon: "chart-line",
        execute: () => createBlock({ meta: { view: "sysinfo" } }),
    });
    commandRegistry.register({
        id: "open:help",
        label: "Open Help",
        category: "Open",
        icon: "circle-question",
        execute: () => createBlock({ meta: { view: "help" } }),
    });
    commandRegistry.register({
        id: "open:swarm",
        label: "Open Swarm",
        category: "Open",
        icon: "bee",
        iconColor: "#f59e0b",
        execute: () => createBlock({ meta: { view: "swarm" } }),
    });

    // ---- split ----
    commandRegistry.register({
        id: "split:right",
        label: "Split Right",
        category: "Split",
        icon: "table-columns",
        execute: async () => {
            const blockId = getFocusedBlockIdForSplit();
            if (blockId) await createBlockSplitHorizontally(getDefaultSplitBlockDef(), blockId, "after");
        },
    });
    commandRegistry.register({
        id: "split:left",
        label: "Split Left",
        category: "Split",
        icon: "table-columns",
        execute: async () => {
            const blockId = getFocusedBlockIdForSplit();
            if (blockId) await createBlockSplitHorizontally(getDefaultSplitBlockDef(), blockId, "before");
        },
    });
    commandRegistry.register({
        id: "split:down",
        label: "Split Down",
        category: "Split",
        icon: "table-rows",
        execute: async () => {
            const blockId = getFocusedBlockIdForSplit();
            if (blockId) await createBlockSplitVertically(getDefaultSplitBlockDef(), blockId, "after");
        },
    });
    commandRegistry.register({
        id: "split:up",
        label: "Split Up",
        category: "Split",
        icon: "table-rows",
        execute: async () => {
            const blockId = getFocusedBlockIdForSplit();
            if (blockId) await createBlockSplitVertically(getDefaultSplitBlockDef(), blockId, "before");
        },
    });

    // ---- window ----
    commandRegistry.register({
        id: "window:new",
        label: "New Window",
        category: "Window",
        icon: "clone",
        execute: () => getApi().openNewWindow().catch(console.error),
    });
    commandRegistry.register({
        id: "window:close",
        label: "Close Window",
        category: "Window",
        icon: "xmark",
        execute: () => getApi().closeWindow().catch(console.error),
    });
    commandRegistry.register({
        id: "window:minimize",
        label: "Minimize Window",
        category: "Window",
        icon: "window-minimize",
        execute: () => getApi().minimizeWindow(),
    });
    commandRegistry.register({
        id: "window:maximize",
        label: "Toggle Maximize",
        category: "Window",
        icon: "window-maximize",
        execute: () => getApi().maximizeWindow(),
    });

    // ---- tab ----
    commandRegistry.register({
        id: "tab:new",
        label: "New Tab",
        category: "Tab",
        icon: "plus",
        execute: () => createTab(),
    });
    commandRegistry.register({
        id: "tab:close",
        label: "Close Tab",
        category: "Tab",
        icon: "xmark",
        execute: () => {
            const ws = atoms.workspace();
            if (!ws) return;
            const tabId = atoms.activeTabId();
            WorkspaceService.CloseTab(ws.oid, tabId).catch(console.error);
        },
    });
    commandRegistry.register({
        id: "tab:next",
        label: "Next Tab",
        category: "Tab",
        execute: () => switchTab(1),
    });
    commandRegistry.register({
        id: "tab:prev",
        label: "Previous Tab",
        category: "Tab",
        execute: () => switchTab(-1),
    });

    // ---- pane ----
    commandRegistry.register({
        id: "pane:close",
        label: "Close Pane",
        category: "Pane",
        icon: "xmark",
        execute: () => {
            const layoutModel = getLayoutModelForStaticTab();
            fireAndForget(layoutModel.closeFocusedNode.bind(layoutModel));
        },
    });
    commandRegistry.register({
        id: "pane:magnify",
        label: "Toggle Magnify",
        category: "Pane",
        icon: "up-right-and-down-left-from-center",
        execute: () => {
            const layoutModel = getLayoutModelForStaticTab();
            const focusedNode = layoutModel.focusedNode?.();
            if (focusedNode != null) {
                layoutModel.magnifyNodeToggle(focusedNode.id);
            }
        },
    });
    commandRegistry.register({
        id: "pane:focus:right",
        label: "Focus Pane Right",
        category: "Pane",
        execute: () => {
            const layoutModel = getLayoutModelForStaticTab();
            layoutModel.switchNodeFocusInDirection(NavigateDirection.Right);
        },
    });
    commandRegistry.register({
        id: "pane:focus:left",
        label: "Focus Pane Left",
        category: "Pane",
        execute: () => {
            const layoutModel = getLayoutModelForStaticTab();
            layoutModel.switchNodeFocusInDirection(NavigateDirection.Left);
        },
    });
    commandRegistry.register({
        id: "pane:focus:up",
        label: "Focus Pane Up",
        category: "Pane",
        execute: () => {
            const layoutModel = getLayoutModelForStaticTab();
            layoutModel.switchNodeFocusInDirection(NavigateDirection.Up);
        },
    });
    commandRegistry.register({
        id: "pane:focus:down",
        label: "Focus Pane Down",
        category: "Pane",
        execute: () => {
            const layoutModel = getLayoutModelForStaticTab();
            layoutModel.switchNodeFocusInDirection(NavigateDirection.Down);
        },
    });

    // ---- dev ----
    commandRegistry.register({
        id: "dev:devtools",
        label: "Toggle DevTools",
        category: "Dev",
        icon: "code",
        execute: () => getApi().toggleDevtools(),
    });
    commandRegistry.register({
        id: "dev:restart_backend",
        label: "Restart Backend",
        category: "Dev",
        icon: "rotate",
        execute: () => getApi().restartBackend().catch(console.error),
    });
    commandRegistry.register({
        id: "dev:open_settings",
        label: "Open Settings File",
        category: "Dev",
        icon: "cog",
        execute: async () => {
            try {
                const path = await invokeCommand<string>("ensure_settings_file");
                await invokeCommand("open_in_editor", { path });
            } catch (e) {
                console.error("[command-palette] Failed to open settings:", e);
            }
        },
    });

    // ---- view / app (added for the native macOS menu bar — Phase 1) ----
    commandRegistry.register({
        id: "view:command-palette",
        label: "Command Palette",
        category: "View",
        icon: "magnifying-glass",
        execute: () => {
            openModal(CommandPaletteModal);
        },
    });
    commandRegistry.register({
        id: "view:zoom:in",
        label: "Zoom In",
        category: "View",
        icon: "magnifying-glass-plus",
        execute: () => zoomIn(),
    });
    commandRegistry.register({
        id: "view:zoom:out",
        label: "Zoom Out",
        category: "View",
        icon: "magnifying-glass-minus",
        execute: () => zoomOut(),
    });
    commandRegistry.register({
        id: "view:zoom:reset",
        label: "Actual Size",
        category: "View",
        icon: "magnifying-glass",
        execute: () => zoomReset(),
    });
    commandRegistry.register({
        id: "app:identity",
        label: "Identity & Memory",
        category: "App",
        icon: "id-card",
        execute: () => void openOrFocusPaneByView("armory"),
    });
    commandRegistry.register({
        id: "help:docs",
        label: "Online Docs",
        category: "Help",
        icon: "book",
        execute: () => {
            getApi().openExternal("https://docs.agentmux.ai");
        },
    });
}

// ---------------------------------------------------------------------------
// IPC event bridge
// ---------------------------------------------------------------------------

// Listen for `run_command` dispatched by the Rust IPC handler.
window.addEventListener("agentmux-run-command", ((e: CustomEvent) => {
    const id = e.detail?.id as string;
    if (!commandRegistry.run(id)) {
        console.warn(`[command-palette] Unknown command: ${id}`);
    }
}) as EventListener);

// Native macOS menu bar (Phase 1, SPEC_MACOS_NATIVE_MENU_BAR_2026-06-03): the
// host broadcasts `menu:invoke` (via the generic `agentmux-event` channel) to
// every top-level window; only the FOCUSED window runs the command, so a menu
// action lands in the window the user is actually in. Routed through the same
// registry as the palette + hamburger — the spec's single source of truth.
window.addEventListener("agentmux-event", ((e: CustomEvent) => {
    const detail = e.detail;
    if (!detail || detail.event !== "menu:invoke") return;
    if (!document.hasFocus()) return;
    const id = detail.payload?.commandId as string;
    if (!id) return;
    if (!commandRegistry.run(id)) {
        console.warn(`[menu] Unknown command: ${id}`);
    }
}) as EventListener);

// Listen for `open_agent` dispatched by the Rust IPC handler (App API).
// Creates a new agent pane with agentId pre-set so the AgentView auto-launches.
// Reads the persisted ui:zoom before block creation so term:zoom is seeded
// in the initial meta — this is the only open path for the normal user flow.
window.addEventListener("agentmux-open-agent", (async (e: CustomEvent) => {
    const agentId = e.detail?.agentId as string;
    if (!agentId) {
        console.warn("[open-agent] missing agentId");
        return;
    }
    try {
        const { createBlock } = await import("@/app/store/global");
        const { RpcApi } = await import("@/app/store/rpc-api");
        const { TabRpcClient } = await import("@/app/store/rpc-util");
        const meta: Record<string, unknown> = { view: "agent", agentId };
        try {
            const zoomContent = await RpcApi.GetAgentContentCommand(TabRpcClient, {
                agent_id: agentId,
                content_type: "ui:zoom",
            });
            if (zoomContent?.content) {
                const z = parseFloat(zoomContent.content);
                if (!isNaN(z) && z >= 0.5 && z <= 2.0 && Math.abs(z - 1.0) > 0.001) {
                    meta["term:zoom"] = z;
                }
            }
        } catch {
            // zoom lookup is best-effort — open the pane at default zoom
        }
        const blockId = await createBlock({ meta });
        console.log(`[open-agent] created agent pane ${blockId} for agent ${agentId}`);
    } catch (err) {
        console.error("[open-agent] failed:", err);
    }
}) as EventListener);
