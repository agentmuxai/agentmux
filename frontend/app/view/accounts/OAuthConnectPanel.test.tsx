// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Connect Google (docs/specs/SPEC_GOOGLE_CONNECTORS_GMAIL_CALENDAR_DRIVE_2026_10_14.md
// §7 "Connect (new account)"): the saved client is used without retyping, the
// BYO fields show once with the console steps, and a Reconnect re-runs the
// consent for the same account.

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const rpc = vi.hoisted(() => ({
    AccountOAuthClientGetCommand: vi.fn(),
    AccountOAuthClientClearCommand: vi.fn(),
    AccountOAuthStartCommand: vi.fn(),
    AccountOAuthPollCommand: vi.fn(),
    AccountOAuthCancelCommand: vi.fn(),
    ListIdentityAccountsCommand: vi.fn(),
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: rpc }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));
const openLink = vi.hoisted(() => vi.fn());
vi.mock("@/app/store/global", () => ({ openLink }));
vi.mock("@/util/clipboard", () => ({ writeText: vi.fn() }));

import { OAuthConnectPanel } from "./OAuthConnectPanel";
import { oauthNote, setOAuthNote } from "./oauth-account-state";

const CAL = "https://www.googleapis.com/auth/calendar.events";
const BLOCKED =
    "blocked by your organization: a google Workspace administrator doesn't allow this app (admin_policy_enforced). " +
    "Ask the administrator to allow it, or use a Workspace internal client.";

const NOT_SAVED = { saved: false, hasSecret: false, builtinAvailable: false };
const SAVED = { saved: true, clientId: "123-abc.apps.googleusercontent.com", hasSecret: true, mode: "byo", builtinAvailable: false };

beforeEach(() => {
    rpc.AccountOAuthClientGetCommand.mockResolvedValue(NOT_SAVED);
    rpc.AccountOAuthClientClearCommand.mockResolvedValue({ cleared: true });
    rpc.AccountOAuthStartCommand.mockResolvedValue({
        sessionId: "s1",
        status: { status: "url-available", authUrl: "https://accounts.google.com/o/oauth2/v2/auth?x" },
    });
    rpc.AccountOAuthPollCommand.mockResolvedValue({ status: "pending" });
    rpc.AccountOAuthCancelCommand.mockResolvedValue({});
    rpc.ListIdentityAccountsCommand.mockResolvedValue([]);
});
afterEach(() => {
    cleanup();
    vi.clearAllMocks();
    setOAuthNote("acc-1", null);
});

const connectButton = () => screen.getByRole("button", { name: "Connect Google" });

describe("Connect Google", () => {
    it("uses the saved client without asking for it again", async () => {
        rpc.AccountOAuthClientGetCommand.mockResolvedValue(SAVED);
        render(() => <OAuthConnectPanel provider="google" onConnected={() => {}} />);
        await screen.findByText("Using your saved client");
        expect(screen.getByText(SAVED.clientId)).toBeTruthy();
        expect(screen.getByText("Your own client")).toBeTruthy();
        expect(screen.queryByPlaceholderText(/apps\.googleusercontent\.com/)).toBeNull();

        fireEvent.click(connectButton());
        await waitFor(() => expect(rpc.AccountOAuthStartCommand).toHaveBeenCalledTimes(1));
        const req = rpc.AccountOAuthStartCommand.mock.calls[0][1];
        // No client in the request: srv uses the saved one. No name needed:
        // the sign-in's email names the account.
        expect(req).toEqual({ provider: "google", name: undefined, accountId: undefined, scopes: undefined });
        await waitFor(() => expect(openLink).toHaveBeenCalledWith("https://accounts.google.com/o/oauth2/v2/auth?x"));
    });

    it("shows the BYO fields once, with the console steps, and sends the client with its mode", async () => {
        render(() => <OAuthConnectPanel provider="google" onConnected={() => {}} />);
        await screen.findByText(/application type “Desktop app”/);
        const id = screen.getByPlaceholderText(/apps\.googleusercontent\.com/) as HTMLInputElement;
        const secret = screen.getByPlaceholderText(/stored in your OS keychain/) as HTMLInputElement;

        // Google's Desktop client needs its secret for the token exchange.
        fireEvent.input(id, { target: { value: " 123-abc.apps.googleusercontent.com " } });
        fireEvent.click(connectButton());
        expect(await screen.findByText("Client secret is required")).toBeTruthy();
        expect(rpc.AccountOAuthStartCommand).not.toHaveBeenCalled();

        fireEvent.input(secret, { target: { value: "GOCSPX-shh" } });
        fireEvent.click(screen.getByRole("radio", { name: "Workspace internal" }));
        fireEvent.click(connectButton());
        await waitFor(() => expect(rpc.AccountOAuthStartCommand).toHaveBeenCalledTimes(1));
        expect(rpc.AccountOAuthStartCommand.mock.calls[0][1]).toMatchObject({
            provider: "google",
            clientId: "123-abc.apps.googleusercontent.com",
            clientSecret: "GOCSPX-shh",
            clientMode: "workspace",
        });
        // The secret leaves the frontend with the request; nothing keeps it.
        expect(secret.value).toBe("");
    });

    it("offers a different client, and forgets the saved one", async () => {
        rpc.AccountOAuthClientGetCommand.mockResolvedValueOnce(SAVED).mockResolvedValueOnce(NOT_SAVED);
        render(() => <OAuthConnectPanel provider="google" onConnected={() => {}} />);
        await screen.findByText("Using your saved client");

        fireEvent.click(screen.getByRole("button", { name: "Use a different client" }));
        expect(screen.getByPlaceholderText(/apps\.googleusercontent\.com/)).toBeTruthy();
        fireEvent.click(screen.getByRole("button", { name: "Use the saved client" }));
        expect(screen.queryByPlaceholderText(/apps\.googleusercontent\.com/)).toBeNull();

        fireEvent.click(screen.getByRole("button", { name: "Forget this client" }));
        await waitFor(() => expect(rpc.AccountOAuthClientClearCommand).toHaveBeenCalledWith({}, { provider: "google" }));
        await screen.findByPlaceholderText(/apps\.googleusercontent\.com/);
        expect(screen.queryByText("Using your saved client")).toBeNull();
    });

    it("reports a partial grant for the new account", async () => {
        rpc.AccountOAuthClientGetCommand.mockResolvedValue(SAVED);
        rpc.AccountOAuthStartCommand.mockResolvedValue({
            sessionId: "s1",
            status: { status: "success", accountId: "acc-1", grantedScopes: ["openid"], missingScopes: [CAL], email: "me@example.com" },
        });
        const onConnected = vi.fn();
        render(() => <OAuthConnectPanel provider="google" onConnected={onConnected} />);
        await screen.findByText("Using your saved client");
        fireEvent.click(connectButton());
        await waitFor(() => expect(onConnected).toHaveBeenCalledTimes(1));
        expect(onConnected.mock.calls[0][0]).toEqual({ accountId: "acc-1", missingScopes: [CAL], email: "me@example.com" });
        expect(oauthNote("acc-1")?.missingScopes).toEqual([CAL]);
    });

    it("says when the organization blocks the app", async () => {
        rpc.AccountOAuthClientGetCommand.mockResolvedValue(SAVED);
        rpc.AccountOAuthStartCommand.mockResolvedValue({ sessionId: "s1", status: { status: "failed", error: BLOCKED } });
        render(() => <OAuthConnectPanel provider="google" onConnected={() => {}} />);
        await screen.findByText("Using your saved client");
        fireEvent.click(connectButton());
        expect(await screen.findByText("Blocked by your organization")).toBeTruthy();
        expect(screen.getByText(/Ask the administrator to allow it/)).toBeTruthy();
        expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
    });
});

