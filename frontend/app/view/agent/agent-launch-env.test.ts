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

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        GetMemoryCommand: (...args: unknown[]) => getMemory(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/util/logger", () => ({
    Logger: { warn: (...args: unknown[]) => loggerWarn(...args) },
}));

import { resolveEffectiveLaunchProvider, resolveInitialRuntimeConfig } from "./agent-launch-env";
import { DEFAULT_RUNTIME_CONFIG } from "./types";
import type { ProviderModel } from "./providers/types";

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
