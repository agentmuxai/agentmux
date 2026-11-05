// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The client fields a provider that shows them offers (a provider with no built-in client, and Google's
// dev and test fallback, SPEC_GOOGLE_PHASE_B_AGENTMUXS_OWN_CLIENT_2026_11_01.md D5), and that Google's own
// catalog entry shows none.

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const rpc = vi.hoisted(() => ({
    AccountOAuthClientGetCommand: vi.fn(),
    AccountOAuthClientClearCommand: vi.fn(),
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: rpc }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/global", () => ({ openLink: vi.fn() }));

import { oauthInfo, type OAuthServiceInfo } from "./oauth-catalog";
import { createOAuthClientState, OAuthClientSection } from "./OAuthClientSection";

const NOT_SAVED = { saved: false, hasSecret: false, builtinAvailable: false, offeredServices: [] };
const SAVED = { saved: true, clientId: "123-abc.apps.googleusercontent.com", hasSecret: true, mode: "byo", builtinAvailable: false, offeredServices: [] };

/** Google's entry with its client fields shown, as the fallback keeps them for a developer. */
const google = (): OAuthServiceInfo => ({ ...oauthInfo("google")!, hidesClient: false });

function mount(svc: OAuthServiceInfo) {
    const client = createOAuthClientState(svc);
    render(() => <OAuthClientSection client={client} />);
    return client;
}

beforeEach(() => {
    rpc.AccountOAuthClientGetCommand.mockResolvedValue(NOT_SAVED);
    rpc.AccountOAuthClientClearCommand.mockResolvedValue({ cleared: true });
});
afterEach(() => {
    cleanup();
    vi.clearAllMocks();
});

describe("OAuthClientSection", () => {
    it("shows no client fields for Google by default, saved client or not", async () => {
        expect(oauthInfo("google")?.hidesClient).toBe(true);
        const client = mount(oauthInfo("google")!);
        await waitFor(() => expect(client.state.loaded()).toBe(true));
        expect(client.state.showFields()).toBe(false);
        expect(screen.queryByPlaceholderText(/apps\.googleusercontent\.com/)).toBeNull();
        cleanup();

        rpc.AccountOAuthClientGetCommand.mockResolvedValue(SAVED);
        const saved = mount(oauthInfo("google")!);
        await waitFor(() => expect(saved.state.loaded()).toBe(true));
        expect(saved.state.showFields()).toBe(false);
        expect(screen.queryByText("Using your saved client")).toBeNull();
        // A saved client is still the one a connect runs with: no fields are asked of the person.
        expect(saved.state.requestFields()).toEqual({ fields: {} });
    });

    it("shows the fields once, with the console steps, and sends the client with its mode", async () => {
        const client = mount(google());
        await screen.findByText(/application type “Desktop app”/);
        const id = screen.getByPlaceholderText(/apps\.googleusercontent\.com/) as HTMLInputElement;
        const secret = screen.getByPlaceholderText(/stored in your OS keychain/) as HTMLInputElement;

        // Google's Desktop client needs its secret for the token exchange.
        fireEvent.input(id, { target: { value: " 123-abc.apps.googleusercontent.com " } });
        expect(client.state.requestFields()).toEqual({ error: "Client secret is required" });

        fireEvent.input(secret, { target: { value: "GOCSPX-shh" } });
        fireEvent.click(screen.getByRole("radio", { name: "Workspace internal" }));
        expect(client.state.requestFields()).toEqual({
            fields: { clientId: "123-abc.apps.googleusercontent.com", clientSecret: "GOCSPX-shh", clientMode: "workspace" },
        });
        client.state.clearSecret();
        expect(secret.value).toBe("");
    });

    it("offers a different client, and forgets the saved one", async () => {
        rpc.AccountOAuthClientGetCommand.mockResolvedValueOnce(SAVED).mockResolvedValueOnce(NOT_SAVED);
        mount(google());
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
});
