// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Hand-maintained service-RPC bindings. Keep in sync with the agentmux-srv
// RPC handlers (backend/rpc_types/, server/websocket.rs). The original Go
// generator (cmd/generate/main-generatets.go) was removed with the Go backend.

import * as MOS from "./mos";

import type { BlockControllerRuntimeStatus } from "@/types/rpc/BlockControllerRuntimeStatus";
import type { ConnStatus } from "@/types/rpc/ConnStatus";

/** A window a restore rebuilt beside the main one: its workspace, and where it
 *  was (physical pixels, from srv's window row). srv `session_windows.rs`. */
export type RestoredWindow = { workspaceid: string; x?: number; y?: number; width?: number; height?: number };

/** A closed window of the open workspace, for the status bar panel (srv
 *  `session_closed.rs`). `agents` are the names of its agents. */
export type ClosedWindow = {
    workspaceid: string;
    closed_at_ms: number;
    name: string;
    tabs: number;
    panes: number;
    agents: string[];
    /** How many of its agents keep running without it (parked at its close). */
    running: number;
};

/** An agent running in a window's workspace; `working` when it is mid-turn. */
export type WindowAgent = { block_id: string; name: string; working: boolean };

/** A user workspace, a named set of windows (srv `user_workspace_switch.rs`):
 *  `windows` and `closed` count its open and closed windows as last saved. */
export type UserWorkspaceInfo = {
    id: string;
    name: string;
    current: boolean;
    opened_at: number | null;
    windows: number;
    closed: number;
};

/** What a switch did: the windows the host closes, and how many wait to open
 *  beside the one that asked (`TakeRestoredWindows`). */
export type WorkspaceSwitched = {
    id: string;
    name: string;
    closed: { window_id: string; label: string }[];
    beside: number;
};

/** What Import… shows before it opens a file. */
export type WorkspaceImportPreview = {
    name: string;
    trusted: boolean;
    windows: number;
    closed: number;
    tabs: { name: string; panes: string[] }[];
    commands: string[];
    notes: string[];
};

// blockservice.BlockService (block)
class BlockServiceType {
    GetControllerStatus(arg2: string): Promise<BlockControllerRuntimeStatus> {
        return MOS.callBackendService("block", "GetControllerStatus", Array.from(arguments))
    }

    // save the terminal state to a blockfile
    SaveTerminalState(blockId: string, state: string, stateType: string, ptyOffset: number, termSize: TermSize): Promise<void> {
        return MOS.callBackendService("block", "SaveTerminalState", Array.from(arguments))
    }
}

export const BlockService = new BlockServiceType();

// clientservice.ClientService (client)
class ClientServiceType {
    // @returns object updates
    AgreeTos(): Promise<void> {
        return MOS.callBackendService("client", "AgreeTos", Array.from(arguments))
    }
    FocusWindow(arg2: string): Promise<void> {
        return MOS.callBackendService("client", "FocusWindow", Array.from(arguments))
    }
    GetAllConnStatus(): Promise<ConnStatus[]> {
        return MOS.callBackendService("client", "GetAllConnStatus", Array.from(arguments))
    }
    GetClientData(): Promise<Client> {
        return MOS.callBackendService("client", "GetClientData", Array.from(arguments))
    }
    GetTab(arg1: string): Promise<Tab> {
        return MOS.callBackendService("client", "GetTab", Array.from(arguments))
    }
    TelemetryUpdate(arg2: boolean): Promise<void> {
        return MOS.callBackendService("client", "TelemetryUpdate", Array.from(arguments))
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
        return MOS.callBackendService("object", "CreateBlock", Array.from(arguments))
    }

    // @returns object updates
    DeleteBlock(blockId: string): Promise<void> {
        return MOS.callBackendService("object", "DeleteBlock", Array.from(arguments))
    }

    // close a pane: stop and delete every block in its stack, each process
    // stopped before its records go. One id = one tab of a stack; the pane
    // itself is removed only when no member survives.
    // frontendWaits: the pane stayed on screen showing the shutdown log, so
    // srv removes each closed tab from the layout itself (§5.5).
    ClosePane(blockIds: string[], frontendWaits?: boolean): Promise<void> {
        return MOS.callBackendService("object", "ClosePane", Array.from(arguments))
    }

    // end an agent gracefully and close its own tab (/quit, /exit);
    // SPEC_AGENT_SELF_QUIT_2026_09_24.md §5. Resolves to the quit summary.
    QuitAgent(blockId: string): Promise<any> {
        return MOS.callBackendService("object", "QuitAgent", Array.from(arguments))
    }

    // shut an agent down from My agents or its pane's menu (the conversation is kept);
    // SPEC_CLOSING_PARKS_AGENTS_SHUTDOWN_IS_EXPLICIT §4.4. Resolves to the quit summary.
    ShutdownAgent(blockId: string, stopBackground?: boolean): Promise<any> {
        return MOS.callBackendService("object", "ShutdownAgent", Array.from(arguments))
    }

