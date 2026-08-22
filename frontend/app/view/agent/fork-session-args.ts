// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Fork-session argument resolution — extracted out of
// agent-model.ts::launchAgentDefinition (SPEC_AGENT_QUICK_FORK_NEW_TAB_2026_08_21.md
// Phase 1) so this contract has its own unit tests instead of living
// inline in a large function. Two real bugs were found here, in the same
// spot, one review round apart (PR #2725, reagent + Codex) — a strong
// signal this logic deserved direct coverage, not just indirect coverage
// via a mocked integration test.

/**
 * Resolve fork-session behavior for a launch: whether to append the
 * `--fork-session` CLI flag, and whether to actually seed a session id to
 * resume from at all.
 *
 * `--fork-session` is Claude Code CLI syntax, validated only for Claude
 * (`SPEC_AGENT_PANE_FORKS_AND_AUX_PINS_2026_06_15.md` §6.4's empirical
 * gate) — every other provider must fall back to a true fresh start, not
 * a plain resume of `continueSessionId`. Two failure modes fixed here:
 *
 * - A fork requested (`forkSession: true`) for a non-Claude provider used
 *   to still seed `continueSessionId`, silently resuming the exact same
 *   live session the fork's source was still driving — not a fresh start
 *   (Codex's review of PR #2725). Fixed: when a fork is requested but the
 *   provider doesn't support it, `continueSessionId` is dropped entirely.
 * - A fork requested with no actual session to fork from (empty
 *   `continueSessionId`) used to still push a bare `--fork-session` flag
 *   with nothing to resume (reagent's review of PR #2725). Fixed: the
 *   flag is only appended when there's a real session id to pair it with.
 */
export function resolveForkSessionArgs(
    overrides: { continueSessionId?: string; forkSession?: boolean } | undefined,
    providerId: string
): { continueSessionId: string; appendForkFlag: boolean } {
    const continueSessionId = overrides?.continueSessionId?.trim() ?? "";
    const forkSessionSupported = providerId === "claude";
    if (overrides?.forkSession && !forkSessionSupported) {
        // Fork requested, but this provider can't fork-session — a plain
        // resume of the parent's live session is worse than a fresh
        // start, so drop the session id entirely rather than fall back
        // to it.
        return { continueSessionId: "", appendForkFlag: false };
    }
    return {
        continueSessionId,
        appendForkFlag: !!overrides?.forkSession && forkSessionSupported && continueSessionId !== "",
    };
}
