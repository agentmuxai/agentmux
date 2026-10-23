// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The open workspace: a named set of windows that srv saves as it changes
// (srv `user_workspace.rs`, docs/specs/SPEC_WORKSPACES_2026_10_23.md). Not the
// `Workspace` object, which is one window's contents. srv keeps which one is
// open on the client object's meta, so its name here follows a rename in any
// window.

import { createSignal } from "solid-js";
import { ObjectService, WorkspaceService, type UserWorkspaceInfo } from "./services";
import { makeORef } from "./mos";
import { pushNotification } from "./flash-notifications";

/** The workspace everyone starts in, and the name shown before srv says. */
export const DEFAULT_WORKSPACE_NAME = "Default Workspace";

/** The longest name srv keeps (as for a window's name). */
export const WORKSPACE_NAME_MAX_LEN = 64;

/** The open workspace's name, from the client object's meta. */
export function workspaceName(client: Client | null | undefined): string {
    const name = client?.meta?.["workspace:name"];
    return typeof name === "string" && name.trim() ? name : DEFAULT_WORKSPACE_NAME;
}

/** The status bar panel's heading: "Default Workspace — 3 windows". */
export function workspaceHeading(name: string, windows: number): string {
    return `${name} — ${windows} window${windows !== 1 ? "s" : ""}`;
}

const pad = (n: number): string => String(n).padStart(2, "0");

/** When a closed window closed, for its row in the status bar panel:
 *  "closed 14:02" today, "closed yesterday 22:10", "closed Oct 17" before. */
export function closedAtLabel(atMs: number, now: Date = new Date()): string {
    if (!atMs) return "closed";
    const at = new Date(atMs);
    const time = `${pad(at.getHours())}:${pad(at.getMinutes())}`;
    const startOfDay = (d: Date): number => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
    const days = Math.round((startOfDay(now) - startOfDay(at)) / 86_400_000);
    if (days <= 0) return `closed ${time}`;
    if (days === 1) return `closed yesterday ${time}`;
    return `closed ${at.toLocaleDateString(undefined, { month: "short", day: "numeric" })}`;
}

/** The title of the only open window's disabled delete. */
export const LAST_WINDOW_TITLE = "The last window; use New Workspace to start over";

/** Rename the open workspace. A blank name is no change. */
export async function renameWorkspace(name: string): Promise<void> {
    const trimmed = name.trim().slice(0, WORKSPACE_NAME_MAX_LEN);
    if (!trimmed) return;
    await WorkspaceService.RenameUserWorkspace("", trimmed);
    void refreshWorkspaceList();
}

const [workspaceList, setWorkspaceList] = createSignal<UserWorkspaceInfo[]>([]);

/** Every workspace, the most recently opened first, as last asked for
 *  (☰ → Workspaces → Open Workspace lists it). */
export { workspaceList };

/** Ask srv for the workspaces again. Keeps the last list on failure. */
export async function refreshWorkspaceList(): Promise<UserWorkspaceInfo[]> {
    try {
        const list = (await WorkspaceService.ListUserWorkspaces()) ?? [];
        setWorkspaceList(list);
        return list;
    } catch (e) {
        console.warn("[user-workspace] couldn't list the workspaces:", e);
        return workspaceList();
    }
}

/** The name New Workspace… suggests: "Workspace 2", or the next number no
 *  workspace has (Default Workspace counts as the first). */
export function nextWorkspaceName(names: string[]): string {
    const taken = new Set(names.map((n) => n.trim().toLowerCase()));
    let n = 2;
    while (taken.has(`workspace ${n}`)) n++;
    return `Workspace ${n}`;
}

/** "2 windows, 1 closed", for a workspace in a list. */
export function workspaceWindowsLine(w: Pick<UserWorkspaceInfo, "windows" | "closed">): string {
    const open = `${w.windows} window${w.windows === 1 ? "" : "s"}`;
    return w.closed > 0 ? `${open}, ${w.closed} closed` : open;
}

let flushInstalled = false;

/** Save the workspace when this window loses focus, if anything changed
 *  (spec §4.1: the app losing focus is a boundary). Idempotent. */
export function installWorkspaceFlushOnBlur(): void {
    if (flushInstalled || typeof window === "undefined") return;
    flushInstalled = true;
    window.addEventListener("blur", () => {
        WorkspaceService.Flush().catch((e) => console.warn("[user-workspace] flush failed:", e));
    });
}

const RESTORED_FROM_TEXT: Record<string, string> = {
    previous: "the last copy that opened cleanly",
    pre_upgrade: "the copy kept from before the update",
};

/** When a launch had to open an older copy of the workspace (srv sets
 *  `workspace:restored_from`), say so once, and clear it. */
export function surfaceWorkspaceRestoreNotice(client: Client | null | undefined): void {
    const from = client?.meta?.["workspace:restored_from"];
    if (typeof from !== "string" || !from) return;
    pushNotification({
        id: "workspace-restored-from",
        icon: "clock-rotate-left",
        title: `${workspaceName(client)} opened from an older copy`,
        message: `Its last save didn't open, so AgentMux used ${RESTORED_FROM_TEXT[from] ?? "an older copy"}. Recent changes to your windows may be missing.`,
        timestamp: new Date().toISOString(),
        type: "warning",
        expiration: Date.now() + 20000,
    });
    ObjectService.UpdateObjectMeta(makeORef("client", client.oid), { "workspace:restored_from": null } as MetaType).catch(
        (e) => console.warn("[user-workspace] couldn't clear the restore notice:", e)
    );
}
