// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { JSX } from "solid-js";
import { ClaudeLoginPanel } from "@/app/view/accounts/ClaudeLoginPanel";
import type { PaneRequestProps } from "../pane-requests/pane-requests";

/**
 * The composer's "Sign in with another account…": the in-app Claude sign-in with no agent to link, so
 * it mints a new account, then hands its id back for the agent to switch to
 * (SPEC_ACCOUNT_SWITCH_FROM_THE_COMPOSER_NO_BIND_2026_10_26.md §3.2). Opened with `openPaneModal`, so it
 * covers the agent's pane only (SPEC_PANE_MODALS_NOT_WINDOW_MODALS_2026_11_02.md row 1).
 */
export function AnotherAccountLogin(props: PaneRequestProps & { onSignedIn: (accountId: string) => void }): JSX.Element {
    return <ClaudeLoginPanel onClose={props.close} onSignedIn={props.onSignedIn} scope="pane" />;
}
