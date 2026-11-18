// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What an import's card says of the login it makes (docs/specs/SPEC_VAULT_AGENTS_ENTER_CREDENTIALS_WHEREVER_ASKED_2026_11_16.md
// §6): where its user name is read from, and the share the agent asked for, as a plain line with a tick the person can
// remove. Unticked, the login is saved and shared with nobody.

import { Show, type JSX } from "solid-js";

import { t } from "@/app/i18n/i18n";
import { Rich } from "@/app/i18n/rich";
import type { UseImport } from "@/types/rpc/UseImport";

/** The login's user name key and its share line. `names` are the clipped text the card shows for the key, the site and the agent. */
export function ImportLogin(props: { import: UseImport; names: { userKey: string; site: string; agent: string }; share: boolean; onShare: (on: boolean) => void; disabled: boolean }): JSX.Element {
    return (
        <>
            <Show when={props.import.user_key}>
                <p>
                    <Rich k="vault.importCard.userKey" params={{ userKey: props.names.userKey }} />
                </p>
            </Show>
            <Show when={props.import.share}>
                <label class="vault-check">
                    <input type="checkbox" checked={props.share} onChange={(e) => props.onShare(e.currentTarget.checked)} disabled={props.disabled} />
                    <Rich k="vault.importCard.share" params={{ agent: props.names.agent, site: props.names.site }} />
                </label>
            </Show>
        </>
    );
}
