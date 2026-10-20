// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Restoring a layout another build of AgentMux saved (srv
// `session_other_builds.rs`): every build keeps its own data, so a different
// or newer build starts with the default layout while the previous build's
// layout waits in that build's data. A window that came up with the default
// because nothing was restored offers the newest one in a notice; ☰ → Layouts
// → "Restore layout from another build…" lists them any time.

import { reopenRestoredWindows } from "@/app/init/restored-windows";
import { setNotificationAction } from "@/app/notification/usenotification";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { windowId } from "@/app/store/window-identity";
import { pushNotification } from "@/store/global";
import type { OtherBuildLayout } from "@/types/rpc/OtherBuildLayout";

/** Older layouts are not offered unasked; the menu still lists them. */
export const OFFER_MAX_AGE_MS = 14 * 24 * 60 * 60 * 1000;

/** The notice's id: the offer and the menu's list share it, so one replaces the other. */
export const NOTICE_ID = "restore-other-build";

/** The most layouts the menu's notice offers a button for. */
const MENU_MAX = 4;

const pad = (n: number): string => String(n).padStart(2, "0");

/** "closed 03:28 today", "closed yesterday at 22:10", "closed Oct 17 at 22:10".
 *  A layout whose save time isn't recorded says when its build last ran. */
export function whenSaved(l: OtherBuildLayout, now: Date = new Date()): string {
    const at = new Date(l.saved_at_ms);
    const verb = l.saved_at_exact ? "closed" : "last used";
    const time = `${pad(at.getHours())}:${pad(at.getMinutes())}`;
    const startOfDay = (d: Date): number => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
    const days = Math.round((startOfDay(now) - startOfDay(at)) / 86_400_000);
    if (days === 0) return `${verb} ${time} today`;
    if (days === 1) return `${verb} yesterday at ${time}`;
    const date = at.toLocaleDateString(undefined, { month: "short", day: "numeric" });
    return `${verb} ${date} at ${time}`;
}

const count = (n: number, one: string): string => `${n} ${one}${n === 1 ? "" : "s"}`;

/** "4 tabs, 10 agents", with the windows when there are several. */
export function layoutSummary(l: OtherBuildLayout): string {
    const parts = [count(l.tabs, "tab")];
    if (l.windows > 1) parts.unshift(count(l.windows, "window"));
    parts.push(l.agents.length > 0 ? count(l.agents.length, "agent") : count(l.panes, "pane"));
    return parts.join(", ");
}

function agentsLine(l: OtherBuildLayout): string {
    if (l.agents.length === 0) return "";
    const shown = l.agents.slice(0, 6).join(", ");
    return l.agents.length > 6 ? `${shown} and ${l.agents.length - 6} more` : shown;
}

function notify(type: "info" | "warning" | "error", title: string, message: string): void {
    pushNotification({
        icon: type === "error" ? "triangle-exclamation" : "clock-rotate-left",
        title,
        message,
        timestamp: new Date().toISOString(),
        type,
        expiration: Date.now() + (type === "info" ? 6000 : 15000),
    });
}

const errorText = (err: unknown): string => (err instanceof Error ? err.message : String(err));

let actionSeq = 0;

/** An action key for a notice button that runs `fn`. */
function action(fn: () => void): string {
    const key = `restoreOtherBuild:${++actionSeq}`;
    setNotificationAction(key, fn);
    return key;
}

/** Rebuild `l` in this window. */
export async function restoreLayoutFromOtherBuild(l: OtherBuildLayout): Promise<void> {
    let result;
    try {
        result = await RpcApi.RestoreOtherBuildLayoutCommand(TabRpcClient, {
            window_id: windowId(),
            channel: l.channel,
            version: l.version,
        });
    } catch (err) {
        notify("error", `Couldn't restore the layout from ${l.version}`, errorText(err));
        return;
    }
    if (result.other_windows > 0) {
        await reopenRestoredWindows();
    }
    const added = result.replaced_default ? "" : " beside your tabs";
    notify("info", `Restored the layout from ${l.version}`, `${count(result.tab_ids.length, "tab")}${added}.`);
}

async function listLayouts(): Promise<{ offer: boolean; layouts: OtherBuildLayout[] }> {
    return RpcApi.OtherBuildLayoutsCommand(TabRpcClient, { window_id: windowId() });
}

/** At a cold start: offer the newest recent layout another build saved, when
 *  this window shows the default layout because nothing was restored. */
export async function offerLayoutFromOtherBuild(now: number = Date.now()): Promise<void> {
    let found;
    try {
        found = await listLayouts();
    } catch (err) {
        console.warn("[restore-other-build] couldn't look for other builds' layouts", err);
        return;
    }
    const l = found.offer ? found.layouts.find((c) => now - c.saved_at_ms <= OFFER_MAX_AGE_MS) : undefined;
    if (!l) return;
    const agents = agentsLine(l);
    pushNotification({
        id: NOTICE_ID,
        persistent: true,
        type: "info",
        icon: "clock-rotate-left",
        title: `Restore the layout from ${l.version} (${whenSaved(l, new Date(now))}): ${layoutSummary(l)}?`,
        message: `${agents ? `${agents}. ` : ""}This build had no layout of its own to restore.`,
        timestamp: new Date(now).toISOString(),
        actions: [
            { label: "Restore", actionKey: action(() => void restoreLayoutFromOtherBuild(l)), color: "green" },
            { label: "Not now", actionKey: action(() => {}), color: "grey" },
        ],
    });
}

/** ☰ → Layouts → "Restore layout from another build…": a notice listing the
 *  layouts other builds saved, newest first, each with its Restore button. */
export async function chooseLayoutFromOtherBuild(): Promise<void> {
    let found;
    try {
        found = await listLayouts();
    } catch (err) {
        notify("error", "Couldn't look for other builds' layouts", errorText(err));
        return;
    }
    const layouts = found.layouts.slice(0, MENU_MAX);
    if (layouts.length === 0) {
        notify("info", "No other build has a saved layout", "A build saves its layout when its last window closes.");
        return;
    }
    const now = new Date();
    const versions = layouts.map((l) => l.version);
    const label = (l: OtherBuildLayout): string =>
        versions.filter((v) => v === l.version).length > 1 ? `${l.version}, ${whenSaved(l, now)}` : l.version;
    pushNotification({
        id: NOTICE_ID,
        persistent: true,
        type: "info",
        icon: "clock-rotate-left",
        title: "Restore a layout from another build",
        message: layouts.map((l) => `${l.version} (${whenSaved(l, now)}): ${layoutSummary(l)}`).join("\n"),
        timestamp: now.toISOString(),
        actions: [
            ...layouts.map((l) => ({
                label: `Restore ${label(l)}`,
                actionKey: action(() => void restoreLayoutFromOtherBuild(l)),
                color: "green" as const,
            })),
            { label: "Not now", actionKey: action(() => {}), color: "grey" },
        ],
    });
}
