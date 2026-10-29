// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether a launch writes its account through to the agent's link.
 *
 * A new agent, or a fork (a new definition), takes the account the launch was given. A continuation
 * of an earlier instance is given the account that instance last ran on, which is no longer the
 * agent's when the person moved it to another account since: writing it back undid their switch,
 * and the agent flipped between the two accounts (and their config dirs) on every reopen. A
 * continuation keeps the link the agent already has; it writes only when the agent has none, or
 * when its links couldn't be read (`existing` null), where leaving them is the safe side.
 */
export function shouldWriteThroughAccountLink(opts: {
    accountId: string;
    continuing: boolean;
    existing: readonly unknown[] | null;
}): boolean {
    if (!opts.accountId) return false;
    if (!opts.continuing) return true;
    return opts.existing != null && opts.existing.length === 0;
}
