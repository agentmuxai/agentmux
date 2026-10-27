// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { JSX } from "solid-js";
import type { ModalCloseProps } from "@/app/store/modalmodel";
import { ClaudeLoginPanel } from "@/app/view/accounts/ClaudeLoginPanel";

/**
 * The composer's "Sign in with another account…": the in-app Claude sign-in with no agent to link, so
 * it mints a new account, then hands its id back for the agent to switch to
 * (SPEC_ACCOUNT_SWITCH_FROM_THE_COMPOSER_NO_BIND_2026_10_26.md §3.2). Opened with `openModal`.
 */
export function AnotherAccountLogin(props: ModalCloseProps & { onSignedIn: (accountId: string) => void }): JSX.Element {
    return <ClaudeLoginPanel onClose={props.close} onSignedIn={props.onSignedIn} scope="window" />;
}
