// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * failure-accessory — pure projection of an `AgentFailure` (the classified
 * cause of a non-zero agent exit, carried by the `agentfailure` wave event)
 * into the shared **PaneRow** accessory model. The per-error-class **recovery
 * actions** live here as one map; the row chrome is `<PaneRow>` — the same
 * primitive the session digest / ActivityDock / fork bar render through.
 *
 * "Derive from a source of truth" (no parallel store): the caller passes the
 * live failure + transient view flags + action handlers; this returns a
 * fully-resolved descriptor.
 *
 * Spec: docs/specs/SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16.md §3–§4.
 */

import type { PaneRowAction } from "../components/PaneRow";

/** Handlers the pane wires to the recovery actions. */
export interface FailureActions {
    /** Re-run the failed turn (re-send the last user message → resumes via `--resume`). */
    retry: () => void;
    /** Re-authenticate this agent's provider account (auth failures). */
    loginAgain: () => void;
    /** Seed from the user's existing valid global Claude login (no fresh OAuth). */
    useExistingLogin: () => void;
    /** Open a real terminal window so the browser OAuth can open, then poll for creds. */
    loginViaTerminal: () => void;
    /** Open Armory → Accounts. */
    openArmory: () => void;
    /** Start a fresh agent session — recovery for a context-window overflow,
     *  where resuming the same (full) session would only re-fail. */
    newSession: () => void;
    /** Toggle the expanded stderr-tail body. */
    toggleDetails: () => void;
    /** Clear the failure (dismiss the row). */
    dismiss: () => void;
}

/** Transient view state for the failure row (no backing store). */
export interface FailureViewState {
    /** stderr tail revealed. */
    expanded: boolean;
    /** Seconds left on the auto-retry countdown, or null when not armed. */
    autoRetryIn: number | null;
    /** True while a retry is in flight (disables the button). */
    retrying: boolean;
    /**
     * True when the provider supports seed-from-global recovery (Claude only).
     * Promotes "Use existing login" to primary and adds "Login via terminal"
     * as the fresh-OAuth path instead of the broken spawned-PTY path.
     */
    canSeed?: boolean;
}

/** Per-class sigil. */
const ICON: Record<AgentFailure["code"], string> = {
    auth: "🔐",
    rate_limited: "⏱",
    overloaded: "🌀",
    network: "🌐",
    usage_limit: "🚫",
    context_exceeded: "📏",
    max_turns: "🔁",
    killed: "⛔",
    spawn_failure: "🧩",
    no_output: "❔",
    unknown_non_zero: "⚠",
};

/** Classes whose retry is safe to fire **automatically** (transient throttling). */
export function isTransient(code: AgentFailure["code"]): boolean {
    return code === "rate_limited" || code === "overloaded" || code === "network";
}

/** The resolved PaneRow descriptor for a failure (props for `<PaneRow>`). */
export interface FailureRow {
    sigil: string;
    title: string;
    meta: string;
    accent: "error";
    actions: PaneRowAction[];
    expanded: boolean;
    /** Human-readable explanation (rendered in the expanded body). */
    detail: string;
    /** Raw provider stderr/result tail (rendered in the expanded body). */
    stderrTail?: string;
}

/** Project a failure + view state + handlers into a PaneRow descriptor. */
export function failureToRow(f: AgentFailure, view: FailureViewState, on: FailureActions): FailureRow {
    const meta: string[] = [f.code];
    if (f.signal != null) meta.push(`signal ${f.signal}`);
    else if (f.exitCode != null) meta.push(`exit ${f.exitCode}`);
    if (f.retryable) meta.push("retryable");

    const retryLabel = view.autoRetryIn != null ? `Retry now (${view.autoRetryIn}s)` : "Retry now";
    const retry: PaneRowAction = {
        glyph: "↻", label: retryLabel, title: "Re-run the last turn", primary: true,
        disabled: view.retrying, onClick: on.retry,
    };
    const openArmory: PaneRowAction = {
        // Same "vault" FontAwesome icon as the widget bar's Armory entry
        // (agentmux-srv/src/config/widgets.json) and the hamburger menu's
        // Armory item, instead of a generic gear emoji.
        icon: "vault", label: "Armory → Accounts", title: "Open Armory → Accounts", onClick: on.openArmory,
    };

    const actions: PaneRowAction[] = [];
    switch (f.code) {
        case "auth":
            if (view.canSeed) {
                // Claude provider: seed-from-global is the reliable path;
                // "Login via terminal" opens a real console so the browser
                // can launch (the spawned PTY path is headless and hangs).
                actions.push(
                    { glyph: "🌐", label: "Use existing login", title: "Copy your valid global Claude login into this agent (no re-OAuth)", primary: true, onClick: on.useExistingLogin },
                    { glyph: "🖥", label: "Login via terminal", title: "Open a terminal window where the browser login can complete", onClick: on.loginViaTerminal },
                    openArmory,
                );
            } else {
                actions.push(
                    { glyph: "🔑", label: "Login Again", title: "Re-authenticate this agent", primary: true, onClick: on.loginAgain },
                    { glyph: "🌐", label: "Use existing login", title: "Copy your existing valid global Claude login into this agent (no re-OAuth)", onClick: on.useExistingLogin },
                    openArmory,
                );
            }
            break;
        case "usage_limit":
            actions.push({ ...openArmory, label: "Armory (switch / upgrade)", primary: true });
            break;
        case "spawn_failure":
            // Clear `icon` — this isn't an Armory action, so it must NOT
            // inherit openArmory's vault icon (icon takes precedence over
            // glyph in PaneRow's render).
            actions.push({ ...openArmory, icon: undefined, glyph: "🧩", label: "Provider setup", title: "Fix the provider install", primary: true });
            break;
        case "rate_limited":
        case "overloaded":
        case "network":
            actions.push(retry);
            break;
        case "max_turns":
            actions.push({ ...retry, glyph: "▶", label: "Continue" });
            break;
        case "context_exceeded":
            // Resuming a context-exceeded session just re-fails (the window is
            // still full), so the only real recovery is a fresh session.
            actions.push({
                glyph: "🆕", label: "New session", title: "Start a fresh session — the current one's context window is full",
                primary: true, onClick: on.newSession,
            });
            break;
        default: // killed, no_output, unknown_non_zero
            actions.push({ ...retry, label: "Retry" });
            break;
    }

    // Offer the expander whenever there's expandable content. The body always
    // carries `detail` (the explanation), so classes with no stderr tail (auth,
    // usage_limit, context_exceeded) still need a way to reveal it — gating only
    // on `stderrTail` left their explanation unreachable.
    if (f.detail || f.stderrTail) {
        actions.push({
            glyph: view.expanded ? "▾" : "▸",
            label: view.expanded ? "Hide details" : "Details",
            title: "Show the explanation and any captured provider output",
            onClick: on.toggleDetails,
        });
    }
    actions.push({ glyph: "×", title: "Dismiss", danger: true, onClick: on.dismiss });

    return {
        sigil: ICON[f.code] ?? "⚠",
        title: f.title || "Agent run failed",
        meta: meta.join(" · "),
        accent: "error",
        actions,
        expanded: view.expanded,
        detail: f.detail,
        stderrTail: f.stderrTail,
    };
}
