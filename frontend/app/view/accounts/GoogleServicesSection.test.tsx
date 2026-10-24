// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A Google account's services, its agents' levels and its activity
// (docs/specs/SPEC_GOOGLE_CONNECTORS_GMAIL_CALENDAR_DRIVE_2026_10_14.md §7, D4, D7).

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const google = vi.hoisted(() => vi.fn());
const api = vi.hoisted(() => ({ value: { approvals: { google } } as Record<string, unknown> }));
vi.mock("@/app/store/app-api", () => ({ getApi: () => api.value }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import type { Account } from "@/app/view/identity/identity-model";
import { callLine, SERVICES, serviceState } from "./google-services";
import { GoogleServicesSection } from "./GoogleServicesSection";

const P = "https://www.googleapis.com/auth/";

function account(scopes: string[]): Account {
    return {
        id: "g1",
        name: "google",
        provider: "google",
        kind: "oauth",
        display_name: "me@example.com",
        secret_ref: { backend: "keychain", service: "agentmux", account: "g1" },
        context: { email: "me@example.com", granted_scopes: ["openid", ...scopes.map((s) => P + s)] },
        status: "valid",
        created_at: 0,
        updated_at: 0,
    } as unknown as Account;
}

const READ = ["calendar.calendarlist.readonly", "calendar.events.readonly", "calendar.events.freebusy"];

let panel: Record<string, unknown>;

beforeEach(() => {
    api.value = { approvals: { google } };
    panel = {
        agents: [{ agentId: "def-1", name: "Naki", levels: {} }],
        calls: [{ at: 1_700_000_000_000, agent_id: "def-1", agent_name: "Naki", service: "calendar", action: "list_events", ids: "primary", outcome: "ok" }],
        services: ["calendar"],
    };
    google.mockImplementation(async (op: string) => (op === "account" ? panel : true));
});

afterEach(() => {
    cleanup();
    google.mockReset();
});

describe("google-services", () => {
    it("says whether the token covers a service", () => {
        const cal = SERVICES[0];
        expect(serviceState(cal, READ.map((s) => P + s))).toBe("on");
        expect(serviceState(cal, [P + "calendar.events.readonly"])).toBe("partly");
        expect(serviceState(cal, ["openid"])).toBe("off");
    });

    it("writes a log line in words", () => {
        expect(callLine({ at: 0, agent_id: "d", agent_name: "Naki", service: "calendar", action: "free_busy", ids: "primary,a@b", outcome: "ok" })).toBe(
            "Naki: free_busy (primary,a@b)"
        );
        expect(callLine({ at: 0, agent_id: "", agent_name: "", service: "calendar", action: "list_calendars", ids: "", outcome: "ok" })).toBe(
            "An agent: list_calendars"
        );
    });
});

describe("GoogleServicesSection", () => {
    it("lists the bound agents and sets a level through the host", async () => {
        render(() => <GoogleServicesSection account={account(READ)} />);
        expect(await screen.findByText("Naki: Calendar")).toBeTruthy();
        expect(screen.getByText("On")).toBeTruthy();
        expect(screen.getByText("Naki: list_events (primary)")).toBeTruthy();
        fireEvent.click(screen.getByRole("radio", { name: "Read" }));
        await waitFor(() => expect(google).toHaveBeenCalledWith("set", { account_id: "g1", agent_id: "def-1", service: "calendar", level: "read" }));
    });

    it("offers Turn on for a service the account hasn't granted", async () => {
        render(() => <GoogleServicesSection account={account([])} />);
        expect(await screen.findByRole("button", { name: "Turn on Calendar" })).toBeTruthy();
        expect(screen.getByText("Off")).toBeTruthy();
    });

    it("shows srv's refusal", async () => {
        google.mockImplementation(async (op: string) => {
            if (op === "account") return panel;
            throw new Error("Naki isn't bound to this account: bind it first");
        });
        render(() => <GoogleServicesSection account={account(READ)} />);
        fireEvent.click(await screen.findByRole("radio", { name: "Read" }));
        expect((await screen.findByRole("alert")).textContent).toContain("bind it first");
    });

    it("shows a failed load instead of throwing", async () => {
        google.mockImplementation(async () => {
            throw new Error("srv refused: only the AgentMux host can change who uses a Google account");
        });
        render(() => <GoogleServicesSection account={account(READ)} />);
        expect((await screen.findByRole("alert")).textContent).toContain("only the AgentMux host");
        expect(screen.queryByText(/No agent is bound/)).toBeNull();
        expect(screen.getByText("Services")).toBeTruthy();
    });

    it("asks for the desktop app where there is no host", async () => {
        api.value = {};
        render(() => <GoogleServicesSection account={account(READ)} />);
        expect(screen.getByText(/Open AgentMux's desktop app/)).toBeTruthy();
        expect(google).not.toHaveBeenCalled();
    });
});
