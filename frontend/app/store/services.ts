// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Hand-maintained service-RPC bindings. Keep in sync with the agentmux-srv
// RPC handlers (backend/rpc_types.rs, server/websocket.rs). The original Go
// generator (cmd/generate/main-generatets.go) was removed with the Go backend.

import * as WOS from "./wos";

// blockservice.BlockService (block)
class BlockServiceType {
    GetControllerStatus(arg2: string): Promise<BlockControllerRuntimeStatus> {
        return WOS.callBackendService("block", "GetControllerStatus", Array.from(arguments))
    }

    // save the terminal state to a blockfile
    SaveTerminalState(blockId: string, state: string, stateType: string, ptyOffset: number, termSize: TermSize): Promise<void> {
        return WOS.callBackendService("block", "SaveTerminalState", Array.from(arguments))
    }
}

export const BlockService = new BlockServiceType();

// clientservice.ClientService (client)
class ClientServiceType {
    // @returns object updates
    AgreeTos(): Promise<void> {
        return WOS.callBackendService("client", "AgreeTos", Array.from(arguments))
    }
    FocusWindow(arg2: string): Promise<void> {
        return WOS.callBackendService("client", "FocusWindow", Array.from(arguments))
    }
    GetAllConnStatus(): Promise<ConnStatus[]> {
        return WOS.callBackendService("client", "GetAllConnStatus", Array.from(arguments))
    }
    GetClientData(): Promise<Client> {
        return WOS.callBackendService("client", "GetClientData", Array.from(arguments))
    }
    GetTab(arg1: string): Promise<Tab> {
        return WOS.callBackendService("client", "GetTab", Array.from(arguments))
    }
    TelemetryUpdate(arg2: boolean): Promise<void> {
        return WOS.callBackendService("client", "TelemetryUpdate", Array.from(arguments))
    }
}

export const ClientService = new ClientServiceType();

// objectservice.ObjectService (object)
class ObjectServiceType {
    // @returns blockId (and object updates)
    // `tabId` (optional) overrides uicontext.active_tab_id on the
    // server. Use it when the caller knows the target tab independent
    // of the current active-tab atom — e.g. tab-presets applying a
    // layout to a freshly-created tab without racing against the user
    // switching tabs mid-flow.
    CreateBlock(blockDef: BlockDef, rtOpts: RuntimeOpts, tabId?: string): Promise<string> {
        return WOS.callBackendService("object", "CreateBlock", Array.from(arguments))
    }

    // @returns object updates
    DeleteBlock(blockId: string): Promise<void> {
        return WOS.callBackendService("object", "DeleteBlock", Array.from(arguments))
    }

    // get wave object by oref
    GetObject(oref: string): Promise<WaveObj> {
        return WOS.callBackendService("object", "GetObject", Array.from(arguments))
    }

    // @returns objects
    GetObjects(orefs: string[]): Promise<WaveObj[]> {
        return WOS.callBackendService("object", "GetObjects", Array.from(arguments))
    }

    // @returns object updates
    UpdateObject(waveObj: WaveObj, returnUpdates: boolean): Promise<void> {
        return WOS.callBackendService("object", "UpdateObject", Array.from(arguments))
    }

    // @returns object updates
    UpdateObjectMeta(oref: string, meta: MetaType): Promise<void> {
        return WOS.callBackendService("object", "UpdateObjectMeta", Array.from(arguments))
    }

    // @returns object updates
    UpdateTabName(tabId: string, name: string): Promise<void> {
        return WOS.callBackendService("object", "UpdateTabName", Array.from(arguments))
    }
}

export const ObjectService = new ObjectServiceType();

// userinputservice.UserInputService (userinput)
class UserInputServiceType {
    SendUserInputResponse(arg1: UserInputResponse): Promise<void> {
        return WOS.callBackendService("userinput", "SendUserInputResponse", Array.from(arguments))
    }
}

export const UserInputService = new UserInputServiceType();

// windowservice.WindowService (window)
class WindowServiceType {
    CloseWindow(windowId: string): Promise<void> {
        return WOS.callBackendService("window", "CloseWindow", Array.from(arguments))
    }
    // `hostLabel` (optional, SPEC_POOL_ADOPTION_AND_WINDOW_ROW_CRUMB Residual 2):
    // the caller's CEF window label, persisted as a `host:label` meta crumb on
    // the created Window row so srv-side cleanup can attribute rows without
    // the host registration chain.
    //
    // `restoreIfAvailable` (optional, SPEC_SESSION_RESTORE_AND_SAVED_LAYOUTS_
    // 2026_08_13 Feature 1): only the true cold-start call site in
    // app-init.ts sets this — it tells srv to replay the last-session
    // snapshot (if one exists) instead of seeding the hardcoded default
    // 4-pane layout. Every other caller (tear-off, "Open New Window") omits
    // it and keeps today's always-blank-workspace behavior.
    CreateWindow(winSize: WinSize, workspaceId: string, hostLabel?: string, restoreIfAvailable?: boolean): Promise<WaveWindow> {
        return WOS.callBackendService("window", "CreateWindow", Array.from(arguments))
    }
    GetWindow(windowId: string): Promise<WaveWindow> {
        return WOS.callBackendService("window", "GetWindow", Array.from(arguments))
    }

    // move block to new window
    // @returns object updates
    MoveBlockToNewWindow(currentTabId: string, blockId: string): Promise<void> {
        return WOS.callBackendService("window", "MoveBlockToNewWindow", Array.from(arguments))
    }

