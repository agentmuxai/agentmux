// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for `resolveEffectiveLaunchProvider` — the actual fix for the
 * PR #2592 review finding that fixing only the backend layer-3
 * credential gate wasn't sufficient: `launchAgentDefinition` still
 * resolved which CLI to launch from the driftable `agent.provider`
 * column, independent of what the gate validates against the agent's
 * bound bundle. Extracted into its own function specifically so this
 * logic is testable in isolation — `launchAgentDefinition` itself has
 * no existing direct-invocation test anywhere in this codebase (every
 * caller mocks it away).
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const getMemory = vi.fn();
const loggerWarn = vi.fn();
const resolvePrereqs = vi.fn();
/** srv's `resolve.prereqs` answer for node/npm. */
const prereqs = (node: boolean, npm: boolean) => ({
    results: [
        { tool: "node", found: node, path: node ? "/usr/bin/node" : null },
        { tool: "npm", found: npm, path: npm ? "/usr/bin/npm" : null },
    ],
});

const resolveCli = vi.fn();
// A plain function, not a vi.fn, for the failure case: vitest reports what a
// vi.fn implementation throws as a test error even when the caller catches it.
let resolveCliImpl: ((...args: unknown[]) => unknown) | null = null;

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        GetBundleCommand: (...args: unknown[]) => getMemory(...args),
        ResolveCliCommand: (...args: unknown[]) => (resolveCliImpl ?? resolveCli)(...args),
        ResolvePrereqsCommand: (...args: unknown[]) => resolvePrereqs(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/util/logger", () => ({
    Logger: { warn: (...args: unknown[]) => loggerWarn(...args) },
}));
vi.mock("@/app/store/global", () => ({
    getApi: () => ({}),
}));

import * as launchEnv from "./agent-launch-env";
import {
    checkNodejsForProvider,
    commitLaunch,
    resolveCliBin,
    resolveEffectiveLaunchProvider,
    resolveInitialRuntimeConfig,
} from "./agent-launch-env";
import { DEFAULT_RUNTIME_CONFIG } from "./types";
import type { ProviderDefinition, ProviderModel } from "./providers/types";
import type { AgentDefinition } from "@/app/store/rpc-api";

function agentWith(provider: string, memory_id: string): AgentDefinition {
    return { id: "a1", provider, memory_id } as AgentDefinition;
}

function models(...specs: Array<{ value: string; default?: boolean }>): ProviderModel[] {
    return specs.map((s) => ({ value: s.value, label: s.value, default: s.default }));
}

describe("resolveEffectiveLaunchProvider", () => {
    beforeEach(() => {
        getMemory.mockReset();
        loggerWarn.mockReset();
    });

    afterEach(() => {
        vi.clearAllMocks();
    });

    it("returns agent.provider directly when the agent is unbound", async () => {
        const agent = agentWith("claude", "");
        const result = await resolveEffectiveLaunchProvider(agent);
        expect(result).toBe("claude");
        expect(getMemory).not.toHaveBeenCalled();
    });

    // The core regression case: a drifted agent.provider must not win
    // over the bound bundle's own copy — this is the exact scenario the
    // PR #2592 review flagged (gate validates "claude" from the bundle,
    // frontend used to launch "codex" from the drifted column).
    it("prefers the bound bundle's provider over a drifted agent.provider", async () => {
        getMemory.mockResolvedValue({ id: "mem1", provider: "claude" });
        const agent = agentWith("codex", "mem1");
        const result = await resolveEffectiveLaunchProvider(agent);
        expect(result).toBe("claude");
        expect(getMemory).toHaveBeenCalledWith({}, { id: "mem1" });
    });

    it("falls back to agent.provider when the bundle fetch fails", async () => {
        getMemory.mockRejectedValue(new Error("not found"));
        const agent = agentWith("claude", "mem-deleted");
        const result = await resolveEffectiveLaunchProvider(agent);
        expect(result).toBe("claude");
        expect(loggerWarn).toHaveBeenCalled();
    });

    it("falls back to agent.provider when the bundle's provider is empty", async () => {
        getMemory.mockResolvedValue({ id: "mem1", provider: "" });
        const agent = agentWith("claude", "mem1");
        const result = await resolveEffectiveLaunchProvider(agent);
        expect(result).toBe("claude");
    });
});

