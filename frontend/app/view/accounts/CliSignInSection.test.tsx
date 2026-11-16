// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Reconnect for a CLI sign-in's account (SPEC_CONNECTORS_EVERY_WAY_TO_CONNECT_2026_11_14.md §4.3): the details say the
// provider no longer accepts the login and run the CLI's login again for the SAME account, so its directory, and the
// agents bound to it, are kept.

import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { Account } from "@/app/view/identity/identity-model";

const panel = vi.hoisted(() => ({ props: [] as Array<Record<string, unknown>> }));
vi.mock("./ProviderLoginPanel", () => ({
    ProviderLoginPanel: (props: Record<string, unknown>) => {
        panel.props.push(props);
        return <div data-testid="login-panel">signing in</div>;
    },
}));

import { CliSignInSection } from "./CliSignInSection";
import { requestReconnect, takeReconnectRequest } from "./oauth-account-state";

function account(over: Partial<Account> = {}): Account {
    return {
        id: "acct-1",
        name: "claude-oauth",
        provider: "claude" as Account["provider"],
        kind: "oauth",
        secret_ref: { backend: "oauth_config_dir", dir: "/tmp/acct-1" },
        context: {},
        assigned_agents: [],
        status: "needs_reauth",
        created_at: "0",
        updated_at: "0",
        ...over,
    };
}

afterEach(() => {
    cleanup();
    panel.props.length = 0;
    takeReconnectRequest("acct-1");
});

describe("CliSignInSection", () => {
    it("says the provider no longer accepts the login and offers Reconnect", () => {
        render(() => <CliSignInSection account={account()} />);
        expect(screen.getByText("Anthropic sign-in")).toBeTruthy();
        expect(screen.getByText(/Anthropic no longer accepts this sign-in/)).toBeTruthy();
        expect(screen.getByRole("button", { name: "Reconnect" })).toBeTruthy();
        expect(screen.queryByTestId("login-panel")).toBeNull();
    });

    it("runs the login again for the same account, so its directory and bindings are kept", () => {
        render(() => <CliSignInSection account={account({ provider: "codex" as Account["provider"], status: "expired" })} />);
        screen.getByRole("button", { name: "Reconnect" }).click();
        expect(screen.getByTestId("login-panel")).toBeTruthy();
        expect(panel.props).toHaveLength(1);
        expect(panel.props[0]).toMatchObject({ existingAccountId: "acct-1", brand: "ChatGPT", inline: true });
        expect((panel.props[0].provider as { id: string }).id).toBe("codex");
        // The linkTarget stays unset: the account is refreshed, no agent is relinked.
        expect(panel.props[0].linkTarget).toBeUndefined();
    });

    it("starts at once for a Reconnect clicked on the card", () => {
        requestReconnect("acct-1");
        render(() => <CliSignInSection account={account()} />);
        expect(screen.getByTestId("login-panel")).toBeTruthy();
        expect(panel.props[0]).toMatchObject({ existingAccountId: "acct-1" });
    });

    it("draws nothing for a login that works, a service sign-in or a CLI Connectors doesn't run", () => {
        const { container, unmount } = render(() => <CliSignInSection account={account({ status: "valid" })} />);
        expect(container.textContent).toBe("");
        unmount();
        const service = render(() => (
            <CliSignInSection account={account({ provider: "google", secret_ref: { backend: "keychain", service: "s", account: "a" } })} />
        ));
        expect(service.container.textContent).toBe("");
        service.unmount();
        const grok = render(() => <CliSignInSection account={account({ provider: "grok" as Account["provider"] })} />);
        expect(grok.container.textContent).toBe("");
    });

    it("closes the sign-in when the panel does", () => {
        render(() => <CliSignInSection account={account()} />);
        screen.getByRole("button", { name: "Reconnect" }).click();
        (panel.props[0].onClose as () => void)();
        expect(screen.queryByTestId("login-panel")).toBeNull();
        expect(screen.getByRole("button", { name: "Reconnect" })).toBeTruthy();
    });
});
