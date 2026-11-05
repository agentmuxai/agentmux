// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Claude sign-in panel's state: the signals the view reads, the fields the login's callbacks write, and
 * the cleanup that kills an orphaned login.
 */

import { createSignal, onCleanup } from "solid-js";

import { loginBackend } from "@/app/view/agent/flows/login-backend";
import type { InAppLoginPhase } from "@/app/view/agent/components/InAppLoginPanel";
import type { ClaudeLoginProps } from "./claude-login-model";

export function createLoginState(props: ClaudeLoginProps) {
    const [phase, setPhase] = createSignal<InAppLoginPhase>("starting");
    const [authUrl, setAuthUrl] = createSignal<string | undefined>(undefined);
    const [error, setError] = createSignal<string | null>(null);
    const [done, setDone] = createSignal(false);
    // Signal (not a plain `let`) so `Modal`'s `closeOnBackdropClick` can
    // reactively track it — preserves the original hand-rolled overlay's
    // "don't dismiss on backdrop click mid-login" behavior.
    const [inFlight, setInFlight] = createSignal(false);
    const s = {
        props,
        phase,
        setPhase,
        authUrl,
        setAuthUrl,
        error,
        setError,
        done,
        setDone,
        inFlight,
        setInFlight,
        cancelled: false,
        terminalRequested: false,
        // Set by onAccountRegistered — run-provider-login.ts fires it ONLY once
        // the account row is actually persisted. A credential can be validly
        // seeded/pasted on disk while that persist call itself fails
        // (REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_2026_07_27.md) —
        // trusting the outcome string alone would show "Signed in" for a login
        // the resolver's spawn gate then blocks on the agent's next spawn, with
        // no account ever having existed. useAgentControllerStatus.ts's
        // relogin() makes the same check (#2414).
        registeredAccountId: undefined as string | undefined,
    };

    // Kill a genuinely orphaned login (in-flight up to 5 min) if the panel
    // unmounts before it resolves — otherwise the spawned CLI child and
    // this component's poll keep running detached, holding the host's
    // singleton cli_login_* state and blocking any later login attempt
    // from starting (#2414).
    //
    // Guarded on `inFlight`: this panel can also unmount AFTER reaching its
    // success/error screen, while still counted as "mounted" for a moment
    // (e.g. the user lingers on "✓ Signed in" before closing). cancelCliLogin()
    // hits the host's SINGLE global login slot — if a different window/surface
    // has since started its own login, an unconditional call here would kill
    // that unrelated, newer attempt instead of a no-op cleanup of our own
    // already-finished one.
    onCleanup(() => {
        s.cancelled = true;
        if (inFlight()) {
            loginBackend().cancel().catch(() => {});
        }
    });

    return s;
}

export type LoginState = ReturnType<typeof createLoginState>;