describe("resolveInitialRuntimeConfig", () => {
    // Fixes the latent bug this function exists to close: launchAgentDefinition
    // never set "agent:runtime" meta at all on a fresh launch, so
    // getRuntimeConfig's fallback (DEFAULT_RUNTIME_CONFIG, hardcoded to
    // Claude's "sonnet") silently applied to every launch regardless of
    // harness.
    it("uses an explicit override model when given, even if the provider has its own default", () => {
        const result = resolveInitialRuntimeConfig("gpt-5.5", models({ value: "gpt-5-mini", default: true }));
        expect(result.model).toBe("gpt-5.5");
    });

    it("falls back to the provider's own default model when no override is given", () => {
        const result = resolveInitialRuntimeConfig(
            undefined,
            models({ value: "gpt-5-mini" }, { value: "gpt-5.5", default: true }),
        );
        expect(result.model).toBe("gpt-5.5");
    });

    it("falls back to DEFAULT_RUNTIME_CONFIG.model when the provider declares no models at all", () => {
        const result = resolveInitialRuntimeConfig(undefined, undefined);
        expect(result.model).toBe(DEFAULT_RUNTIME_CONFIG.model);
    });

    it("falls back to DEFAULT_RUNTIME_CONFIG.model when the provider's model list has no default entry", () => {
        const result = resolveInitialRuntimeConfig(undefined, models({ value: "gpt-5-mini" }, { value: "gpt-5.5" }));
        expect(result.model).toBe(DEFAULT_RUNTIME_CONFIG.model);
    });

    it("carries permissionMode and effort from DEFAULT_RUNTIME_CONFIG unchanged", () => {
        const result = resolveInitialRuntimeConfig("gpt-5.5", undefined);
        expect(result.permissionMode).toBe(DEFAULT_RUNTIME_CONFIG.permissionMode);
        expect(result.effort).toBe(DEFAULT_RUNTIME_CONFIG.effort);
    });
});

// History (PR #2947): this check once probed the CEF host's own PATH, which
// lacks srv's login-shell enrichment (Homebrew/nvm on macOS), so Claude was
// exempted by id to avoid a false "Node.js is not installed". It now asks srv
// (`resolve.prereqs`), where npm actually runs, so the exemption is gone and
// every npm-installed provider — Claude included — gets the same check.
describe("checkNodejsForProvider", () => {
    beforeEach(() => {
        resolvePrereqs.mockReset();
    });

    afterEach(() => {
        vi.clearAllMocks();
    });

    it("asks srv (resolve.prereqs, the PATH npm actually runs with) for node and npm", async () => {
        resolvePrereqs.mockResolvedValue(prereqs(true, true));
        await checkNodejsForProvider({ id: "codex", npmPackage: "@openai/codex" });
        expect(resolvePrereqs).toHaveBeenCalledWith(expect.anything(), { tools: ["node", "npm"] });
    });

    it("checks claude like every other npm-installed provider (no PATH-mismatch exemption any more)", async () => {
        resolvePrereqs.mockResolvedValue(prereqs(false, false));
        const result = await checkNodejsForProvider({ id: "claude", npmPackage: "@anthropic-ai/claude-code" });
        expect(result).toContain("Node.js is not installed");
    });

    it("skips the check entirely for a provider with no npmPackage (e.g. kimi, pip-based)", async () => {
        const result = await checkNodejsForProvider({ id: "kimi", npmPackage: "" });
        expect(result).toBeNull();
        expect(resolvePrereqs).not.toHaveBeenCalled();
    });

    it("returns null when Node.js and npm are both available", async () => {
        resolvePrereqs.mockResolvedValue(prereqs(true, true));
        const result = await checkNodejsForProvider({ id: "codex", npmPackage: "@openai/codex" });
        expect(result).toBeNull();
    });

    it("returns a friendly Node.js-missing message when Node.js itself is unavailable", async () => {
        resolvePrereqs.mockResolvedValue(prereqs(false, false));
        const result = await checkNodejsForProvider({ id: "codex", npmPackage: "@openai/codex" });
        expect(result).toContain("Node.js is not installed");
    });

    it("returns a friendly npm-missing message when Node.js is present but npm is not", async () => {
        resolvePrereqs.mockResolvedValue(prereqs(true, false));
        const result = await checkNodejsForProvider({ id: "codex", npmPackage: "@openai/codex" });
        expect(result).toContain("npm is not installed");
    });

    it("does not block launch when the check itself fails", async () => {
        resolvePrereqs.mockRejectedValue(new Error("RPC unavailable"));
        const result = await checkNodejsForProvider({ id: "codex", npmPackage: "@openai/codex" });
        expect(result).toBeNull();
    });
});