    // set window position and size
    // @returns object updates
    SetWindowPosAndSize(windowId: string, pos: Point, size: WinSize): Promise<void> {
        return WOS.callBackendService("window", "SetWindowPosAndSize", Array.from(arguments))
    }
    SwitchWorkspace(windowId: string, workspaceId: string): Promise<Workspace> {
        return WOS.callBackendService("window", "SwitchWorkspace", Array.from(arguments))
    }
}

export const WindowService = new WindowServiceType();

// workspaceservice.WorkspaceService (workspace)
class WorkspaceServiceType {
    // @returns CloseTabRtn (and object updates)
    CloseTab(workspaceId: string, tabId: string): Promise<CloseTabRtnType> {
        return WOS.callBackendService("workspace", "CloseTab", Array.from(arguments))
    }

    // @returns tabId (and object updates)
    CreateTab(workspaceId: string, tabName: string, activateTab: boolean, pinned: boolean): Promise<string> {
        return WOS.callBackendService("workspace", "CreateTab", Array.from(arguments))
    }

    // @returns workspaceId
    CreateWorkspace(name: string, icon: string, color: string, applyDefaults: boolean): Promise<string> {
        return WOS.callBackendService("workspace", "CreateWorkspace", Array.from(arguments))
    }

    // @returns object updates
    DeleteWorkspace(workspaceId: string): Promise<string> {
        return WOS.callBackendService("workspace", "DeleteWorkspace", Array.from(arguments))
    }

    // @returns workspace
    GetWorkspace(workspaceId: string): Promise<Workspace> {
        return WOS.callBackendService("workspace", "GetWorkspace", Array.from(arguments))
    }
    ListWorkspaces(): Promise<WorkspaceListEntry[]> {
        return WOS.callBackendService("workspace", "ListWorkspaces", Array.from(arguments))
    }

    // @returns object updates
    SetActiveTab(workspaceId: string, tabId: string): Promise<void> {
        return WOS.callBackendService("workspace", "SetActiveTab", Array.from(arguments))
    }

    // @returns object updates
    UpdateTabIds(workspaceId: string, tabIds: string[], pinnedTabIds: string[]): Promise<void> {
        return WOS.callBackendService("workspace", "UpdateTabIds", Array.from(arguments))
    }

    // @returns object updates
    UpdateWorkspace(workspaceId: string, name: string): Promise<void> {
        return WOS.callBackendService("workspace", "UpdateWorkspace", Array.from(arguments))
    }

    // Move a block from one tab to another within the same workspace
    // @returns object updates
    MoveBlockToTab(workspaceId: string, blockId: string, sourceTabId: string, destTabId: string, autoClose?: boolean): Promise<void> {
        return WOS.callBackendService("workspace", "MoveBlockToTab", Array.from(arguments))
    }

    // Promote a block from a tab into a new tab
    // @returns new tab id (and object updates)
    PromoteBlockToTab(workspaceId: string, blockId: string, sourceTabId: string, autoClose?: boolean): Promise<string> {
        return WOS.callBackendService("workspace", "PromoteBlockToTab", Array.from(arguments))
    }

    // Reorder a tab within the workspace
    // @returns object updates
    ReorderTab(workspaceId: string, tabId: string, newIndex: number): Promise<void> {
        return WOS.callBackendService("workspace", "ReorderTab", Array.from(arguments))
    }

    // Move a tab from one workspace to another
    // @returns object updates
    MoveTabToWorkspace(tabId: string, sourceWsId: string, destWsId: string, insertIndex?: number): Promise<void> {
        return WOS.callBackendService("workspace", "MoveTabToWorkspace", Array.from(arguments))
    }

    // Move the only tab out of a tear-off workspace and delete that workspace.
    // Used for cancel-back (ESC / drop-on-source) and merge — both produce a
    // single-tab source workspace that MoveTabToWorkspace would refuse to
    // empty out. `wasPinned` controls whether the tab lands in the dest's
    // pinnedtabids (cancel-back of a pinned tab) or tabids (default).
    // Returns object updates (source workspace deleted, dest workspace updated).
    RestoreTornOffTab(tabId: string, sourceWsId: string, destWsId: string, insertIndex?: number, wasPinned?: boolean): Promise<void> {
        return WOS.callBackendService("workspace", "RestoreTornOffTab", Array.from(arguments))
    }

    // Tear off a block into a new workspace
    // @returns new workspace id (and object updates)
    TearOffBlock(blockId: string, sourceTabId: string, sourceWsId: string, autoClose?: boolean): Promise<string> {
        return WOS.callBackendService("workspace", "TearOffBlock", Array.from(arguments))
    }

    // Tear off a tab into a new workspace
    // @returns new workspace id (and object updates)
    TearOffTab(tabId: string, sourceWsId: string): Promise<string> {
        return WOS.callBackendService("workspace", "TearOffTab", Array.from(arguments))
    }

    // Re-dock a floating pane's block into an existing tab in another
    // workspace. The inverse of TearOffBlock. Source floater auto-closes
    // via the empty-tab watcher in floating-pane-workspace.tsx (PR #1089)
    // once its tab.blockids becomes empty.
    //
    // Phase 4b: optional targetBlockId + direction route the drop to the
    // exact slot the ghost previewed (SplitHorizontal/SplitVertical instead of
    // the generic InsertNode). Pass null for both to keep the old behavior.
    //
    // @returns { redocked: true, block_id, target_tab_id } (and object updates)
    RedockFloatingPane(
        blockId: string,
        sourceTabId: string,
        sourceWsId: string,
        targetTabId: string,
        targetWsId: string,
        targetBlockId?: string | null,
        direction?: number | null,
    ): Promise<{ redocked: boolean; block_id?: string; target_tab_id?: string }> {
        return WOS.callBackendService("workspace", "RedockFloatingPane", Array.from(arguments))
    }
}

export const WorkspaceService = new WorkspaceServiceType();