    // what a shutdown would end, for the confirm: {working, claims, crons_targeting}
    ShutdownPreview(blockId: string): Promise<any> {
        return MOS.callBackendService("object", "ShutdownPreview", Array.from(arguments))
    }

    // get wave object by oref
    GetObject(oref: string): Promise<MuxObj> {
        return MOS.callBackendService("object", "GetObject", Array.from(arguments))
    }

    // @returns objects
    GetObjects(orefs: string[]): Promise<MuxObj[]> {
        return MOS.callBackendService("object", "GetObjects", Array.from(arguments))
    }

    // @returns object updates
    UpdateObject(muxObj: MuxObj, returnUpdates: boolean): Promise<void> {
        return MOS.callBackendService("object", "UpdateObject", Array.from(arguments))
    }

    // @returns object updates
    UpdateObjectMeta(oref: string, meta: MetaType): Promise<void> {
        return MOS.callBackendService("object", "UpdateObjectMeta", Array.from(arguments))
    }

    // @returns object updates
    UpdateTabName(tabId: string, name: string): Promise<void> {
        return MOS.callBackendService("object", "UpdateTabName", Array.from(arguments))
    }
}

export const ObjectService = new ObjectServiceType();

// userinputservice.UserInputService (userinput)
class UserInputServiceType {
    SendUserInputResponse(arg1: UserInputResponse): Promise<void> {
        return MOS.callBackendService("userinput", "SendUserInputResponse", Array.from(arguments))
    }
}

export const UserInputService = new UserInputServiceType();

// windowservice.WindowService (window)
class WindowServiceType {
    CloseWindow(windowId: string): Promise<void> {
        return MOS.callBackendService("window", "CloseWindow", Array.from(arguments))
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
    // 3-pane layout. Every other caller (tear-off, "Open New Window") omits
    // it and keeps today's always-blank-workspace behavior.
    // `seedView`/`seedMeta` (optional, arg 4/5 — same additive pattern as
    // `restoreIfAvailable` above): a widget's "Open in New Window" passes
    // its resolved view + full blockdef.meta here so srv seeds the fresh
    // tab with ONLY that one pane instead of the default 3-pane layout.
    // Every other caller omits them and keeps the default-seed behavior.
    // `previousWindowId` (optional, arg 6): the row this renderer had before a
    // reload; srv copies its theme and opacity onto the new row
    // (SPEC_PER_WINDOW_THEME_AND_OPACITY_2026_10_12 §4.6).
    CreateWindow(
        winSize: WinSize,
        workspaceId: string,
        hostLabel?: string,
        restoreIfAvailable?: boolean,
        seedView?: string,
        seedMeta?: Record<string, unknown>,
        previousWindowId?: string,
        anotherWindowOpening?: boolean
    ): Promise<MuxWindow> {
        return MOS.callBackendService("window", "CreateWindow", Array.from(arguments))
    }
    GetWindow(windowId: string): Promise<MuxWindow> {
        return MOS.callBackendService("window", "GetWindow", Array.from(arguments))
    }
    // Take before opening a window that should inherit appearance; pass to
    // InheritWindowAppearance (SPEC_PER_WINDOW_THEME_AND_OPACITY_2026_10_12 §6).
    MarkWindowOpen(): Promise<number> {
        return MOS.callBackendService("window", "MarkWindowOpen", Array.from(arguments))
    }
    // Copy `sourceWindowId`'s theme and opacity onto the window with host label
    // `hostLabel` opened after `mark`, whether or not its row exists yet.
    InheritWindowAppearance(sourceWindowId: string, hostLabel: string, mark: number): Promise<void> {
        return MOS.callBackendService("window", "InheritWindowAppearance", Array.from(arguments))
    }
    // The workspaces a restore rebuilt beside the main window's, the rest of
    // the last session's windows, each handed out once (srv `session_windows.rs`).
    TakeRestoredWindows(): Promise<RestoredWindow[]> {
        return MOS.callBackendService("window", "TakeRestoredWindows", Array.from(arguments))
    }
    // The open workspace's closed windows (srv `session_closed.rs`), newest first.
    ListClosedWindows(): Promise<ClosedWindow[]> {
        return MOS.callBackendService("window", "ListClosedWindows", Array.from(arguments))
    }
    // Reopen one: drops it from the list and returns where it was, for
    // `getApi().openRestoredWindow`.
    TakeClosedWindow(workspaceId: string): Promise<RestoredWindow> {
        return MOS.callBackendService("window", "TakeClosedWindow", Array.from(arguments))
    }
    // Remove a closed window from the workspace: deletes its contents.
    DeleteClosedWindow(workspaceId: string): Promise<void> {
        return MOS.callBackendService("window", "DeleteClosedWindow", Array.from(arguments))
    }
    // Remove an open window from the workspace: its next close deletes its
    // contents instead of keeping them. Refused for the only open window.
    DiscardWorkspaceOnClose(windowId: string): Promise<void> {
        return MOS.callBackendService("window", "DiscardWorkspaceOnClose", Array.from(arguments))
    }
    // The agents running in the window's workspace, for the close warning.
    WindowAgents(windowId: string): Promise<WindowAgent[]> {
        return MOS.callBackendService("window", "WindowAgents", Array.from(arguments))
    }

    // move block to new window
    // @returns object updates
    MoveBlockToNewWindow(currentTabId: string, blockId: string): Promise<void> {
        return MOS.callBackendService("window", "MoveBlockToNewWindow", Array.from(arguments))
    }

    // set window position and size
    // @returns object updates
    SetWindowPosAndSize(windowId: string, pos: Point, size: WinSize): Promise<void> {
        return MOS.callBackendService("window", "SetWindowPosAndSize", Array.from(arguments))
    }
    SwitchWorkspace(windowId: string, workspaceId: string): Promise<Workspace> {
        return MOS.callBackendService("window", "SwitchWorkspace", Array.from(arguments))
    }
}

export const WindowService = new WindowServiceType();

// workspaceservice.WorkspaceService (workspace)
class WorkspaceServiceType {
    // @returns CloseTabRtn (and object updates)
    CloseTab(workspaceId: string, tabId: string): Promise<CloseTabRtnType> {
        return MOS.callBackendService("workspace", "CloseTab", Array.from(arguments))
    }

