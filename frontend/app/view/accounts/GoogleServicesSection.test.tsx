// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A Google account's services, its agents' levels and its activity
// (docs/specs/SPEC_GOOGLE_CONNECTORS_GMAIL_CALENDAR_DRIVE_2026_10_14.md §7, D4, D7).

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const google = vi.hoisted(() => vi.fn());
const api = vi.hoisted(() => ({ value: { approvals: { google } } as Record<string, unknown> }));
vi.mock("@/app/store/app-api", () => ({ getApi: () => api.value }));
const rpc = vi.hoisted(() => ({ poll: vi.fn(), cancel: vi.fn() }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { AccountOAuthPollCommand: rpc.poll, AccountOAuthCancelCommand: rpc.cancel },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import type { Account } from "@/app/view/identity/identity-model";
import { callLine, pickDriveFiles, SERVICES, serviceState } from "./google-services";
import { GoogleServicesSection } from "./GoogleServicesSection";

const P = "https://www.googleapis.com/auth/";

function account(scopes: string[], clientMode?: "byo" | "workspace" | "builtin"): Account {
    return {
        id: "g1",
        name: "google",
        provider: "google",
        kind: "oauth",
        display_name: "me@example.com",
        secret_ref: { backend: "keychain", service: "agentmux", account: "g1" },
        context: {
            email: "me@example.com",
            granted_scopes: ["openid", ...scopes.map((s) => P + s)],
            ...(clientMode ? { oauth_client: { mode: clientMode } } : {}),
        },
        status: "valid",
        created_at: 0,
        updated_at: 0,
    } as unknown as Account;
}

const READ = ["calendar.calendarlist.readonly", "calendar.events.readonly", "calendar.events.freebusy"];
const DRIVE = ["drive.file"];

let panel: Record<string, unknown>;

beforeEach(() => {
    api.value = { approvals: { google } };
    panel = {
        agents: [{ agentId: "def-1", name: "Naki", levels: {} }],
        calls: [{ at: 1_700_000_000_000, agent_id: "def-1", agent_name: "Naki", service: "calendar", action: "list_events", ids: "primary", outcome: "ok" }],
        services: ["calendar", "drive", "gmail"],
    };
    google.mockImplementation(async (op: string) => (op === "account" ? panel : true));
});

afterEach(() => {
    cleanup();
    google.mockReset();
    rpc.poll.mockReset();
    rpc.cancel.mockReset();
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

describe("pickDriveFiles", () => {
    it("reports a refusal from Google and stops asking", async () => {
        google.mockResolvedValue({ session_id: "oauth-2" });
        rpc.poll.mockResolvedValue({ status: "failed", error: "access denied: the consent screen was cancelled or declined" });
        await expect(pickDriveFiles("g1")).rejects.toThrow("access denied");
        expect(rpc.poll).toHaveBeenCalledTimes(1);
    });

    it("cancels the session when the person cancels", async () => {
        google.mockResolvedValue({ session_id: "oauth-3" });
        expect(await pickDriveFiles("g1", () => true)).toEqual([]);
        expect(rpc.cancel).toHaveBeenCalledWith(expect.anything(), { sessionId: "oauth-3" });
        expect(rpc.poll).not.toHaveBeenCalled();
    });
});

describe("GoogleServicesSection", () => {
    it("lists the bound agents and sets a level through the host", async () => {
        render(() => <GoogleServicesSection account={account(READ)} />);
        expect(await screen.findByText("Naki: Calendar")).toBeTruthy();
        expect(screen.getByText("On")).toBeTruthy();
        expect(screen.getByText("Naki: list_events (primary)")).toBeTruthy();
        expect(screen.getByText("Naki: Drive")).toBeTruthy();
        fireEvent.click(screen.getAllByRole("radio", { name: "Read" })[0]);
        await waitFor(() => expect(google).toHaveBeenCalledWith("set", { account_id: "g1", agent_id: "def-1", service: "calendar", level: "read" }));
    });

    it("offers Turn on for a service the account hasn't granted", async () => {
        render(() => <GoogleServicesSection account={account([])} />);
        expect(await screen.findByRole("button", { name: "Turn on Calendar" })).toBeTruthy();
        expect(screen.getByRole("button", { name: "Turn on Drive" })).toBeTruthy();
        expect(screen.getAllByText("Off").length).toBe(3);
    });

    it("offers Gmail only on a client the person brought", async () => {
        const { unmount } = render(() => <GoogleServicesSection account={account([], "byo")} />);
        expect(await screen.findByRole("button", { name: "Turn on Gmail" })).toBeTruthy();
        unmount();
        render(() => <GoogleServicesSection account={account([], "workspace")} />);
        expect(await screen.findByRole("button", { name: "Turn on Gmail" })).toBeTruthy();
    });

    it("offers Gmail writing once reading is on, with its own consent", async () => {
        render(() => <GoogleServicesSection account={account(["gmail.readonly"], "byo")} />);
        expect(await screen.findByText("Drafts, labels and sending")).toBeTruthy();
        expect(screen.getByRole("button", { name: "Allow drafts, labels and sending" })).toBeTruthy();
        expect(screen.getByText(/nothing happens until you approve it/)).toBeTruthy();
    });

    it("doesn't offer Gmail writing before reading is on, or once it is granted", async () => {
        const { unmount } = render(() => <GoogleServicesSection account={account([], "byo")} />);
        expect(await screen.findByRole("button", { name: "Turn on Gmail" })).toBeTruthy();
        expect(screen.queryByText("Drafts, labels and sending")).toBeNull();
        unmount();
        render(() => <GoogleServicesSection account={account(["gmail.readonly", "gmail.compose", "gmail.modify"], "byo")} />);
        expect(await screen.findByText("Drafts, labels and sending")).toBeTruthy();
        expect(screen.queryByRole("button", { name: /Allow drafts/ })).toBeNull();
    });

    it("gives an agent the Read and write level on Gmail only, through the host", async () => {
        panel.agents = [{ agentId: "def-1", name: "Naki", levels: { gmail: "read" } }];
        render(() => <GoogleServicesSection account={account(["gmail.readonly"], "byo")} />);
        const gmail = await screen.findByRole("radiogroup", { name: "Gmail for Naki" });
        expect(within(gmail).getByRole("radio", { name: "Read" }).getAttribute("aria-checked")).toBe("true");
        fireEvent.click(within(gmail).getByRole("radio", { name: "Read and write" }));
        await waitFor(() => expect(google).toHaveBeenCalledWith("set", { account_id: "g1", agent_id: "def-1", service: "gmail", level: "write" }));
        expect(within(screen.getByRole("radiogroup", { name: "Calendar for Naki" })).queryByRole("radio", { name: "Read and write" })).toBeNull();
    });

    it("says why Gmail can't be turned on with AgentMux's own client", async () => {
        render(() => <GoogleServicesSection account={account([], "builtin")} />);
        expect(await screen.findByText(/Gmail needs your own Google client/)).toBeTruthy();
        expect(screen.queryByRole("button", { name: "Turn on Gmail" })).toBeNull();
        expect(screen.getByRole("button", { name: "Turn on Calendar" })).toBeTruthy();
    });

    it("shows srv's refusal", async () => {
        google.mockImplementation(async (op: string) => {
            if (op === "account") return panel;
            throw new Error("Naki isn't bound to this account: bind it first");
        });
        render(() => <GoogleServicesSection account={account(READ)} />);
        fireEvent.click((await screen.findAllByRole("radio", { name: "Read" }))[0]);
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

    it("offers the Drive picker only once Drive is on", async () => {
        const { unmount } = render(() => <GoogleServicesSection account={account(READ)} />);
        expect(await screen.findByText("Naki: Drive")).toBeTruthy();
        expect(screen.queryByRole("button", { name: /Give agents files from Drive/ })).toBeNull();
        unmount();
        render(() => <GoogleServicesSection account={account([...READ, ...DRIVE])} />);
        expect(await screen.findByRole("button", { name: /Give agents files from Drive/ })).toBeTruthy();
    });

    it("opens the picker and says how many files the person gave", async () => {
        google.mockImplementation(async (op: string) => (op === "account" ? panel : op === "pick" ? { session_id: "oauth-1" } : true));
        rpc.poll.mockResolvedValue({ status: "picked", accountId: "g1", fileIds: ["a", "b"] });
        render(() => <GoogleServicesSection account={account([...READ, ...DRIVE])} />);
        fireEvent.click(await screen.findByRole("button", { name: /Give agents files from Drive/ }));
        expect(await screen.findByText(/Gave agents 2 files/)).toBeTruthy();
        expect(google).toHaveBeenCalledWith("pick", { account_id: "g1" });
        expect(rpc.poll).toHaveBeenCalledWith(expect.anything(), { sessionId: "oauth-1" });
    });

    it("shows why the picker failed", async () => {
        google.mockImplementation(async (op: string) => {
            if (op === "account") return panel;
            throw new Error("turn Drive on for this account first");
        });
        render(() => <GoogleServicesSection account={account([...READ, ...DRIVE])} />);
        fireEvent.click(await screen.findByRole("button", { name: /Give agents files from Drive/ }));
        expect((await screen.findByRole("alert")).textContent).toContain("turn Drive on");
    });

    it("asks for the desktop app where there is no host", async () => {
        api.value = {};
        render(() => <GoogleServicesSection account={account(READ)} />);
        expect(screen.getByText(/Open AgentMux's desktop app/)).toBeTruthy();
        expect(google).not.toHaveBeenCalled();
    });
});