describe("Reconnect", () => {
    it("starts by itself for the same account and the scopes it had", async () => {
        rpc.AccountOAuthClientGetCommand.mockResolvedValue(SAVED);
        render(() => (
            <OAuthConnectPanel provider="google" accountId="acc-1" scopes={[CAL]} autoStart onConnected={() => {}} />
        ));
        await waitFor(() => expect(rpc.AccountOAuthStartCommand).toHaveBeenCalledTimes(1));
        expect(rpc.AccountOAuthStartCommand.mock.calls[0][1]).toEqual({
            provider: "google",
            name: undefined,
            accountId: "acc-1",
            scopes: [CAL],
        });
    });

    it("waits for the client when none is saved, prefilled with the account's", async () => {
        render(() => (
            <OAuthConnectPanel
                provider="google"
                accountId="acc-1"
                autoStart
                clientIdHint="123-abc.apps.googleusercontent.com"
                onConnected={() => {}}
            />
        ));
        const id = (await screen.findByPlaceholderText(/apps\.googleusercontent\.com/)) as HTMLInputElement;
        expect(id.value).toBe("123-abc.apps.googleusercontent.com");
        expect(rpc.AccountOAuthStartCommand).not.toHaveBeenCalled();
        expect(screen.getByRole("button", { name: "Reconnect" })).toBeTruthy();
    });

    it("marks the account blocked when its organization refuses", async () => {
        rpc.AccountOAuthClientGetCommand.mockResolvedValue(SAVED);
        rpc.AccountOAuthStartCommand.mockResolvedValue({ sessionId: "s1", status: { status: "failed", error: BLOCKED } });
        render(() => <OAuthConnectPanel provider="google" accountId="acc-1" autoStart onConnected={() => {}} />);
        await screen.findByText("Blocked by your organization");
        expect(oauthNote("acc-1")?.blocked).toBe(BLOCKED);
    });
});

describe("other providers", () => {
    it("GitHub asks for a name and a client id only, with no client type", async () => {
        const name = () => "";
        render(() => <OAuthConnectPanel provider="github" name={name} onConnected={() => {}} />);
        await screen.findByPlaceholderText("Iv1.0123456789abcdef");
        expect(screen.queryByPlaceholderText(/stored in your OS keychain/)).toBeNull();
        expect(screen.queryByRole("radio", { name: "Workspace internal" })).toBeNull();
        fireEvent.click(screen.getByRole("button", { name: "Connect GitHub" }));
        expect(await screen.findByText("Enter a name first")).toBeTruthy();
        expect(rpc.AccountOAuthStartCommand).not.toHaveBeenCalled();
    });

    it("GitHub sends its client without a mode", async () => {
        render(() => <OAuthConnectPanel provider="github" name={() => "work"} onConnected={() => {}} />);
        fireEvent.input(await screen.findByPlaceholderText("Iv1.0123456789abcdef"), { target: { value: "Iv1.abc" } });
        fireEvent.click(screen.getByRole("button", { name: "Connect GitHub" }));
        await waitFor(() => expect(rpc.AccountOAuthStartCommand).toHaveBeenCalledTimes(1));
        expect(rpc.AccountOAuthStartCommand.mock.calls[0][1]).toEqual({
            provider: "github",
            name: "work",
            accountId: undefined,
            scopes: undefined,
            clientId: "Iv1.abc",
            clientSecret: undefined,
            clientMode: undefined,
        });
    });
});
