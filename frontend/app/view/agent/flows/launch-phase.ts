// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * LaunchPhase — what the mount-time launch flow (launch-flow.ts,
 * useAgentControllerStatus.ts's relogin) is actually doing right now,
 * surfaced to AgentFooter's working row instead of a generic "Working…"
 * for every phase. Mirrors the discriminated-union pattern of
 * auth/auth-state.ts's `AuthState.kind` (a different, pre-launch-modal flow).
 *
 * Any variant carrying `deadlineMs` represents a real timer/poll the user
 * is waiting on — formatPhaseLabel renders its remaining time so no wait
 * is ever silent (see the maintainer's rule: a timer without a visible
 * notification is a bug, not an implementation detail).
 */
/** Display-only estimate for how long tier 1's URL-capture wait takes,
 *  shown as a countdown while `waiting-for-login-link` is active. Must
 *  track cli_login.rs's actual URL_CAPTURE_TIMEOUT_SECS (currently 15s) —
 *  the frontend can't read that Rust constant directly, so this is a
 *  second source of truth kept in sync by hand. Shared here (not
 *  duplicated per call site) so there's exactly one place to update.
 *  reagent flagged a stale duplicate literal in useAgentControllerStatus.ts
 *  on PR #2300. */
export const LOGIN_LINK_CAPTURE_LABEL_MS = 15_000;

export type LaunchPhase =
    | { kind: "resolving-cli" }
    | { kind: "checking-auth" }
    /** Tier 1's PTY/pipe URL-capture attempt — only reachable for providers
     *  where `headlessLoginUrlUnsupported` is NOT set (Codex/Gemini/OpenClaw).
     *  See catalog.ts and cli_login.rs's URL_CAPTURE_TIMEOUT_SECS. */
    | { kind: "waiting-for-login-link"; deadlineMs: number }
    | { kind: "opening-login-terminal" }
    /** Covers both tier 1's "opened" completion poll and tier 3's terminal
     *  completion poll — both are a 5-minute CheckCliAuthCommand loop. */
    | { kind: "waiting-for-login-completion"; deadlineMs: number }
    | { kind: "verifying" }
    | { kind: "ready" }
    | { kind: "failed"; reason: string };

function fmtRemaining(ms: number): string {
    const s = Math.max(0, Math.ceil(ms / 1000));
    return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`;
}

/** Render `phase` as a footer-line label given the current time (`nowMs`,
 *  typically from the caller's own 1s tick so the countdown live-updates).
 *  Returns null for phases with nothing distinct to say (the generic
 *  "Working…" fallback already covers them fine) so callers can fall
 *  through to their existing default text. */
export function formatPhaseLabel(phase: LaunchPhase | null | undefined, nowMs: number): string | null {
    if (!phase) return null;
    switch (phase.kind) {
        case "resolving-cli":
            return "Resolving CLI";
        case "checking-auth":
            return "Checking authentication";
        case "waiting-for-login-link":
            return `Waiting for login link… up to ${fmtRemaining(phase.deadlineMs - nowMs)}`;
        case "opening-login-terminal":
            return "Opening login terminal";
        case "waiting-for-login-completion":
            return `Waiting for you to finish logging in… up to ${fmtRemaining(phase.deadlineMs - nowMs)}`;
        case "verifying":
            return "Verifying login";
        case "ready":
        case "failed":
            return null;
    }
}