    // @returns tabId (and object updates)
    CreateTab(workspaceId: string, tabName: string, activateTab: boolean, pinned: boolean): Promise<string> {
        return MOS.callBackendService("workspace", "CreateTab", Array.from(arguments))
    }

    // @returns workspaceId
    CreateWorkspace(name: string, icon: string, color: string, applyDefaults: boolean): Promise<string> {
        return MOS.callBackendService("workspace", "CreateWorkspace", Array.from(arguments))
    }

    // @returns object updates
    DeleteWorkspace(workspaceId: string): Promise<string> {
        return MOS.callBackendService("workspace", "DeleteWorkspace", Array.from(arguments))
    }

    // @returns workspace
    GetWorkspace(workspaceId: string): Promise<Workspace> {
        return MOS.callBackendService("workspace", "GetWorkspace", Array.from(arguments))
    }
    ListWorkspaces(): Promise<WorkspaceListEntry[]> {
        return MOS.callBackendService("workspace", "ListWorkspaces", Array.from(arguments))
    }

    // @returns object updates
    SetActiveTab(workspaceId: string, tabId: string): Promise<void> {
        return MOS.callBackendService("workspace", "SetActiveTab", Array.from(arguments))
    }

    // @returns object updates
    UpdateTabIds(workspaceId: string, tabIds: string[], pinnedTabIds: string[]): Promise<void> {
        return MOS.callBackendService("workspace", "UpdateTabIds", Array.from(arguments))
    }

    // @returns object updates
    UpdateWorkspace(workspaceId: string, name: string): Promise<void> {
        return MOS.callBackendService("workspace", "UpdateWorkspace", Array.from(arguments))
    }

    // The user's workspace, a named set of windows (srv `user_workspace.rs`),
    // not the window's contents the methods above act on.
    // Save it now if anything changed since its last save.
    Flush(): Promise<{ saved: boolean }> {
        return MOS.callBackendService("workspace", "Flush", Array.from(arguments))
    }

    // Rename a workspace (`""` is the open one).
    // @returns object updates (the client, whose meta holds the open one's name)
    RenameUserWorkspace(workspaceId: string, name: string): Promise<void> {
        return MOS.callBackendService("workspace", "RenameUserWorkspace", Array.from(arguments))
    }

    // Every workspace, the most recently opened first.
    ListUserWorkspaces(): Promise<UserWorkspaceInfo[]> {
        return MOS.callBackendService("workspace", "ListUserWorkspaces", Array.from(arguments))
    }

    // Make a workspace and switch to it from window `windowId`.
    // @returns object updates (the client)
    CreateUserWorkspace(name: string, windowId: string): Promise<WorkspaceSwitched> {
        return MOS.callBackendService("workspace", "CreateUserWorkspace", Array.from(arguments))
    }

    // Close the open workspace and open another, reusing window `windowId`.
    // @returns object updates (the client)
    SwitchUserWorkspace(workspaceId: string, windowId: string): Promise<WorkspaceSwitched> {
        return MOS.callBackendService("workspace", "SwitchUserWorkspace", Array.from(arguments))
    }

