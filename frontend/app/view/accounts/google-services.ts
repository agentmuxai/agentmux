// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A Google account's services and who may use them
// (docs/specs/SPEC_GOOGLE_CONNECTORS_GMAIL_CALENDAR_DRIVE_2026_10_14.md D4, D7,
// §7 "Turn a service on" and "Give agents access"). Grants are changed only
// through the host (`approvals.google`), never an RPC an agent could call.

import { getApi } from "@/app/store/app-api";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

const PREFIX = "https://www.googleapis.com/auth/";

export type GoogleService = "calendar" | "drive";
export type GrantLevel = "read" | "write";

export interface ServiceInfo {
    service: GoogleService;
    label: string;
    /** What turning it on lets agents do, in one line. */
    line: string;
    /** The scopes "Turn on" asks for (srv's `Service::scopes` at read). */
    readScopes: string[];
}

export const SERVICES: ServiceInfo[] = [
    {
        service: "calendar",
        label: "Calendar",
        line: "Agents you allow can read your calendars, events and free/busy times.",
        readScopes: ["calendar.calendarlist.readonly", "calendar.events.readonly", "calendar.events.freebusy"].map(
            (s) => PREFIX + s
        ),
    },
    {
        service: "drive",
        label: "Drive",
        line: "Agents you allow can read the files you pick for them in Google's file picker, and the ones AgentMux makes. Never the rest of your Drive.",
        readScopes: [PREFIX + "drive.file"],
    },
];

/** Whether the account's token covers a service: all, some or none of its scopes. */
export function serviceState(info: ServiceInfo, granted: readonly string[]): "on" | "partly" | "off" {
    const have = info.readScopes.filter((s) => granted.includes(s)).length;
    if (have === info.readScopes.length) return "on";
    return have === 0 ? "off" : "partly";
}

export interface PanelAgent {
    agentId: string;
    name: string;
    levels: Partial<Record<GoogleService, GrantLevel>>;
}

export interface GoogleCall {
    at: number;
    agent_id: string;
    agent_name: string;
    service: string;
    action: string;
    ids: string;
    outcome: string;
}

export interface GooglePanel {
    agents: PanelAgent[];
    calls: GoogleCall[];
    services: GoogleService[];
}

/** One log line in words: "Naki: list_events (primary)". */
export function callLine(c: GoogleCall): string {
    const who = c.agent_name || c.agent_id || "An agent";
    return `${who}: ${c.action}${c.ids ? ` (${c.ids})` : ""}`;
}

/** The host relay, absent outside the desktop app. */
function relay(): ((op: string, args?: Record<string, unknown>) => Promise<unknown>) | undefined {
    return getApi()?.approvals?.google;
}

export function canManageGrants(): boolean {
    return typeof relay() === "function";
}

export async function loadGooglePanel(accountId: string): Promise<GooglePanel> {
    const r = relay();
    if (!r) throw new Error("Open AgentMux's desktop app to choose which agents use this account.");
    const data = (await r("account", { account_id: accountId })) as Partial<GooglePanel> | null;
    return { agents: data?.agents ?? [], calls: data?.calls ?? [], services: data?.services ?? [] };
}

export async function setGoogleGrant(
    accountId: string,
    agentId: string,
    service: GoogleService,
    level: GrantLevel | "off"
): Promise<void> {
    const r = relay();
    if (!r) throw new Error("Open AgentMux's desktop app to choose which agents use this account.");
    await r("set", { account_id: accountId, agent_id: agentId, service, level });
}


/** How often the picker's session is asked whether the person has chosen, and for how long. */
const PICK_POLL_MS = 1500;
const PICK_WAIT_MS = 6 * 60 * 1000;

/**
 * Open Google's file picker for `accountId` in the system browser and wait for the person to
 * choose. Resolves with the ids picked (none if they closed it without choosing); rejects with
 * a sentence when it can't start or Google refuses. `cancelled()` is asked between polls.
 */
export async function pickDriveFiles(accountId: string, cancelled: () => boolean = () => false): Promise<string[]> {
    const r = relay();
    if (!r) throw new Error("Open AgentMux's desktop app to give agents files from Drive.");
    const started = (await r("pick", { account_id: accountId })) as { session_id?: string } | null;
    const sessionId = started?.session_id;
    if (!sessionId) throw new Error("Google's file picker didn't open.");
    const until = Date.now() + PICK_WAIT_MS;
    while (Date.now() < until) {
        if (cancelled()) {
            void RpcApi.AccountOAuthCancelCommand(TabRpcClient, { sessionId });
            return [];
        }
        const s = (await RpcApi.AccountOAuthPollCommand(TabRpcClient, { sessionId })) as { status: string; fileIds?: string[]; error?: string };
        if (s.status === "picked") return s.fileIds ?? [];
        if (s.status === "failed") throw new Error(s.error || "Google's file picker failed.");
        await new Promise((resolve) => setTimeout(resolve, PICK_POLL_MS));
    }
    void RpcApi.AccountOAuthCancelCommand(TabRpcClient, { sessionId });
    throw new Error("Nothing was picked in time. Try again.");
}
