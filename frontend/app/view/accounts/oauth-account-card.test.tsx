// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A Google account's card and details
// (docs/specs/SPEC_GOOGLE_CONNECTORS_GMAIL_CALENDAR_DRIVE_2026_10_14.md §7
// "Status on the card", "Some unticked", "Disconnect").

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const rpc = vi.hoisted(() => ({
    ListIdentityAccountsCommand: vi.fn(),
    ListAllAgentIdentitiesCommand: vi.fn(),
    ListAgentDefinitionsCommand: vi.fn(),
    MuxBusStatusCommand: vi.fn(),
    MuxBusCloudConfigCommand: vi.fn(),
    AccountOAuthClientGetCommand: vi.fn(),
    AccountOAuthStartCommand: vi.fn(),
    AccountOAuthPollCommand: vi.fn(),
    AccountOAuthCancelCommand: vi.fn(),
    DeleteIdentityAccountCommand: vi.fn(),
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: rpc }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));
vi.mock("@/app/view/identity/bind-to-agent-menu", () => ({ buildAccountRowMenu: vi.fn() }));

import type { BlockNodeModel } from "@/app/block/blocktypes";
import { AccountDeleteNotice } from "@/app/view/identity/identity-accounts-tab";
import { IdentityViewModel, type Account } from "@/app/view/identity/identity-model";
import { AccountsManager } from "./accounts-manager";
import { setOAuthNote } from "./oauth-account-state";

const CAL = "https://www.googleapis.com/auth/calendar.events";

function google(over: Record<string, unknown> = {}) {
    return {
        id: "g1",
        name: "google",
        provider: "google",
        kind: "oauth",
        // srv sets the display name to the email.
        display_name: "me@example.com",
        secret_ref: { backend: "keychain", service: "agentmux", account: "g1" },
        context: {
            email: "me@example.com",
            granted_scopes: ["openid", "https://www.googleapis.com/auth/userinfo.email", CAL],
            oauth_client: { mode: "byo", client_id: "123-abc.apps.googleusercontent.com" },
        },
        status: "valid",
        created_at: 0,
        updated_at: 0,
        ...over,
    };
}

beforeEach(() => {
    rpc.ListIdentityAccountsCommand.mockResolvedValue([google()]);
    rpc.ListAllAgentIdentitiesCommand.mockResolvedValue([]);
    rpc.ListAgentDefinitionsCommand.mockResolvedValue([]);
    rpc.MuxBusStatusCommand.mockResolvedValue({ connected: false });
    rpc.MuxBusCloudConfigCommand.mockResolvedValue({});
    rpc.AccountOAuthClientGetCommand.mockResolvedValue({
        saved: true,
        clientId: "123-abc.apps.googleusercontent.com",
        hasSecret: true,
        mode: "byo",
        builtinAvailable: false,
    });
    rpc.AccountOAuthStartCommand.mockResolvedValue({ sessionId: "s1", status: { status: "pending" } });
    rpc.AccountOAuthPollCommand.mockResolvedValue({ status: "pending" });
    rpc.AccountOAuthCancelCommand.mockResolvedValue({});
});
afterEach(() => {
    cleanup();
    vi.resetAllMocks();
    setOAuthNote("g1", null);
});

const card = () => screen.getByRole("button", { name: /^me@example\.com/ });

describe("a Google account's card", () => {
    it("shows the email, Connected, and the client it uses", async () => {
        render(() => <AccountsManager />);
        await waitFor(() => expect(card()).toBeTruthy());
        expect(card().querySelector(".ui-card-title")?.textContent).toBe("me@example.com");
        // The display name only repeats the email, so the brand shows instead.
        expect(card().querySelector(".ui-card-subtitle")?.textContent).toBe("Google");
        expect(screen.getByRole("img", { name: "Connected" })).toBeTruthy();
        expect(screen.getByText("Your own client")).toBeTruthy();
        expect(screen.queryByRole("button", { name: "Reconnect" })).toBeNull();
    });

    it("says Reconnect needed, and its Reconnect re-runs the consent for the same account", async () => {
        rpc.ListIdentityAccountsCommand.mockResolvedValue([google({ status: "needs_reauth" })]);
        render(() => <AccountsManager />);
        await waitFor(() => expect(screen.getByRole("img", { name: "Reconnect needed" })).toBeTruthy());
        expect(card().querySelector(".account-card-note")?.textContent).toBe("Reconnect needed");

        fireEvent.click(screen.getByRole("button", { name: "Reconnect" }));
        await waitFor(() => expect(rpc.AccountOAuthStartCommand).toHaveBeenCalledTimes(1));
        expect(rpc.AccountOAuthStartCommand.mock.calls[0][1]).toEqual({
            provider: "google",
            name: undefined,
            accountId: "g1",
            scopes: [CAL],
        });
        // It runs in the details, which show the browser prompt.
        expect(card().getAttribute("aria-expanded")).toBe("true");
        expect(await screen.findByText(/Waiting for browser authorization/)).toBeTruthy();
    });

    it("says which permissions weren't granted", async () => {
        setOAuthNote("g1", { missingScopes: [CAL] });
        render(() => <AccountsManager />);
        await waitFor(() => expect(card()).toBeTruthy());
        expect(card().querySelector(".account-card-note")?.textContent).toBe(
            "Partly granted: Calendar events wasn't allowed",
        );
    });

    it("shows Blocked by your organization after a blocked sign-in", async () => {
        setOAuthNote("g1", { blocked: "blocked by your organization: … (admin_policy_enforced)." });
        render(() => <AccountsManager />);
        await waitFor(() => expect(screen.getByRole("img", { name: "Blocked by your organization" })).toBeTruthy());
        expect(card().classList.contains("is-attention")).toBe(true);
        expect(screen.getByRole("button", { name: "Reconnect" })).toBeTruthy();
    });

    it("keeps Valid for an API key", async () => {
        rpc.ListIdentityAccountsCommand.mockResolvedValue([
            google({ id: "o1", provider: "openai", kind: "api_key", context: { email: "me@example.com" } }),
        ]);
        render(() => <AccountsManager />);
        await waitFor(() => expect(screen.getByRole("img", { name: "Valid" })).toBeTruthy());
        expect(screen.queryByText("Your own client")).toBeNull();
    });
});

