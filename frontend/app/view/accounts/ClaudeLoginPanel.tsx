// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ClaudeLoginPanel — the in-app Claude/Anthropic login session, driven from
 * a context that has no agent block: Connectors → Accounts' "Connect"
 * (fresh account, no `linkTarget`) and a per-binding
 * "Connect / Re-login" action (`linkTarget` + `existingAccountId`, refresh
 * not mint). Both reuse the exact same session logic and UI
 * (`InAppLoginPanel`) as the agent pane's own login — see
 * docs/specs/SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md §3.3 surface 3.
 *
 * Deliberately simpler than the since-removed launch modal's
 * `AuthFlowController`: there
 * is no provider dropdown to switch away from mid-login (this panel is
 * Claude-only) and no launch sequence waiting on the result, so a plain
 * boolean in-flight guard + a few signals is enough — no action-token
 * staleness tracking needed.
 */

import type { JSX } from "solid-js";

import type { ClaudeLoginProps } from "./claude-login-model";
import { createLoginSession } from "./claude-login-session";
import { createLoginState } from "./claude-login-state";
import { LoginView } from "./claude-login-view";

/**
 * The panel is a state object (claude-login-state.ts), the session that acts on it (claude-login-session.ts,
 * with the outcome handling in claude-login-finish.ts), and the page that reads both. The login starts as
 * soon as the panel is made.
 */
export function ClaudeLoginPanel(props: ClaudeLoginProps): JSX.Element {
    const s = createLoginState(props);
    const session = createLoginSession(s);

    void session.start();

    return <LoginView s={s} session={session} />;
}

ClaudeLoginPanel.displayName = "ClaudeLoginPanel";
