// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { buildStartupPayload, resolveAccounts, type ResolvedAccount, type StartupPayloadOpts } from "./buildStartupPayload";

function makeAgent(overrides: Partial<ForgeAgent> = {}): ForgeAgent {
    return {
        id: "test-agent",
        slug: "test-agent",
        name: "TestAgent",
        icon: "robot",
        provider: "claude",
        description: "A test agent for unit tests.",
        working_directory: "/home/user/project",
        shell: "",
        provider_flags: "",
        auto_start: 0,
        restart_on_crash: 0,
        idle_timeout_minutes: 0,
        created_at: 1700000000000,
        agent_type: "host",
        environment: "",
        agent_bus_id: "",
        is_seeded: 0,
        accounts: "",
        ...overrides,
    };
}

function makeOpts(overrides: Partial<StartupPayloadOpts> = {}): StartupPayloadOpts {
    return {
        agent: makeAgent(),
        providerDisplayName: "Claude Code",
        workDir: "/home/user/project",
        version: "0.33.200",
        accounts: [],
        peerAgents: [],
        startupContent: null,
        ...overrides,
    };
}

describe("buildStartupPayload", () => {
    it("includes identity section with agent info", () => {
        const result = buildStartupPayload(makeOpts());
        expect(result).toContain("# Session Context");
        expect(result).toContain("**Name:** TestAgent");
        expect(result).toContain("**Provider:** Claude Code");
        expect(result).toContain("**Working Directory:** /home/user/project");
        expect(result).toContain("**AgentMux Version:** 0.33.200");
    });

    it("includes agent description when present", () => {
        const result = buildStartupPayload(makeOpts());
        expect(result).toContain("## Description");
        expect(result).toContain("A test agent for unit tests.");
    });

    it("omits description section when empty", () => {
        const result = buildStartupPayload(makeOpts({
            agent: makeAgent({ description: "" }),
        }));
        expect(result).not.toContain("## Description");
    });

    it("includes assigned accounts", () => {
        const accounts: ResolvedAccount[] = [
            {
                provider: "github",
                name: "My GitHub PAT",
                kind: "pat",
                accessMethod: "env:GITHUB_TOKEN",
                context: { github_username: "testuser" },
            },
        ];
        const result = buildStartupPayload(makeOpts({ accounts }));
        expect(result).toContain("## Assigned Accounts");
        expect(result).toContain("### github — My GitHub PAT");
        expect(result).toContain("**Kind:** pat");
        expect(result).toContain("**Access:** env:GITHUB_TOKEN");
        expect(result).toContain("**Github Username:** testuser");
    });

    it("omits accounts section when none assigned", () => {
        const result = buildStartupPayload(makeOpts({ accounts: [] }));
        expect(result).not.toContain("## Assigned Accounts");
    });

    it("includes peer agents", () => {
        const peers = [
            makeAgent({ id: "other-1", name: "AlphaBot", provider: "claude", description: "Handles deploys" }),
            makeAgent({ id: "other-2", name: "BetaBot", provider: "gemini", description: "" }),
        ];
        const result = buildStartupPayload(makeOpts({ peerAgents: peers }));
        expect(result).toContain("## Peer Agents");
        expect(result).toContain("**AlphaBot** (claude) — Handles deploys");
        expect(result).toContain("**BetaBot** (gemini)");
    });

    it("excludes self from peer agents", () => {
        const self = makeAgent({ id: "test-agent", name: "TestAgent" });
        const result = buildStartupPayload(makeOpts({
            agent: self,
            peerAgents: [self, makeAgent({ id: "other", name: "Other" })],
        }));
        expect(result).not.toContain("**TestAgent** (claude)");
        expect(result).toContain("**Other** (claude)");
    });

    it("caps peer agents at 10", () => {
        const peers = Array.from({ length: 15 }, (_, i) =>
            makeAgent({ id: `peer-${i}`, name: `Peer${i}` })
        );
        const result = buildStartupPayload(makeOpts({ peerAgents: peers }));
        expect(result).toContain("**Peer0**");
        expect(result).toContain("**Peer9**");
        expect(result).not.toContain("**Peer10**");
        expect(result).toContain("...and 5 more");
    });

    it("includes startup instructions with template expansion", () => {
        const result = buildStartupPayload(makeOpts({
            startupContent: "Hello {{AGENT}}, you are running on {{PROVIDER}}.",
        }));
        expect(result).toContain("## Startup Instructions");
        expect(result).toContain("Hello TestAgent, you are running on Claude Code.");
    });

    it("returns null when startup content is __SKIP__", () => {
        const result = buildStartupPayload(makeOpts({
            startupContent: "__SKIP__",
        }));
        expect(result).toBeNull();
    });

    it("omits slug line when slug equals name", () => {
        const result = buildStartupPayload(makeOpts({
            agent: makeAgent({ name: "TestAgent", slug: "TestAgent" }),
        }));
        expect(result).not.toContain("**Slug:**");
    });

    it("includes slug line when different from name", () => {
        const result = buildStartupPayload(makeOpts({
            agent: makeAgent({ name: "Test Agent", slug: "test-agent" }),
        }));
        expect(result).toContain("**Slug:** test-agent");
    });
});

describe("resolveAccounts", () => {
    const mockAccounts = [
        {
            id: "acct-github-1",
            name: "Work GitHub",
            provider: "github",
            kind: "pat",
            secret_ref: { backend: "env", env_var: "GITHUB_TOKEN" },
            context: { github_username: "octocat", github_scopes: ["repo", "read:org"] },
        },
        {
            id: "acct-aws-1",
            name: "Prod AWS",
            provider: "aws",
            kind: "role",
            secret_ref: { backend: "secrets_manager", sm_path: "/prod/aws/key" },
            context: { aws_region: "us-east-1", aws_role_arn: "arn:aws:iam::role/deploy" },
        },
    ];

    it("resolves matching accounts", () => {
        const result = resolveAccounts(
            { github: "acct-github-1", aws: "acct-aws-1" },
            mockAccounts,
        );
        expect(result).toHaveLength(2);
        expect(result[0].provider).toBe("github");
        expect(result[0].name).toBe("Work GitHub");
        expect(result[0].accessMethod).toBe("env:GITHUB_TOKEN");
        expect(result[0].context.github_username).toBe("octocat");
    });

    it("skips null assignments", () => {
        const result = resolveAccounts(
            { github: "acct-github-1", aws: null },
            mockAccounts,
        );
        expect(result).toHaveLength(1);
        expect(result[0].provider).toBe("github");
    });

    it("skips unresolvable account IDs", () => {
        const result = resolveAccounts(
            { github: "acct-does-not-exist" },
            mockAccounts,
        );
        expect(result).toHaveLength(0);
    });

    it("describes secrets_manager access method", () => {
        const result = resolveAccounts(
            { aws: "acct-aws-1" },
            mockAccounts,
        );
        expect(result[0].accessMethod).toBe("secrets_manager:/prod/aws/key");
    });

    it("flattens array context values", () => {
        const result = resolveAccounts(
            { github: "acct-github-1" },
            mockAccounts,
        );
        expect(result[0].context.github_scopes).toBe("repo, read:org");
    });
});
