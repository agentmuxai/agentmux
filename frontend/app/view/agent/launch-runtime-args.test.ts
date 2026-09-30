// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A launched pane's `cmd:args` must already carry its model / effort /
 * permission flags.
 *
 * Regression: `launchAgentDefinition` stored `cmd:args` WITHOUT them and relied
 * on the per-send rebuild (`useAgentCommands`) to add them. A fresh pane spawns
 * lazily on its first message, so the rebuild landed first and hid the gap. A
 * continuation spawns at launch (eager resume), so it ran on the CLI's own
 * defaults (Opus 5.5, medium) while the strip showed the stored selection
 * (docs/retro/RETRO_RESUMED_AGENT_SPAWNS_WITHOUT_RUNTIME_FLAGS_2026_09_30.md).
 */

import { describe, expect, it } from "vitest";
import { buildRuntimeArgs } from "./buildRuntimeArgs";
import { buildLaunchCmdArgs } from "./launch-runtime-args";
import { selectLaunchArgs, withProviderFlags } from "./launch-args";
import { PROVIDERS } from "./providers/catalog";
import type { AgentRuntimeConfig } from "./types";

const claude = PROVIDERS.claude;
const runtime = (over: Partial<AgentRuntimeConfig> = {}): AgentRuntimeConfig => ({
    permissionMode: "bypass",
    model: "sonnet",
    effort: "high",
    ...over,
});
const valueOf = (args: string[], flag: string) => args[args.indexOf(flag) + 1];

describe("buildLaunchCmdArgs", () => {
    it("a persistent launch carries the selected model and effort", () => {
        const args = buildLaunchCmdArgs({ provider: claude, agentMode: "host", runtime: runtime(), providerFlags: "" });
        expect(valueOf(args, "--model")).toBe("sonnet");
        expect(valueOf(args, "--effort")).toBe("high");
        expect(args).toContain("--permission-prompt-tool");
    });

    it("follows the selection, not a fixed default", () => {
        const args = buildLaunchCmdArgs({
            provider: claude,
            agentMode: "host",
            runtime: runtime({ model: "opus", effort: "xhigh" }),
            providerFlags: "",
        });
        expect(valueOf(args, "--model")).toBe("opus");
        expect(valueOf(args, "--effort")).toBe("xhigh");
    });

    it("is exactly what the per-send rebuild produces, so launch and first send cannot disagree", () => {
        const cfg = runtime({ model: "opus", effort: "low" });
        const perSend = withProviderFlags(
            buildRuntimeArgs(selectLaunchArgs(claude, "host"), cfg, claude.id),
            "--add-dir /x",
        );
        const atLaunch = buildLaunchCmdArgs({ provider: claude, agentMode: "host", runtime: cfg, providerFlags: "--add-dir /x" });
        expect(atLaunch).toEqual(perSend);
    });

    it("keeps the agent's provider_flags, after the runtime flags", () => {
        const args = buildLaunchCmdArgs({ provider: claude, agentMode: "host", runtime: runtime(), providerFlags: "--add-dir /x" });
        expect(args.slice(-2)).toEqual(["--add-dir", "/x"]);
    });

    it("appends the one-shot --fork-session last, once", () => {
        const args = buildLaunchCmdArgs({
            provider: claude,
            agentMode: "host",
            runtime: runtime(),
            providerFlags: "--add-dir /x",
            appendForkFlag: true,
        });
        expect(args[args.length - 1]).toBe("--fork-session");
        expect(args.filter((a) => a === "--fork-session")).toHaveLength(1);
        expect(args).toContain("--model");
    });

    it("omits --fork-session unless asked", () => {
        const args = buildLaunchCmdArgs({ provider: claude, agentMode: "host", runtime: runtime(), providerFlags: "" });
        expect(args).not.toContain("--fork-session");
    });

    it("a container launch gets the one-shot args (no --input-format) plus the runtime flags", () => {
        const args = buildLaunchCmdArgs({ provider: claude, agentMode: "container", runtime: runtime(), providerFlags: "" });
        expect(args).not.toContain("--input-format");
        expect(valueOf(args, "--model")).toBe("sonnet");
    });

    it("a provider without --model wiring still gets none", () => {
        const args = buildLaunchCmdArgs({ provider: PROVIDERS.gemini, agentMode: "host", runtime: runtime(), providerFlags: "" });
        expect(args).not.toContain("--model");
    });
});
