// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What Connectors' list shows for each connector: its state in words, its tone, its one control and its place
// (docs/specs/SPEC_CONNECTORS_ONE_LIST_2026_11_19.md §4.2, §4.3, §3 D6).

import { afterEach, describe, expect, it } from "vitest";

import { setOAuthNote } from "@/app/view/accounts/oauth-account-state";
import type { Account } from "@/app/view/identity/identity-model";
import { accountState, connectorEntries, connectorMatches, type CloudSession } from "./connector-entries";

function account(id: string, provider: string, email: string, extra: Record<string, unknown> = {}): Account {
    return {
        id,
        name: `${provider}-${id}`,
        provider,
        kind: "oauth",
        secret_ref: { backend: "keychain", service: "agentmux", account: id },
        context: { email },
        assigned_agents: [],
        status: "valid",
        created_at: "2026-10-01T00:00:00Z",
        updated_at: "2026-10-01T00:00:00Z",
        ...extra,
    } as unknown as Account;
}

const CLOUD_OK: CloudSession = { connected: true, valid: true, email: "cloud@example.com" };
const byKey = (entries: ReturnType<typeof connectorEntries>, key: string) => entries.find((e) => e.key === key)!;

afterEach(() => setOAuthNote("g1", null));

describe("connector order", () => {
    it("lists every connector in the catalog's order with the text-only agents last, when nothing is connected", () => {
        const entries = connectorEntries([], null);
        expect(entries.map((e) => e.key)).toEqual([
            "agentmux", "github", "google", "aws", "openai", "anthropic", "slack", "custom", "xai", "kimi",
        ]);
        expect(entries.every((e) => !e.connected && e.state === null)).toBe(true);
    });

    it("puts the connected ones first, AgentMux Cloud, then the catalog's order, and leaves the rest where they were", () => {
        const entries = connectorEntries([account("a", "aws", "ops@example.com", { kind: "role" }), account("c", "claude", "me@example.com")], CLOUD_OK);
        expect(entries.map((e) => e.key).slice(0, 5)).toEqual(["agentmux", "aws", "anthropic", "github", "google"]);
        expect(entries.slice(0, 3).every((e) => e.connected)).toBe(true);
        expect(entries.slice(3).every((e) => !e.connected)).toBe(true);
    });

    it("is the same order however many accounts there are (the order is data, never the pane's width)", () => {
        const some = connectorEntries([account("g1", "google", "me@example.com")], null).map((e) => e.key);
        expect(some).toEqual(["google", "agentmux", "github", "aws", "openai", "anthropic", "slack", "custom", "xai", "kimi"]);
    });
});

describe("a connector that is not connected", () => {
    it("says what it is for and offers Connect", () => {
        const github = byKey(connectorEntries([], null), "github");
        expect(github.line).toBe("Repositories, pull requests and Actions for your agents");
        expect(github.control).toBe("connect");
    });

    it("offers How to connect, not a button that does nothing, for an agent that signs in inside its own pane", () => {
        const entries = connectorEntries([], null);
        expect(byKey(entries, "xai").control).toBe("howto");
        expect(byKey(entries, "kimi").control).toBe("howto");
        expect(byKey(entries, "xai").textOnly).toBe(true);
    });
});

