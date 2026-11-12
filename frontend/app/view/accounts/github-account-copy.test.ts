// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import type { Account } from "@/app/view/identity/identity-model";

import { modeLabel } from "./accounts-catalog";
import { githubActsAs, githubKindLabel, githubKindShort, isGithubToken } from "./github-account-copy";

type Probe = Pick<Account, "provider" | "kind" | "context">;
const acct = (kind: Account["kind"], context: Record<string, unknown> = {}, provider = "github"): Probe =>
    ({ provider, kind, context }) as Probe;

describe("the GitHub choices", () => {
    it("say what each one is, and other providers keep their own labels", () => {
        expect(modeLabel("oauth", "github").title).toBe("Use AgentMux's GitHub App");
        expect(modeLabel("oauth", "github").sub).toContain("muxagent");
        expect(modeLabel("key", "github").title).toBe("Add a personal access token");
        expect(modeLabel("key", "github").sub).toContain("Agents act as you");
        expect(modeLabel("oauth", "google").title).toBe("Connect with OAuth");
        expect(modeLabel("key", "openai").title).toBe("Add API key / token");
        expect(modeLabel("key").title).toBe("Add API key / token");
    });
});

describe("a GitHub account in words", () => {
    it("is the App when the broker made it, and a token for a token or an older API key", () => {
        expect(githubKindLabel(acct("oauth", { broker: true }))).toBe("AgentMux's GitHub App");
        expect(githubKindShort(acct("oauth", { broker: true }))).toBe("GitHub App");
        for (const kind of ["pat", "api_key"] as const) {
            expect(isGithubToken(acct(kind))).toBe(true);
            expect(githubKindLabel(acct(kind))).toBe("Personal access token");
            expect(githubKindShort(acct(kind))).toBe("Token");
        }
    });

    it("keeps the general label for a sign-in made with the person's own client, and for other providers", () => {
        expect(githubKindLabel(acct("oauth", { oauth_client: { mode: "byo" } }))).toBeNull();
        expect(githubKindShort(acct("oauth"))).toBeNull();
        expect(githubKindLabel(acct("api_key", {}, "openai"))).toBeNull();
        expect(isGithubToken(acct("api_key", {}, "openai"))).toBe(false);
    });

    it("says who agents act as", () => {
        expect(githubActsAs(acct("oauth", { broker: true }))).toContain("muxagent[bot]");
        expect(githubActsAs(acct("pat", { github_login: "octo" }))).toBe("Agents act on GitHub as @octo with this token.");
        expect(githubActsAs(acct("pat"))).toBe("Agents act on GitHub as you with this token.");
        expect(githubActsAs(acct("oauth", { oauth_client: { mode: "byo" } }))).toBeNull();
        expect(githubActsAs(acct("api_key", {}, "openai"))).toBeNull();
    });
});
