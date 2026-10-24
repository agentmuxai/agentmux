// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A Google account's services and who may use them
// (docs/specs/SPEC_GOOGLE_CONNECTORS_GMAIL_CALENDAR_DRIVE_2026_10_14.md D4, D7,
// §7 "Turn a service on" and "Give agents access"). Grants are changed only
// through the host (`approvals.google`), never an RPC an agent could call.

import { getApi } from "@/app/store/app-api";

const PREFIX = "https://www.googleapis.com/auth/";

export type GoogleService = "calendar";
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