describe("Disconnect", () => {
    function modelWith(accounts: Account[]): IdentityViewModel {
        const model = new IdentityViewModel("test", {} as BlockNodeModel);
        (model as unknown as { setAccounts: (a: Account[]) => void }).setAccounts(accounts);
        return model;
    }
    const account = { ...google(), assigned_agents: [], created_at: "0", updated_at: "0" } as unknown as Account;

    it("signs out first and keeps the account when that fails, offering Try again and Remove anyway", async () => {
        rpc.ListIdentityAccountsCommand.mockResolvedValue([google()]);
        rpc.DeleteIdentityAccountCommand.mockResolvedValueOnce({
            deleted: false,
            affectedAgents: [],
            cleanup: null,
            revoke: { outcome: "failed", detail: "network unreachable" },
        });
        const model = modelWith([account]);
        model.requestDelete(account);
        await model.confirmDelete();
        expect(rpc.DeleteIdentityAccountCommand).toHaveBeenCalledWith({}, { id: "g1", require_revoke: true });

        render(() => <AccountDeleteNotice model={model} />);
        expect(
            screen.getByText("Couldn't sign out of Google: network unreachable. Try again, or remove it from AgentMux only."),
        ).toBeTruthy();

        // Try again: the same delete, still requiring the sign-out.
        rpc.DeleteIdentityAccountCommand.mockResolvedValueOnce({
            deleted: false,
            affectedAgents: [],
            cleanup: null,
            revoke: { outcome: "failed", detail: "still offline" },
        });
        fireEvent.click(screen.getByRole("button", { name: "Try again" }));
        await waitFor(() => expect(rpc.DeleteIdentityAccountCommand).toHaveBeenCalledTimes(2));
        expect(rpc.DeleteIdentityAccountCommand.mock.calls[1][1]).toEqual({ id: "g1", require_revoke: true });
        expect(await screen.findByText(/Couldn't sign out of Google: still offline\./)).toBeTruthy();
        const removeAnyway = screen.getByRole("button", { name: "Remove anyway" }) as HTMLButtonElement;
        await waitFor(() => expect(removeAnyway.disabled).toBe(false));

        // Remove anyway: deleted without the sign-out, and said so.
        rpc.DeleteIdentityAccountCommand.mockResolvedValueOnce({
            deleted: true,
            affectedAgents: [],
            cleanup: { outcome: "removed" },
            revoke: { outcome: "failed", detail: "still offline" },
        });
        fireEvent.click(removeAnyway);
        await waitFor(() => expect(rpc.DeleteIdentityAccountCommand).toHaveBeenCalledTimes(3));
        expect(rpc.DeleteIdentityAccountCommand.mock.calls[2][1]).toEqual({ id: "g1" });
        await waitFor(() => expect(screen.queryByText(/Couldn't sign out/)).toBeNull());
        expect(model.deleteNoticeAtom()).toContain("Removed from AgentMux only.");
    });

    it("deletes in one step when the sign-out works", async () => {
        rpc.DeleteIdentityAccountCommand.mockResolvedValue({
            deleted: true,
            affectedAgents: [],
            cleanup: { outcome: "removed" },
            revoke: { outcome: "revoked" },
        });
        const model = modelWith([account]);
        model.requestDelete(account);
        await model.confirmDelete();
        expect(model.revokeFailureAtom()).toBeNull();
        expect(model.deleteNoticeAtom()).toBeNull();
    });
});
