// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * InAppLoginPanel — the in-app OAuth login UI (spec §3.1's session, rendered):
 * URL + Open/Copy, a paste-code box wired to `setProviderAuth`, a live phase
 * line, and Cancel / "Use terminal instead" actions.
 *
 * Extracted from the launch modal's sign-in panel (spec §3.3 surface 1;
 * both since removed) so the agent pane (§3.3 surface 2) and the
 * Connectors surface (§3.3 surface 3) share one UI instead of
 * hand-rolled copies. Callers pass `authUrl` directly.
 *
 * See docs/specs/SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md.
 */

import { Show, type JSX } from "solid-js";
import "./InAppLoginPanel.scss";
import { AuthorizeStep, LoginActions, PasteStep } from "./InAppLoginPanelParts";
import {
    createLoginPaste,
    phaseText,
    type InAppLoginPanelProps,
    type InAppLoginPhase,
} from "./InAppLoginPanelState";

export type { InAppLoginPanelProps, InAppLoginPhase };

export const InAppLoginPanel = (p: InAppLoginPanelProps): JSX.Element => {
    const s = createLoginPaste(p);

    return (
        <div class="in-app-login-waiting">
            <div class="in-app-login-waiting-title">
                🔐 Sign in to {p.providerLabel}
            </div>
            <div class="in-app-login-hint">{phaseText(p.phase, p.providerLabel)}</div>
            {/* reagent P1 on PR #2410: gate on phase too, not just authUrl's
                presence. "Use terminal instead" kills the tier-1 login child
                (cancelCliLogin) and moves phase to "fallback"/"terminal-
                polling" WITHOUT clearing the now-dead authUrl — without this
                gate, the URL/paste box stayed visible for a session that no
                longer exists. Pasting a code at that point would call
                setProviderAuth, whose host handler falls through to the
                non-CLI config-file branch (cli_login_stdin already cleared)
                and silently writes the code as a plain auth_token while
                showing "Code accepted…", misleading the user while the real
                login runs in the separately-opened terminal instead. */}
            <Show
                when={
                    p.phase === "starting" || p.phase === "waiting-authorize"
                        ? p.authUrl
                        : undefined
                }
            >
                {(url) => (
                    <>
                        <AuthorizeStep url={url} />
                        <PasteStep s={s} />
                    </>
                )}
            </Show>
            <LoginActions p={p} />
        </div>
    );
};
