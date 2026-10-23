// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Closing is not deleting (SPEC_OFFLINE_RESILIENCE_2026_10_20.md G8): a close
// keeps the window's workspace (srv `session_closed.rs`) for the next launch
// and "Reopen closed window"; only a confirmed Close workspace deletes one.
// In-app closes warn first when an agent is mid-turn; an OS close (Alt+F4,
// the taskbar) reaches the host without passing through here.

import { setNotificationAction } from "@/app/notification/usenotification";
import { openModal } from "@/app/store/modalmodel";
import { WindowService, type ClosedWindow, type WindowAgent } from "@/app/store/services";
import { windowId } from "@/app/store/window-identity";
import { settingsAtom } from "@/store/config-signals";
import { getApi, pushNotification } from "@/store/global";
import { RpcApi } from "@/store/rpc-api";
import { TabRpcClient } from "@/store/rpc-util";
import { whenAt } from "./restore-other-build";
import { CloseWorkspaceConfirm, WindowCloseConfirm } from "./window-close-confirm";

/** The setting: warn before closing a window while an agent in it is
 *  mid-turn. Unset is on. */
const CONFIRM_CLOSE_WORKING = "window:confirmcloseworking";

/** Whether closing this window warns first, given its agents. Never when
 *  none of them has a turn running. */
export function warnsBeforeClosing(
    agents: WindowAgent[],
    settings: Record<string, unknown> | null | undefined
): boolean {
    return agents.some((a) => a.working) && settings?.["window:confirmcloseworking"] !== false;
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

/** The close warning: how many agents stop, which are working, and that the
 *  layout is kept. */
export function closeWarningText(agents: WindowAgent[]): { title: string; description: string } {
    const working = agents.filter((a) => a.working).map((a) => a.name);
    const verb = working.length === 1 ? "is" : "are";
    return {
        title: `${namesLine(working)} ${verb} working. Close this window?`,
        description:
            `This stops ${count(agents.length, "agent")}; the layout is saved. Their conversations are kept, ` +
            `and the window comes back on the next launch or from ☰ → Reopen closed window. ` +
            `An agent's own task dev build stops with its pane.`,
    };
}

/** Close workspace's confirmation, naming the agents it stops. */
export function closeWorkspaceText(agents: WindowAgent[]): { title: string; description: string } {
    const stops =
        agents.length === 0
            ? ""
            : ` It stops ${count(agents.length, "agent")} (${namesLine(agents.map((a) => a.name))}); ` +
              `their conversations are kept, so you can open them again.`;
    return {
        title: "Close this workspace?",
        description: `This closes the window and deletes its tabs and panes for good.${stops}`,
    };
}

async function agentsInThisWindow(): Promise<WindowAgent[]> {
    try {
        return (await WindowService.WindowAgents(windowId())) ?? [];
    } catch (e) {
        console.warn("[window-close] couldn't list this window's agents", e);
        return [];
    }
}

function notify(type: "info" | "error", title: string, message: string): void {
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

let asking = false;

/** Close this window, as the close button, ☰ → Exit and the palette do.
 *  Warns first when an agent in it is mid-turn. */
export async function requestCloseWindow(): Promise<void> {
    if (asking) return;
    const agents = await agentsInThisWindow();
    if (!warnsBeforeClosing(agents, settingsAtom() as Record<string, unknown>)) {
        await getApi().closeWindow();
        return;
    }
    asking = true;
    openModal(WindowCloseConfirm, {
        ...closeWarningText(agents),
        onConfirm: (dontAskAgain: boolean) => {
            asking = false;
            if (dontAskAgain) {
                RpcApi.SetConfigCommand(TabRpcClient, DONT_ASK_AGAIN as any).catch(console.error);
            }
            getApi().closeWindow().catch(console.error);
        },
        onCancel: () => {
            asking = false;
        },
    });
}

/** ☰ → Close workspace…: confirm, naming the agents it stops, then close this
 *  window and delete its workspace. */
export async function requestCloseWorkspace(): Promise<void> {
    if (asking) return;
    asking = true;
    const agents = await agentsInThisWindow();
    openModal(CloseWorkspaceConfirm, {
        ...closeWorkspaceText(agents),
        onConfirm: async () => {
            asking = false;
            try {
                await WindowService.DiscardWorkspaceOnClose(windowId());
            } catch (err) {
                notify("error", "Couldn't close the workspace", errorText(err));
                return;
            }
            await getApi().closeWindow();
        },
        onCancel: () => {
            asking = false;
        },
    });
}

/** "Ops · 3 tabs, 2 agents · closed 14:02 today". */
export function closedWindowLabel(w: ClosedWindow, now: Date = new Date()): string {
    const what = [count(w.tabs, "tab"), w.agents.length > 0 ? count(w.agents.length, "agent") : count(w.panes, "pane")];
    return [w.name.trim() || "Window", what.join(", "), whenAt(w.closed_at_ms, "closed", now)].join(" · ");
}

/** Reopen `w` in a window of its own, where it was. */
export async function reopenClosedWindow(w: ClosedWindow): Promise<void> {
    try {
        const at = await WindowService.TakeClosedWindow(w.workspaceid);
        await getApi().openRestoredWindow(at);
    } catch (err) {
        notify("error", "Couldn't reopen the window", errorText(err));
    }
}

/** The most windows the chooser offers a button for. */
const CHOOSER_MAX = 4;

let actionSeq = 0;

function action(fn: () => void): string {
    const key = `reopenClosedWindow:${++actionSeq}`;
    setNotificationAction(key, fn);
    return key;
}

/** ☰ → Reopen closed window: the only one at once, or a notice to choose from
 *  the newest. */
export async function chooseClosedWindow(): Promise<void> {
    let closed: ClosedWindow[];
    try {
        closed = (await WindowService.ListClosedWindows()) ?? [];
    } catch (err) {
        notify("error", "Couldn't list the closed windows", errorText(err));
        return;
    }
    if (closed.length === 0) {
        notify("info", "No closed window to reopen", "A window you close keeps its tabs and panes, and is listed here.");
        return;
    }
    if (closed.length === 1) {
        await reopenClosedWindow(closed[0]);
        return;
    }
    const now = new Date();
    const shown = closed.slice(0, CHOOSER_MAX);
    pushNotification({
        id: "reopen-closed-window",
        persistent: true,
        type: "info",
        icon: "window-restore",
        title: "Reopen a closed window",
        message: shown.map((w) => closedWindowLabel(w, now)).join("\n"),
        timestamp: now.toISOString(),
        actions: [
            ...shown.map((w) => ({
                label: `Reopen ${w.name.trim() || whenAt(w.closed_at_ms, "closed", now)}`,
                actionKey: action(() => void reopenClosedWindow(w)),
                color: "green" as const,
            })),
            { label: "Not now", actionKey: action(() => {}), color: "grey" },
        ],
    });
}
