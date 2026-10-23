// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// ☰ → Workspaces and the palette's workspace commands
// (SPEC_WORKSPACES_2026_10_23.md §3.2, §6). srv does the switch
// (`user_workspace_switch.rs`): it saves the open workspace, closes it and
// opens the other in the window the user acted from. This side asks first
// when an agent is working, then closes the windows srv closed and opens the
// ones waiting beside this one. Loaded when used, like window-close.ts.

import { reopenRestoredWindows } from "@/app/init/restored-windows";
import { openModal } from "@/app/store/modalmodel";
import { WorkspaceService, type UserWorkspaceInfo, type WindowAgent, type WorkspaceSwitched } from "@/app/store/services";
import {
    nextWorkspaceName,
    refreshWorkspaceList,
    renameWorkspace,
    workspaceName,
} from "@/app/store/user-workspace";
import { windowId } from "@/app/store/window-identity";
import { settingsAtom } from "@/store/config-signals";
import { atoms, getApi, pushNotification } from "@/store/global";
import { RpcApi } from "@/store/rpc-api";
import { TabRpcClient } from "@/store/rpc-util";
import { DONT_ASK_AGAIN, namesLine, warnsBeforeClosing } from "./window-close";
import { WindowCloseConfirm } from "./window-close-confirm";
import {
    DeleteWorkspaceConfirm,
    ImportPreviewModal,
    WorkspaceNameModal,
    WorkspacePickerModal,
} from "./workspace-modals";

const count = (n: number, one: string): string => `${n} ${one}${n === 1 ? "" : "s"}`;

function notify(type: "info" | "warning" | "error", title: string, message: string): void {
    pushNotification({
        icon: type === "error" ? "triangle-exclamation" : "window-restore",
        title,
        message,
        timestamp: new Date().toISOString(),
        type,
        expiration: Date.now() + (type === "info" ? 6000 : 15000),
    });
}

const errorText = (err: unknown): string => (err instanceof Error ? err.message : String(err));

const currentName = (): string => workspaceName(atoms.client());

/** The warning before a switch closes `workspace` while agents in it work:
 *  "Korp and Loap are working. Close Default Workspace?" */
export function switchWarningText(agents: WindowAgent[], workspace: string): { title: string; description: string } {
    const working = agents.filter((a) => a.working).map((a) => a.name);
    return {
        title: `${namesLine(working)} ${working.length === 1 ? "is" : "are"} working. Close ${workspace}?`,
        description:
            `This stops ${count(agents.length, "agent")}. ${workspace} keeps its windows, ` +
            `and they open again, agents resuming, when you go back to it.`,
    };
}

/** Whether to go ahead and close the open workspace: at once when no agent
 *  in it is mid-turn (or the warning is off), else when the user says so. */
export async function confirmLeaving(): Promise<boolean> {
    let agents: WindowAgent[] = [];
    try {
        agents = (await WorkspaceService.UserWorkspaceAgents()) ?? [];
    } catch (e) {
        console.warn("[workspaces] couldn't list the open workspace's agents", e);
    }
    if (!warnsBeforeClosing(agents, settingsAtom() as Record<string, unknown>)) return true;
    return new Promise<boolean>((resolve) => {
        openModal(WindowCloseConfirm, {
            ...switchWarningText(agents, currentName()),
            confirmLabel: "Close and switch",
            onConfirm: (dontAskAgain: boolean) => {
                if (dontAskAgain) {
                    RpcApi.SetConfigCommand(TabRpcClient, DONT_ASK_AGAIN as any).catch(console.error);
                }
                resolve(true);
            },
            onCancel: () => resolve(false),
        });
    });
}

/** After srv switched: close the windows it closed, open the ones waiting
 *  beside this one, and list the workspaces again. */
async function finishSwitch(switched: WorkspaceSwitched): Promise<void> {
    for (const w of switched.closed ?? []) {
        if (!w.label) continue;
        await getApi()
            .closeWindow(w.label)
            .catch((e: unknown) => console.warn("[workspaces] couldn't close a window", w.label, e));
    }
    if (switched.beside > 0) await reopenRestoredWindows();
    void refreshWorkspaceList();
}

/** Run a switch srv makes (`run`), after the warning if one is due. */
async function switchWith(run: () => Promise<WorkspaceSwitched>, failure: string): Promise<WorkspaceSwitched | null> {
    if (!(await confirmLeaving())) return null;
    let switched: WorkspaceSwitched;
    try {
        switched = await run();
    } catch (err) {
        notify("error", failure, errorText(err));
        return null;
    }
    await finishSwitch(switched);
    return switched;
}

