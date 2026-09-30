// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The `cmd:args` a pane is launched with — base args, the selected runtime
 * flags (`--model`, `--effort`, the permission flags), the agent's own
 * `provider_flags`, and the one-shot `--fork-session`, in that order.
 *
 * ## Why this is one function
 *
 * `launchAgentDefinition` used to store the base args and `provider_flags` but
 * NOT the runtime flags, leaving `useAgentCommands`' per-send rebuild to add
 * them. That hid the gap for a fresh pane (it spawns lazily on its first
 * message, after the rebuild) and exposed it for a continuation, which spawns
 * at launch and so ran on the CLI's own defaults — Opus 5.5 at medium effort —
 * while the strip showed the stored selection
 * (docs/retro/RETRO_RESUMED_AGENT_SPAWNS_WITHOUT_RUNTIME_FLAGS_2026_09_30.md).
 *
 * The composition below is exactly the per-send rebuild's
 * (`withProviderFlags(buildRuntimeArgs(selectLaunchArgs(…)))`), so the args a
 * pane launches with and the args its first send rewrites them to cannot
 * disagree. `--fork-session` is the one addition: a one-shot launch intent that
 * the per-send rebuild deliberately never reapplies, or every turn would fork
 * the session again.
 */

import { buildRuntimeArgs } from "./buildRuntimeArgs";
import { selectLaunchArgs, withProviderFlags, type LaunchArgsProvider } from "./launch-args";
import type { AgentRuntimeConfig } from "./types";

export interface LaunchCmdArgsInput {
    provider: LaunchArgsProvider & { id: string };
    agentMode: string | undefined;
    runtime: AgentRuntimeConfig;
    /** The agent definition's raw `provider_flags` string. */
    providerFlags: unknown;
    /** Append `--fork-session` (see `resolveForkSessionArgs`). */
    appendForkFlag?: boolean;
}

export function buildLaunchCmdArgs(input: LaunchCmdArgsInput): string[] {
    const args = withProviderFlags(
        buildRuntimeArgs(selectLaunchArgs(input.provider, input.agentMode), input.runtime, input.provider.id),
        input.providerFlags,
    );
    return input.appendForkFlag ? [...args, "--fork-session"] : args;
}
