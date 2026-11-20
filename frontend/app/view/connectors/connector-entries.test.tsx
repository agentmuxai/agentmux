// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What Connectors' list shows for each connector: its state in words, its tone, its one control and its place
// (docs/specs/SPEC_CONNECTORS_ONE_LIST_2026_11_19.md §4.2, §4.3, §3 D6, §4.10).

import { afterEach, describe, expect, it } from "vitest";

import { SERVICES, type GoogleService } from "@/app/view/accounts/google-services";
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

/** A Google account that has these services on: the scopes its token was granted. */
function google(id: string, email: string, services: GoogleService[], extra: Record<string, unknown> = {}): Account {
    const granted_scopes = SERVICES.filter((x) => services.includes(x.service)).flatMap((x) => x.readScopes);
    return { ...account(id, "google", email, extra), context: { email, granted_scopes } } as unknown as Account;
}

const CLOUD_OK: CloudSession = { connected: true, valid: true, email: "cloud@example.com" };
const byKey = (entries: ReturnType<typeof connectorEntries>, key: string) => entries.find((e) => e.key === key)!;

afterEach(() => setOAuthNote("g1", null));

describe("connector order", () => {
    it("lists every connector in the catalog's order with the text-only agents last, when nothing is connected", () => {
        const entries = connectorEntries([], null);
        expect(entries.map((e) => e.key)).toEqual([
            "agentmux", "github", "gmail", "calendar", "drive", "aws", "openai", "anthropic", "slack", "custom", "xai", "kimi",
        ]);
        expect(entries.every((e) => !e.connected && e.state === null)).toBe(true);
    });

    it("puts the connected ones first, AgentMux Cloud, then the catalog's order, and leaves the rest where they were", () => {
        const entries = connectorEntries([account("a", "aws", "ops@example.com", { kind: "role" }), account("c", "claude", "me@example.com")], CLOUD_OK);
        expect(entries.map((e) => e.key).slice(0, 5)).toEqual(["agentmux", "aws", "anthropic", "github", "gmail"]);
        expect(entries.slice(0, 3).every((e) => e.connected)).toBe(true);
        expect(entries.slice(3).every((e) => !e.connected)).toBe(true);
    });

    it("is the same order however many accounts there are (the order is data, never the pane's width)", () => {
        const some = connectorEntries([google("g1", "me@example.com", ["gmail"])], null).map((e) => e.key);
        expect(some).toEqual(["gmail", "agentmux", "github", "calendar", "drive", "aws", "openai", "anthropic", "slack", "custom", "xai", "kimi"]);
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
        const g = byKey(connectorEntries([google("g1", "me@example.com", ["gmail"])], null), "gmail");
        expect(g.connected).toBe(true);
        expect(g.state).toBe("Connected · me@example.com");
        expect(g.tone).toBe("ok");
        // Its Connect stays, to add another account (§4.10).
        expect(g.control).toBe("connect");
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
        expect(github.control).toBe("connect");
    });

    it("says 2 accounts alone when both are fine, and the singular for one that needs sign-in", () => {
        const fine = [account("a1", "github", "a@example.com", { kind: "pat" }), account("a2", "github", "b@example.com", { kind: "pat" })];
        expect(byKey(connectorEntries(fine, null), "github").state).toBe("2 accounts");
        const mixed = [fine[0], account("a2", "github", "b@example.com", { kind: "pat", status: "expired" })];
        expect(byKey(connectorEntries(mixed, null), "github").state).toBe("2 accounts · 1 needs sign-in again");
    });

    it("replaces Connect with Reconnect when its one account needs a sign-in again that can be re-run", () => {
        const g = byKey(connectorEntries([google("g1", "me@example.com", ["gmail"], { status: "needs_reauth" })], null), "gmail");
        expect(g.state).toBe("Needs sign-in again · me@example.com");
        expect(g.tone).toBe("warn");
        expect(g.control).toBe("reconnect");
        expect(g.reconnect?.id).toBe("g1");
    });

    it("keeps Connect when the stored sign-in can't be re-run from here (an expired token)", () => {
        const a = byKey(connectorEntries([account("a1", "github", "octo@example.com", { kind: "pat", status: "expired" })], null), "github");
        expect(a.tone).toBe("warn");
        expect(a.control).toBe("connect");
        expect(a.reconnect).toBeUndefined();
    });

    it("says an organization blocked it", () => {
        setOAuthNote("g1", { blocked: "blocked by your organization: … (admin_policy_enforced)." });
        const g = byKey(connectorEntries([google("g1", "me@example.com", ["gmail"])], null), "gmail");
        expect(g.state).toBe("Blocked by your organization · me@example.com");
        expect(g.tone).toBe("error");
        expect(g.control).toBe("reconnect");
    });
});