describe("a connected connector", () => {
    it("says who, in words, for one account", () => {
        const g = byKey(connectorEntries([account("g1", "google", "me@example.com")], null), "google");
        expect(g.connected).toBe(true);
        expect(g.state).toBe("Connected · me@example.com");
        expect(g.tone).toBe("ok");
        expect(g.control).toBe("open");
    });

    it("files a CLI sign-in under its brand (a claude account is Anthropic's)", () => {
        const a = byKey(connectorEntries([account("c", "claude", "me@example.com")], null), "anthropic");
        expect(a.connected).toBe(true);
        expect(a.accounts.map((x) => x.id)).toEqual(["c"]);
    });

    it("says how many accounts, and what is wrong with which, for several", () => {
        const accounts = [
            account("a1", "github", "octo@example.com", { kind: "pat", status: "expired" }),
            account("a2", "github", "other@example.com", { kind: "pat" }),
            account("a3", "github", "third@example.com", { kind: "pat", status: "invalid" }),
        ];
        const github = byKey(connectorEntries(accounts, null), "github");
        expect(github.state).toBe("3 accounts · 1 needs sign-in again · 1 couldn't connect");
        expect(github.tone).toBe("error");
        expect(github.control).toBe("open");
    });

    it("says 2 accounts alone when both are fine, and the singular for one that needs sign-in", () => {
        const fine = [account("a1", "github", "a@example.com", { kind: "pat" }), account("a2", "github", "b@example.com", { kind: "pat" })];
        expect(byKey(connectorEntries(fine, null), "github").state).toBe("2 accounts");
        const mixed = [fine[0], account("a2", "github", "b@example.com", { kind: "pat", status: "expired" })];
        expect(byKey(connectorEntries(mixed, null), "github").state).toBe("2 accounts · 1 needs sign-in again");
    });

    it("replaces the chevron with Reconnect when its one account needs a sign-in again that can be re-run", () => {
        const g = byKey(connectorEntries([account("g1", "google", "me@example.com", { status: "needs_reauth" })], null), "google");
        expect(g.state).toBe("Needs sign-in again · me@example.com");
        expect(g.tone).toBe("warn");
        expect(g.control).toBe("reconnect");
        expect(g.reconnect?.id).toBe("g1");
    });

    it("keeps the chevron when the stored sign-in can't be re-run from here (an expired token)", () => {
        const a = byKey(connectorEntries([account("a1", "github", "octo@example.com", { kind: "pat", status: "expired" })], null), "github");
        expect(a.tone).toBe("warn");
        expect(a.control).toBe("open");
        expect(a.reconnect).toBeUndefined();
    });

    it("says an organization blocked it", () => {
        setOAuthNote("g1", { blocked: "blocked by your organization: … (admin_policy_enforced)." });
        const g = byKey(connectorEntries([account("g1", "google", "me@example.com")], null), "google");
        expect(g.state).toBe("Blocked by your organization · me@example.com");
        expect(g.tone).toBe("error");
        expect(g.control).toBe("reconnect");
    });
});

describe("AgentMux Cloud", () => {
    it("is connected, with its email, when its session works", () => {
        const c = byKey(connectorEntries([], CLOUD_OK), "agentmux");
        expect(c.name).toBe("AgentMux Cloud");
        expect(c.state).toBe("Connected · cloud@example.com");
        expect(c.control).toBe("open");
    });

    it("offers Reconnect when the cloud wants a sign-in again", () => {
        const c = byKey(connectorEntries([], { connected: true, valid: false, needsReauth: true }), "agentmux");
        expect(c.state).toBe("Needs sign-in again");
        expect(c.tone).toBe("warn");
        expect(c.control).toBe("reconnect");
    });

    it("is not connected when signed out", () => {
        const c = byKey(connectorEntries([], { connected: false, valid: false }), "agentmux");
        expect(c.connected).toBe(false);
        expect(c.control).toBe("connect");
    });
});

describe("accountState", () => {
    it("maps every stored status to words and a tone", () => {
        const of = (status: string) => accountState({ id: "x", kind: "api_key", status } as Pick<Account, "id" | "kind" | "status">);
        expect(of("valid")).toEqual({ tone: "ok", word: "Connected" });
        expect(of("expired").tone).toBe("warn");
        expect(of("needs_reauth").word).toBe("Needs sign-in again");
        expect(of("invalid")).toEqual({ tone: "error", word: "Couldn't connect" });
        expect(of("checking").tone).toBe("busy");
        expect(of("unknown")).toEqual({ tone: "idle", word: "Connected, not checked yet" });
    });
});

describe("the filter", () => {
    const entries = connectorEntries([account("g1", "google", "me@example.com")], CLOUD_OK);
    it("matches every word over the name, the line, the state and the accounts", () => {
        expect(connectorMatches(byKey(entries, "google"), "gmail calendar")).toBe(true);
        expect(connectorMatches(byKey(entries, "google"), "me@example")).toBe(true);
        expect(connectorMatches(byKey(entries, "google"), "slack")).toBe(false);
        expect(connectorMatches(byKey(entries, "agentmux"), "cloud@example", "cloud@example.com")).toBe(true);
        expect(connectorMatches(byKey(entries, "slack"), "")).toBe(true);
    });
});