describe("commitLaunch (identity M4b-3)", () => {
    function harness(create: () => Promise<{ id: string }>) {
        const calls: string[] = [];
        const setMeta = vi.fn(async (m: Record<string, unknown>) => {
            calls.push(`setMeta:${String(m.agentInstanceId)}`);
        });
        const resync = vi.fn(async () => {
            calls.push("resync");
        });
        const createInstance = vi.fn(async () => {
            calls.push("create");
            return create();
        });
        return { calls, setMeta, resync, createInstance };
    }

    // The ordering the spec requires: the row is recorded and stamped in the
    // same SetMeta as the block meta before any resync — the resync is where
    // a continuation eager-resumes, and it must already be bound.
    it("records the row, then stamps it in the one SetMeta, then resyncs", async () => {
        const h = harness(async () => ({ id: "row-1" }));
        const result = await commitLaunch({
            isTemplate: true,
            meta: { agentId: "tpl" },
            createInstance: h.createInstance,
            setMeta: h.setMeta,
            resync: h.resync,
            warn: () => {},
        });
        expect(h.calls).toEqual(["create", "setMeta:row-1", "resync"]);
        expect(h.setMeta).toHaveBeenCalledWith({ agentId: "tpl", agentInstanceId: "row-1" });
        expect(result).toEqual({ ok: true, instanceId: "row-1" });
    });

    // A template-backed launch whose row cannot be recorded never resyncs
    // unbound: nothing is written, and the error surfaces in the picker.
    it("aborts a template-backed launch whose create fails, writing nothing", async () => {
        const h = harness(async () => {
            throw new Error("db locked");
        });
        const result = await commitLaunch({
            isTemplate: true,
            meta: { agentId: "tpl" },
            createInstance: h.createInstance,
            setMeta: h.setMeta,
            resync: h.resync,
            warn: () => {},
        });
        expect(h.calls).toEqual(["create"]);
        expect(result.ok).toBe(false);
        expect(result.ok === false && result.error).toContain("db locked");
    });

    // A user agent's block already names its row, so a failed create stays
    // best-effort — but the stamp is cleared (null), never left stale.
    it("continues a user-agent launch whose create fails, clearing the stamp", async () => {
        const h = harness(async () => {
            throw new Error("db locked");
        });
        const result = await commitLaunch({
            isTemplate: false,
            meta: { agentId: "row-user" },
            createInstance: h.createInstance,
            setMeta: h.setMeta,
            resync: h.resync,
            warn: () => {},
        });
        expect(h.calls).toEqual(["create", "setMeta:null", "resync"]);
        expect(result).toEqual({ ok: true, instanceId: null });
    });
});

describe("resolveCliBin", () => {
    // Agent3 on 0.57.0 (2026-09-24): the frontend built the CLI path itself as
    // `<global ~/.agentmux>/instances/v<ver>/cli/<p>/node_modules/.bin/<cli>`,
    // but since v0.56.7 the backend installs CLIs under the channel's data dir,
    // and the path had no `.cmd` on Windows. The backend spawns whatever the
    // pane's `cmd` meta says, so every spawn failed "path not found".
    const provider = {
        id: "claude",
        cliCommand: "claude",
        npmPackage: "@anthropic-ai/claude-code",
        pinnedVersion: "latest",
        windowsInstallCommand: "irm https://example/install.ps1 | iex",
        unixInstallCommand: "curl -fsSL https://example/install.sh | bash",
    } as unknown as ProviderDefinition;
    const channelCli = String.raw`C:\Users\u\.agentmux\channels\local-main-x\versions\0.57.0\data\instances\v0.57.0\cli\claude/node_modules/.bin/claude.cmd`;

    beforeEach(() => {
        resolveCli.mockReset();
        resolveCliImpl = null;
    });

    it("returns the path the backend resolved, not one built from the host's home dir", async () => {
        resolveCli.mockResolvedValue({ cli_path: channelCli, version: "2.1.280", source: "local_install" });
        await expect(resolveCliBin(provider, "block-1")).resolves.toBe(channelCli);
    });

    it("asks ResolveCli with the provider's install fields and the pane's block id", async () => {
        resolveCli.mockResolvedValue({ cli_path: channelCli, version: "x", source: "local_install" });
        await resolveCliBin(provider, "block-1");
        expect(resolveCli).toHaveBeenCalledTimes(1);
        const [, data, opts] = resolveCli.mock.calls[0];
        expect(data).toEqual({
            provider_id: "claude",
            cli_command: "claude",
            npm_package: "@anthropic-ai/claude-code",
            pinned_version: "latest",
            windows_install_command: provider.windowsInstallCommand,
            unix_install_command: provider.unixInstallCommand,
            block_id: "block-1",
        });
        // A first launch may npm-install; same budget as launch-flow.ts.
        expect(opts).toEqual({ timeout: 300000 });
    });

    it("throws instead of returning an empty path", async () => {
        resolveCli.mockResolvedValue({ cli_path: "", version: "", source: "" });
        await expect(resolveCliBin(provider, "block-1")).rejects.toThrow(/no CLI path/);
    });

    it("propagates a backend failure (e.g. CLI missing and not installable)", async () => {
        resolveCliImpl = async () => {
            throw new Error('{"code":"AMX-CLI-001"}');
        };
        let caught: unknown = null;
        try {
            await resolveCliBin(provider, "block-1");
        } catch (e) {
            caught = e;
        }
        expect(String(caught)).toContain("AMX-CLI-001");
    });

    it("no longer exports the host-home path builder", () => {
        expect((launchEnv as Record<string, unknown>).resolveCliDir).toBeUndefined();
    });
});
