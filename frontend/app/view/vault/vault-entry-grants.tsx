// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// An entry's standing grants, under its label in the Vault's table: the sites
// agents may fill it on, and the agents that may run commands or sign in with it
// without asking. Each has its own button to take it back.

import { For, Show, type JSX } from "solid-js";

import { IconButton } from "@/app/element/ui";
import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";
import { COMMAND_GRANT, commandGrantLabel, isSiteGrant, SIGNIN_GRANT, siteOf } from "./vault-use-prompts";

/** One kind of grant: a line of names, each with its revoke button. */
function GrantList(props: { grants: string[]; label: string; lead: string; name: (g: string) => string; stop: (g: string) => string; revoke: (g: string) => void }): JSX.Element {
    return (
        <Show when={props.grants.length > 0}>
            <div class="vault-grants" aria-label={props.label}>
                <span class="warden-manager-dim">{props.lead}</span>
                <For each={props.grants}>
                    {(grant) => (
                        <span class="vault-grant" data-origin={grant}>
                            {props.name(grant)}
                            <IconButton icon="xmark" label={props.stop(grant)} density="compact" onClick={() => props.revoke(grant)} />
                        </span>
                    )}
                </For>
            </div>
        </Show>
    );
}

export function EntryGrants(props: { entry: VaultEntryInfo; revoke: (grant: string) => void }): JSX.Element {
    const label = () => props.entry.label;
    const starting = (prefix: string) => props.entry.grants.filter((g) => g.startsWith(prefix));
    const signer = (g: string) => g.slice(SIGNIN_GRANT.length);
    return (
        <>
            <GrantList
                grants={props.entry.grants.filter(isSiteGrant)}
                label={`Sites ${label()} may be filled into without asking`}
                lead="Agents may fill it on "
                name={siteOf}
                stop={(g) => `Stop filling ${label()} on ${siteOf(g)} without asking`}
                revoke={props.revoke}
            />
            <GrantList
                grants={starting(COMMAND_GRANT)}
                label={`Agents that may run commands with ${label()} without asking`}
                lead="Agents may run commands with it: "
                name={commandGrantLabel}
                stop={(g) => `Stop ${commandGrantLabel(g)} running commands with ${label()} without asking`}
                revoke={props.revoke}
            />
            <GrantList
                grants={starting(SIGNIN_GRANT)}
                label={`Agents that may sign in with ${label()} without asking`}
                lead="Agents may sign in with it: "
                name={signer}
                stop={(g) => `Stop ${signer(g)} signing in with ${label()} without asking`}
                revoke={props.revoke}
            />
        </>
    );
}
