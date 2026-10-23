// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Closing a window keeps it in the open workspace as a closed window (srv
// `session_closed.rs`, SPEC_WORKSPACES_2026_10_23.md §2, §5): the status bar's
// version panel lists it greyed, to reopen or delete. In-app closes warn first
// when an agent is mid-turn; an OS close (Alt+F4, the taskbar) reaches the
// host without passing through here. Kept out of pane code's imports: the
// palette loads it when a command runs.

import { modalsModel, openModal } from "@/app/store/modalmodel";
import { WindowService, type WindowAgent } from "@/app/store/services";
import { workspaceName } from "@/app/store/user-workspace";
import { settingsAtom } from "@/store/config-signals";
import { atoms, getApi, openWindowEntriesAtom, pushNotification } from "@/store/global";
import { RpcApi } from "@/store/rpc-api";
import { TabRpcClient } from "@/store/rpc-util";
import { RemoveWindowConfirm, WindowCloseConfirm } from "./window-close-confirm";

/** The setting: warn before closing a window while an agent in it is
 *  mid-turn. Unset is on. */
const CONFIRM_CLOSE_WORKING = "window:confirmcloseworking";

/** Whether closing this window warns first, given its agents. Never when
 *  none of them has a turn running. */
export function warnsBeforeClosing(
    agents: WindowAgent[],
    settings: Record<string, unknown> | null | undefined
): boolean {
    return agents.some((a) => a.working) && settings?.[CONFIRM_CLOSE_WORKING] !== false;
}

/** What the warning's "Don't ask again" writes. */
export const DONT_ASK_AGAIN = { [CONFIRM_CLOSE_WORKING]: false } as const;

const count = (n: number, one: string): string => `${n} ${one}${n === 1 ? "" : "s"}`;

/** "Alpha, Beta and 3 more". */
export function namesLine(names: string[], max = 4): string {
    if (names.length <= max) {
        return names.length <= 1 ? (names[0] ?? "") : `${names.slice(0, -1).join(", ")} and ${names.at(-1)}`;
    }
    return `${names.slice(0, max).join(", ")} and ${names.length - max} more`;
}

/** "Korp and Loap are working" */
function workingLine(agents: WindowAgent[]): string {
    const working = agents.filter((a) => a.working).map((a) => a.name);
    return `${namesLine(working)} ${working.length === 1 ? "is" : "are"} working`;
}

/** The close warning: who is working, how many agents stop, and where the
 *  window goes. `lastWindow`: closing it quits, and it opens again at the
 *  next launch. */
export function closeWarningText(
    agents: WindowAgent[],
    workspace: string,
    lastWindow: boolean
): { title: string; description: string } {
    const after = lastWindow
        ? `AgentMux closes; the window stays in ${workspace} and opens again at the next launch.`
        : `The window stays in ${workspace}, and you can reopen it from the version panel.`;
    return {
        title: `${workingLine(agents)}. Close this window?`,
        description: `This stops ${count(agents.length, "agent")}. ${after} An agent's own task dev build stops with its pane.`,
    };
}

/** Removing an open window while an agent in it is working. */
export function removeWarningText(agents: WindowAgent[], workspace: string): { title: string; description: string } {
    return {
        title: `${workingLine(agents)}. Remove this window from ${workspace}?`,
        description:
            `This stops ${count(agents.length, "agent")} and deletes the window's tabs and panes. ` +
            `The agents stay in My Agents, with their conversations.`,
    };
}

async function agentsIn(windowId: string): Promise<WindowAgent[]> {
    try {
        return (await WindowService.WindowAgents(windowId)) ?? [];
    } catch (e) {
        console.warn("[window-close] couldn't list the window's agents", e);
        return [];
    }
}

function notifyError(title: string, err: unknown): void {
    pushNotification({
        icon: "triangle-exclamation",
        title,
        message: err instanceof Error ? err.message : String(err),
        timestamp: new Date().toISOString(),
        type: "error",
        expiration: Date.now() + 15000,
    });
}

/** Set while the agents are being asked for, before a confirmation opens. */
let asking = false;

/** The window's agents, unless a close is already being asked about. */
async function agentsUnlessAsking(windowId: string): Promise<WindowAgent[] | null> {
    const open = modalsModel.isModalOpen(WindowCloseConfirm) || modalsModel.isModalOpen(RemoveWindowConfirm);
    if (asking || open) return null;
    asking = true;
    try {
        return await agentsIn(windowId);
    } finally {
        asking = false;
    }
}

const currentWorkspaceName = (): string => workspaceName(atoms.client());

/** Close this window, as the close button, ☰ → Exit and the palette do.
 *  Warns first when an agent in it is mid-turn. */
export async function requestCloseWindow(): Promise<void> {
    const windowId = atoms.muxWindow()?.oid;
    const agents = windowId ? await agentsUnlessAsking(windowId) : [];
    if (!agents) return;
    if (!warnsBeforeClosing(agents, settingsAtom() as Record<string, unknown>)) {
        await getApi().closeWindow();
        return;
    }
    openModal(WindowCloseConfirm, {
        ...closeWarningText(agents, currentWorkspaceName(), openWindowEntriesAtom().length <= 1),
        onConfirm: (dontAskAgain: boolean) => {
            if (dontAskAgain) {
                RpcApi.SetConfigCommand(TabRpcClient, DONT_ASK_AGAIN as any).catch(console.error);
            }
            getApi().closeWindow().catch(console.error);
        },
    });
}

/** The status bar panel's delete for an open window: asks first when an agent
 *  in it is mid-turn, then closes it with its contents deleted (srv
 *  `DiscardWorkspaceOnClose`). */
export async function requestRemoveWindow(w: { windowId: string; label: string }): Promise<void> {
    const agents = await agentsUnlessAsking(w.windowId);
    if (!agents) return;
    const remove = async (): Promise<void> => {
        try {
            await WindowService.DiscardWorkspaceOnClose(w.windowId);
        } catch (err) {
            notifyError("Couldn't remove the window", err);
            return;
        }
        await getApi().closeWindow(w.label);
    };
    if (!agents.some((a) => a.working)) {
        await remove();
        return;
    }
    openModal(RemoveWindowConfirm, { ...removeWarningText(agents, currentWorkspaceName()), onConfirm: remove });
}

/** Reopen a closed window where it was; its agents resume. */
export async function reopenClosedWindow(workspaceId: string): Promise<void> {
    try {
        const at = await WindowService.TakeClosedWindow(workspaceId);
        await getApi().openRestoredWindow(at);
    } catch (err) {
        notifyError("Couldn't reopen the window", err);
    }
}

/** Remove a closed window from the workspace without reopening it. */
export async function removeClosedWindow(workspaceId: string): Promise<void> {
    try {
        await WindowService.DeleteClosedWindow(workspaceId);
    } catch (err) {
        notifyError("Couldn't remove the window", err);
    }
}
