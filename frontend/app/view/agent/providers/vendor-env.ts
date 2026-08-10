// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Resolve the `[envVar, value]` pair to inject for an agent's model vendor
 * override, if any — redirects the harness at a non-default backend (e.g.
 * `ANTHROPIC_BASE_URL` for a `claude`-provider agent). Pure — no I/O — so
 * it's directly unit-testable without agent-model.ts's heavy launch-flow
 * dependencies (RpcApi, block stores, etc.).
 *
 * `null` when there's nothing to override (no `modelVendorBaseUrl`) OR the
 * provider doesn't declare `baseUrlEnvVar` — the latter should already be
 * impossible by the time an agent reaches launch (rejected at
 * `agent.define`), but this stays defensive rather than trusting that
 * write-time validation is the only thing that can ever set this field.
 * Mirrors agentmux-srv's `resolve_vendor_env_override`.
 */
export function resolveVendorEnvOverride(
    modelVendorBaseUrl: string | undefined,
    baseUrlEnvVar: string | undefined,
): [string, string] | null {
    if (!modelVendorBaseUrl || !baseUrlEnvVar) {
        return null;
    }
    return [baseUrlEnvVar, modelVendorBaseUrl];
}
