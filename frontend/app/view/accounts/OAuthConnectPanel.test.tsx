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

vi.mock("./GithubCloudSignIn", () => ({ GithubCloudSignIn: () => <div>Sign in to AgentMux Cloud to connect GitHub</div> }));

import { acceptMuxbusDelivery } from "@/store/muxbus-delivery";
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
    it("fills the client id and secret from the JSON Google downloads, and connects with them", async () => {
        render(() => <OAuthConnectPanel provider="google" onConnected={() => {}} />);
        await screen.findByText(/application type “Desktop app”/);
        const json = JSON.stringify({ installed: { client_id: "9-abc.apps.googleusercontent.com", client_secret: "GOCSPX-test" } });
        const file = new File([json], "client_secret_9-abc.json", { type: "application/json" });
        fireEvent.change(screen.getByTestId("client-json-input"), { target: { files: [file] } });
        await screen.findByText(/from the file/);
        expect((screen.getByPlaceholderText(/apps\.googleusercontent\.com/) as HTMLInputElement).value).toBe("9-abc.apps.googleusercontent.com");

        fireEvent.click(connectButton());
        await waitFor(() => expect(rpc.AccountOAuthStartCommand).toHaveBeenCalledTimes(1));
        const req = rpc.AccountOAuthStartCommand.mock.calls[0][1];
        expect(req.clientId).toBe("9-abc.apps.googleusercontent.com");
        expect(req.clientSecret).toBe("GOCSPX-test");
    });

    it("says why a Web client's file or another file can't be used, and fills nothing", async () => {
        render(() => <OAuthConnectPanel provider="google" onConnected={() => {}} />);
        await screen.findByText(/application type “Desktop app”/);
        const input = screen.getByTestId("client-json-input");
        const web = new File([JSON.stringify({ web: { client_id: "1.apps.googleusercontent.com", client_secret: "GOCSPX-x" } })], "web.json");
        fireEvent.change(input, { target: { files: [web] } });
        expect(await screen.findByText(/Web application client/)).toBeTruthy();
        fireEvent.change(input, { target: { files: [new File(["hello"], "notes.json")] } });
        expect(await screen.findByText(/isn't a Google OAuth client file/)).toBeTruthy();
        expect((screen.getByPlaceholderText(/apps\.googleusercontent\.com/) as HTMLInputElement).value).toBe("");
        expect(screen.queryByText(/from the file/)).toBeNull();
    });

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

describe("Connect GitHub", () => {
    const GITHUB_URL = "https://github.com/login/oauth/authorize?x";
    const mismatch = {
        sessionId: "s1",
        status: { status: "confirm-user", currentLogin: "old", currentId: 1, newLogin: "new", newId: 2 },
    };

    beforeEach(() => {
        // AgentMux's own App is configured: no client to ask for.
        rpc.AccountOAuthClientGetCommand.mockResolvedValue({ saved: false, hasSecret: false, builtinAvailable: true });
    });

    it("asks before replacing the GitHub user this account is connected as, then connects again with replace", async () => {
        rpc.AccountOAuthStartCommand.mockResolvedValueOnce(mismatch).mockResolvedValueOnce({
            sessionId: "s2",
            status: { status: "url-available", authUrl: GITHUB_URL },
        });
        render(() => <OAuthConnectPanel provider="github" name={() => "work"} onConnected={() => {}} />);
        fireEvent.click(await screen.findByRole("button", { name: "Connect GitHub" }));
        expect(await screen.findByText("Connect a different GitHub account?")).toBeTruthy();
        // The flow ended at the broker's question: nothing is polled, and nothing waits for a browser.
        expect(rpc.AccountOAuthPollCommand).not.toHaveBeenCalled();
        expect(screen.queryByRole("button", { name: "Cancel" })).toBeNull();

        fireEvent.click(screen.getByRole("button", { name: "Replace with @new" }));
        await waitFor(() => expect(rpc.AccountOAuthStartCommand).toHaveBeenCalledTimes(2));
        expect(rpc.AccountOAuthStartCommand.mock.calls[0][1].replaceGithubUser).toBeUndefined();
        expect(rpc.AccountOAuthStartCommand.mock.calls[1][1]).toMatchObject({ provider: "github", replaceGithubUser: true });
        await waitFor(() => expect(openLink).toHaveBeenCalledWith(GITHUB_URL));
    });

    it("keeps the connected user, and starts nothing, when the person says so", async () => {
        rpc.AccountOAuthStartCommand.mockResolvedValueOnce(mismatch);
        render(() => <OAuthConnectPanel provider="github" name={() => "work"} onConnected={() => {}} />);
        fireEvent.click(await screen.findByRole("button", { name: "Connect GitHub" }));
        fireEvent.click(await screen.findByRole("button", { name: "Keep @old" }));
        expect(await screen.findByRole("button", { name: "Connect GitHub" })).toBeTruthy();
        expect(rpc.AccountOAuthStartCommand).toHaveBeenCalledTimes(1);
    });
});

describe("Connect GitHub without an AgentMux Cloud sign-in", () => {
    const GITHUB_BUILTIN = { saved: false, hasSecret: false, builtinAvailable: true };
    const delivery = (state: string) => acceptMuxbusDelivery({ state, since_ms: 1 });

    it("says to sign in to AgentMux Cloud, in place of Connect", async () => {
        rpc.AccountOAuthClientGetCommand.mockResolvedValue(GITHUB_BUILTIN);
        delivery("signed_out");
        render(() => <OAuthConnectPanel provider="github" onConnected={() => {}} />);
        expect(await screen.findByText("Sign in to AgentMux Cloud to connect GitHub")).toBeTruthy();
        expect(screen.queryByRole("button", { name: "Connect GitHub" })).toBeNull();
        expect(rpc.AccountOAuthStartCommand).not.toHaveBeenCalled();
    });

    it("offers Connect once the Cloud is signed in", async () => {
        rpc.AccountOAuthClientGetCommand.mockResolvedValue(GITHUB_BUILTIN);
        delivery("connected");
        render(() => <OAuthConnectPanel provider="github" onConnected={() => {}} />);
        expect(await screen.findByRole("button", { name: "Connect GitHub" })).toBeTruthy();
        expect(screen.queryByText("Sign in to AgentMux Cloud to connect GitHub")).toBeNull();
    });

    it("turns srv's needs-cloud-sign-in answer into the same prompt, not an error", async () => {
        rpc.AccountOAuthClientGetCommand.mockResolvedValue(GITHUB_BUILTIN);
        rpc.AccountOAuthStartCommand.mockResolvedValue({ error: "needs-cloud-sign-in" });
        delivery("connected");
        render(() => <OAuthConnectPanel provider="github" name={() => "work"} onConnected={() => {}} />);
        fireEvent.click(await screen.findByRole("button", { name: "Connect GitHub" }));
        expect(await screen.findByText("Sign in to AgentMux Cloud to connect GitHub")).toBeTruthy();
        expect(screen.queryByText("needs-cloud-sign-in")).toBeNull();
    });

    it("drops the prompt that srv's answer raised once the person has signed in", async () => {
        rpc.AccountOAuthClientGetCommand.mockResolvedValue(GITHUB_BUILTIN);
        rpc.AccountOAuthStartCommand.mockResolvedValue({ error: "needs-cloud-sign-in" });
        delivery("connected");
        render(() => <OAuthConnectPanel provider="github" name={() => "work"} onConnected={() => {}} />);
        fireEvent.click(await screen.findByRole("button", { name: "Connect GitHub" }));
        expect(await screen.findByText("Sign in to AgentMux Cloud to connect GitHub")).toBeTruthy();
        // The sign-in lapsed, then the person signed in again from the prompt.
        acceptMuxbusDelivery({ state: "signed_out", since_ms: 2 });
        acceptMuxbusDelivery({ state: "connected", since_ms: 3 });
        await waitFor(() => expect(screen.queryByText("Sign in to AgentMux Cloud to connect GitHub")).toBeNull());
        expect(screen.getByRole("button", { name: "Connect GitHub" })).toBeTruthy();
    });

    it("a Reconnect while signed out shows the prompt once and starts nothing, then starts when signed in", async () => {
        rpc.AccountOAuthClientGetCommand.mockResolvedValue(GITHUB_BUILTIN);
        rpc.AccountOAuthStartCommand.mockResolvedValue({ error: "needs-cloud-sign-in" });
        delivery("signed_out");
        render(() => <OAuthConnectPanel provider="github" accountId="acc-1" autoStart onConnected={() => {}} />);
        expect(await screen.findByText("Sign in to AgentMux Cloud to connect GitHub")).toBeTruthy();
        expect(screen.getAllByText("Sign in to AgentMux Cloud to connect GitHub")).toHaveLength(1);
        expect(rpc.AccountOAuthStartCommand).not.toHaveBeenCalled();
        acceptMuxbusDelivery({ state: "connected", since_ms: 5 });
        await waitFor(() => expect(rpc.AccountOAuthStartCommand).toHaveBeenCalledTimes(1));
    });

    it("shows srv's answer once when the state was not known yet and the gate shows the prompt", async () => {
        rpc.AccountOAuthClientGetCommand.mockResolvedValue(GITHUB_BUILTIN);
        rpc.AccountOAuthStartCommand.mockResolvedValue({ error: "needs-cloud-sign-in" });
        delivery("connected");
        render(() => <OAuthConnectPanel provider="github" name={() => "work"} onConnected={() => {}} />);
        fireEvent.click(await screen.findByRole("button", { name: "Connect GitHub" }));
        expect(await screen.findByText("Sign in to AgentMux Cloud to connect GitHub")).toBeTruthy();
        acceptMuxbusDelivery({ state: "signed_out", since_ms: 6 });
        await waitFor(() => expect(screen.getAllByText("Sign in to AgentMux Cloud to connect GitHub")).toHaveLength(1));
    });

    it("leaves the person's own saved app alone", async () => {
        rpc.AccountOAuthClientGetCommand.mockResolvedValue({ saved: true, clientId: "Iv1.abc", hasSecret: false, mode: "byo", builtinAvailable: true });
        delivery("signed_out");
        render(() => <OAuthConnectPanel provider="github" onConnected={() => {}} />);
        expect(await screen.findByRole("button", { name: "Connect GitHub" })).toBeTruthy();
    });
});
