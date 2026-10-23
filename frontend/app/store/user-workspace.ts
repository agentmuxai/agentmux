// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The open workspace: a named set of windows that srv saves as it changes
// (srv `user_workspace.rs`, docs/specs/SPEC_WORKSPACES_2026_10_23.md). Not the
// `Workspace` object, which is one window's contents. srv keeps which one is
// open on the client object's meta, so its name here follows a rename in any
// window.

import { ObjectService, WorkspaceService } from "./services";
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

/** Rename the open workspace. A blank name is no change. */
export async function renameWorkspace(name: string): Promise<void> {
    const trimmed = name.trim().slice(0, WORKSPACE_NAME_MAX_LEN);
    if (!trimmed) return;
    await WorkspaceService.RenameUserWorkspace("", trimmed);
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