/** Open workspace `w`, closing the open one. */
export async function switchWorkspace(w: Pick<UserWorkspaceInfo, "id" | "name" | "current">): Promise<void> {
    if (w.current) return;
    await switchWith(() => WorkspaceService.SwitchUserWorkspace(w.id, windowId()), `Couldn't open ${w.name}`);
}

/** New Workspace…: ask for a name, then switch to a new workspace with the
 *  default panes. */
export async function newWorkspace(): Promise<void> {
    const list = await refreshWorkspaceList();
    openModal(WorkspaceNameModal, {
        title: "New workspace",
        description: `${currentName()} closes; it keeps its windows.`,
        initial: nextWorkspaceName(list.map((w) => w.name)),
        confirmLabel: "Create",
        onSubmit: async (name: string) => {
            await switchWith(() => WorkspaceService.CreateUserWorkspace(name, windowId()), "Couldn't create the workspace");
        },
    });
}

/** Open Workspace… (the palette's): choose one from a list. */
export async function openWorkspacePicker(): Promise<void> {
    const list = await refreshWorkspaceList();
    openModal(WorkspacePickerModal, {
        title: "Open workspace",
        description: `${currentName()} closes; it keeps its windows.`,
        workspaces: list,
        onPick: (w: UserWorkspaceInfo) => void switchWorkspace(w),
    });
}

/** Rename workspace…: the open one. */
export function renameWorkspacePrompt(): void {
    openModal(WorkspaceNameModal, {
        title: "Rename workspace",
        initial: currentName(),
        confirmLabel: "Rename",
        onSubmit: (name: string) =>
            renameWorkspace(name).catch((err: unknown) => notify("error", "Couldn't rename the workspace", errorText(err))),
    });
}

/** Delete `w`, a workspace that isn't open. */
export async function deleteWorkspace(w: UserWorkspaceInfo): Promise<void> {
    try {
        await WorkspaceService.DeleteUserWorkspace(w.id);
    } catch (err) {
        notify("error", `Couldn't delete ${w.name}`, errorText(err));
        return;
    }
    await refreshWorkspaceList();
    notify("info", `Deleted ${w.name}`, "Its agents are still in My Agents.");
}

/** Delete workspace…: choose one of the others, then confirm. */
export async function deleteWorkspacePrompt(): Promise<void> {
    const others = (await refreshWorkspaceList()).filter((w) => !w.current);
    if (others.length === 0) {
        notify("info", "No other workspace to delete", `${currentName()} is open, and the open workspace can't be deleted.`);
        return;
    }
    openModal(WorkspacePickerModal, {
        title: "Delete workspace",
        description: "The open workspace can't be deleted; open another one first.",
        workspaces: others,
        onPick: (w: UserWorkspaceInfo) =>
            openModal(DeleteWorkspaceConfirm, { workspace: w, onConfirm: () => deleteWorkspace(w) }),
    });
}

/** Export…: write the open workspace to a file the user picks. */
export async function exportWorkspace(): Promise<void> {
    const name = currentName();
    const path = await getApi()?.showExportWorkspaceDialog?.(name);
    if (!path) return;
    try {
        const result = await WorkspaceService.ExportUserWorkspace(path);
        if (result.warnings.length > 0) {
            notify("warning", `Exported ${name} — check before sharing`, [result.path, ...result.warnings].join("\n"));
        } else {
            notify("info", `Exported ${name}`, result.path);
        }
    } catch (err) {
        notify("error", "Couldn't export the workspace", errorText(err));
    }
}

/** Import…: open an exported workspace, or an older layout file, as a new
 *  workspace, after a preview of what it opens. */
export async function importWorkspace(): Promise<void> {
    const path = await getApi()?.showImportWorkspaceDialog?.();
    if (!path) return;
    let preview;
    try {
        preview = await WorkspaceService.PreviewImport(path);
    } catch (err) {
        notify("error", "Couldn't read the file", errorText(err));
        return;
    }
    openModal(ImportPreviewModal, {
        preview,
        current: currentName(),
        onImport: async (runCommands: boolean) => {
            let notes: string[] = [];
            const switched = await switchWith(async () => {
                const r = await WorkspaceService.ImportUserWorkspace(path, runCommands, windowId());
                notes = r.notes ?? [];
                return r;
            }, "Couldn't import the workspace");
            if (!switched) return;
            if (notes.length > 0) {
                notify("warning", `Imported ${switched.name}`, notes.join("\n"));
            } else {
                notify("info", `Imported ${switched.name}`, `${count(preview.windows, "window")}, ${count(preview.tabs.length, "tab")}.`);
            }
        },
    });
}