describe("Connect stays (§4.10)", () => {
    it("is on every connector that can take another account, connected or not", () => {
        const connected = connectorEntries([account("a1", "github", "a@example.com", { kind: "pat" }), account("k1", "custom", "k", { kind: "api_key" })], null);
        for (const key of ["github", "custom", "aws", "slack"]) {
            expect(byKey(connected, key).canAdd).toBe(true);
            expect(byKey(connected, key).control).toBe("connect");
        }
        expect(byKey(connected, "github").connected).toBe(true);
        expect(byKey(connected, "aws").connected).toBe(false);
    });

    it("is not on the agents that sign in inside their own pane", () => {
        const entries = connectorEntries([], null);
        expect(byKey(entries, "xai").canAdd).toBe(false);
        expect(byKey(entries, "kimi").canAdd).toBe(false);
    });
});

describe("Gmail, Calendar and Drive", () => {
    it("are three cards in Google's place, each with Connect, when nothing is connected", () => {
        const entries = connectorEntries([], null);
        expect(entries.some((e) => e.key === "google")).toBe(false);
        for (const key of ["gmail", "calendar", "drive"]) {
            const e = byKey(entries, key);
            expect(e.connected).toBe(false);
            expect(e.control).toBe("connect");
            expect(e.tile.id).toBe("google");
        }
        expect(["gmail", "calendar", "drive"].map((k) => byKey(entries, k).name)).toEqual(["Gmail", "Calendar", "Drive"]);
    });

    it("are each one Google service, which is what their Connect asks for", () => {
        const entries = connectorEntries([], null);
        for (const info of SERVICES) expect(byKey(entries, info.service).service).toBe(info.service);
        expect(byKey(entries, "github").service).toBeUndefined();
    });

    it("hold the Google accounts that have the service on, so one account can show under several cards", () => {
        const both = google("g1", "me@example.com", ["gmail", "calendar"]);
        const other = google("g2", "work@example.com", ["drive"]);
        const entries = connectorEntries([both, other], null);
        expect(byKey(entries, "gmail").accounts.map((a) => a.id)).toEqual(["g1"]);
        expect(byKey(entries, "calendar").accounts.map((a) => a.id)).toEqual(["g1"]);
        expect(byKey(entries, "drive").accounts.map((a) => a.id)).toEqual(["g2"]);
        expect(["gmail", "calendar", "drive"].every((k) => byKey(entries, k).connected)).toBe(true);
    });

    it("count several accounts on one service, each able to be added to", () => {
        const entries = connectorEntries([google("g1", "a@example.com", ["gmail"]), google("g2", "b@example.com", ["gmail"])], null);
        const gmail = byKey(entries, "gmail");
        expect(gmail.state).toBe("2 accounts");
        expect(gmail.control).toBe("connect");
        expect(byKey(entries, "drive").connected).toBe(false);
    });

    it("list a Google account with no service on under Gmail alone, so it can still be opened, turned on or deleted", () => {
        const entries = connectorEntries([google("g1", "me@example.com", [])], null);
        expect(byKey(entries, "gmail").accounts.map((a) => a.id)).toEqual(["g1"]);
        expect(byKey(entries, "calendar").connected).toBe(false);
        expect(byKey(entries, "drive").connected).toBe(false);
    });
});

describe("AgentMux Cloud", () => {
    it("is connected, with its email, when its session works", () => {
        const c = byKey(connectorEntries([], CLOUD_OK), "agentmux");
        expect(c.name).toBe("AgentMux Cloud");
        expect(c.state).toBe("Connected · cloud@example.com");
        expect(c.control).toBe("open");
    });

    it("is the one connector with a single sign-in: no Connect once connected", () => {
        const c = byKey(connectorEntries([], CLOUD_OK), "agentmux");
        expect(c.canAdd).toBe(false);
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
    const entries = connectorEntries([google("g1", "me@example.com", ["gmail"])], CLOUD_OK);
    it("matches every word over the name, the line, the state and the accounts", () => {
        expect(connectorMatches(byKey(entries, "gmail"), "gmail mail")).toBe(true);
        expect(connectorMatches(byKey(entries, "gmail"), "me@example")).toBe(true);
        expect(connectorMatches(byKey(entries, "gmail"), "slack")).toBe(false);
        expect(connectorMatches(byKey(entries, "calendar"), "me@example")).toBe(false);
        expect(connectorMatches(byKey(entries, "agentmux"), "cloud@example", "cloud@example.com")).toBe(true);
        expect(connectorMatches(byKey(entries, "slack"), "")).toBe(true);
    });
});