    // Delete a workspace that isn't open, and its windows' contents.
    DeleteUserWorkspace(workspaceId: string): Promise<void> {
        return MOS.callBackendService("workspace", "DeleteUserWorkspace", Array.from(arguments))
    }

    // The agents running in the open workspace's windows.
    UserWorkspaceAgents(): Promise<WindowAgent[]> {
        return MOS.callBackendService("workspace", "UserWorkspaceAgents", Array.from(arguments))
    }

    // Write the open workspace to `path` (`*.agentmux-workspace.json`).
    ExportUserWorkspace(path: string, includeResume?: boolean): Promise<{ path: string; warnings: string[] }> {
        return MOS.callBackendService("workspace", "ExportUserWorkspace", Array.from(arguments))
    }

    // Describe a workspace or layout file before Import… opens it.
    PreviewImport(path: string): Promise<WorkspaceImportPreview> {
        return MOS.callBackendService("workspace", "PreviewImport", Array.from(arguments))
    }

    // Open a workspace or layout file as a new workspace and switch to it.
    // `runCommands` null means "as trusted". @returns object updates (the client)
    ImportUserWorkspace(
        path: string,
        runCommands: boolean | null,
        windowId: string
    ): Promise<WorkspaceSwitched & { notes: string[] }> {
        return MOS.callBackendService("workspace", "ImportUserWorkspace", Array.from(arguments))
    }

    // Move a block from one tab to another within the same workspace
    // @returns object updates
    MoveBlockToTab(workspaceId: string, blockId: string, sourceTabId: string, destTabId: string, autoClose?: boolean): Promise<void> {
        return MOS.callBackendService("workspace", "MoveBlockToTab", Array.from(arguments))
    }

    // Promote a block from a tab into a new tab
    // @returns new tab id (and object updates)
    PromoteBlockToTab(workspaceId: string, blockId: string, sourceTabId: string, autoClose?: boolean): Promise<string> {
        return MOS.callBackendService("workspace", "PromoteBlockToTab", Array.from(arguments))
    }

    // Reorder a tab within the workspace
    // @returns object updates
    ReorderTab(workspaceId: string, tabId: string, newIndex: number): Promise<void> {
        return MOS.callBackendService("workspace", "ReorderTab", Array.from(arguments))
    }

    // Move a tab from one workspace to another
    // @returns object updates
    MoveTabToWorkspace(tabId: string, sourceWsId: string, destWsId: string, insertIndex?: number): Promise<void> {
        return MOS.callBackendService("workspace", "MoveTabToWorkspace", Array.from(arguments))
    }

    // Move the only tab out of a tear-off workspace and delete that workspace.
    // Used for cancel-back (ESC / drop-on-source) and merge — both produce a
    // single-tab source workspace that MoveTabToWorkspace would refuse to
    // empty out. `wasPinned` controls whether the tab lands in the dest's
    // pinnedtabids (cancel-back of a pinned tab) or tabids (default).
    // Returns object updates (source workspace deleted, dest workspace updated).
    RestoreTornOffTab(tabId: string, sourceWsId: string, destWsId: string, insertIndex?: number, wasPinned?: boolean): Promise<void> {
        return MOS.callBackendService("workspace", "RestoreTornOffTab", Array.from(arguments))
    }

    // Tear off a block into a new workspace
    // @returns new workspace id (and object updates)
    // `members`: the pane's whole tab stack in order, `blockId` being its visible
    // tab — a header drag tears the pane off with all of its tabs. Omit it for
    // one block (a pane tab's pill).
    TearOffBlock(
        blockId: string,
        sourceTabId: string,
        sourceWsId: string,
        autoClose?: boolean,
        members?: string[],
    ): Promise<string> {
        return MOS.callBackendService("workspace", "TearOffBlock", Array.from(arguments))
    }

    // Tear off a tab into a new workspace
    // @returns new workspace id (and object updates)
    TearOffTab(tabId: string, sourceWsId: string): Promise<string> {
        return MOS.callBackendService("workspace", "TearOffTab", Array.from(arguments))
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
    // `asTab` (with targetBlockId): land the block as a TAB of targetBlockId's
    // pane, active, instead of a split/insert — the rollback of a failed
    // pane-tab tear-off (SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §3.5).
    //
    // `members` + `activeBlockId`: a floater holding a whole pane (several tabs)
    // redocks all of them. `blockId` is the first member, which lands as the
    // pane; the rest land as its tabs in order; `activeBlockId` ends visible.
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
        asTab?: boolean,
        members?: string[],
        activeBlockId?: string,
    ): Promise<{ redocked: boolean; block_id?: string; target_tab_id?: string }> {
        return MOS.callBackendService("workspace", "RedockFloatingPane", Array.from(arguments))
    }
}

export const WorkspaceService = new WorkspaceServiceType();

