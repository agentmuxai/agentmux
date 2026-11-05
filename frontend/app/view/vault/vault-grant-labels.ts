// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// How a standing grant reads in the Vault pane: a site, an agent's commands, or
// an agent's sign-ins on a site.

import { siteOf } from "./vault-use-question";

/** A grant for commands is stored as `command:<agent>:<part>`; a site's is its origin. */
export const COMMAND_GRANT = "command:";

/** An agent's "Always" for signing in with a saved sign-in is stored as
 *  `signin:<agent>|<profile>|<origin>`: the site and profile the person approved. */
export const SIGNIN_GRANT = "signin:";

/** `signin:<agent>|<profile>|<origin>` as a person reads it: `Agent1 on example.com`. */
export function signinGrantLabel(grant: string): string {
    const [agent, , origin] = grant.slice(SIGNIN_GRANT.length).split("|");
    return origin ? `${agent} on ${siteOf(origin)}` : agent;
}

/** A grant for a site, not one for an agent's commands or sign-ins. */
export function isSiteGrant(grant: string): boolean {
    return !grant.startsWith(COMMAND_GRANT) && !grant.startsWith(SIGNIN_GRANT);
}

/** `command:<agent>:<part>` as a person reads it: `Agent1 (value)`. */
export function commandGrantLabel(grant: string): string {
    const rest = grant.slice(COMMAND_GRANT.length);
    const cut = rest.lastIndexOf(":");
    return cut < 0 ? rest : `${rest.slice(0, cut)} (${rest.slice(cut + 1)})`;
}

/** A grant as a person reads it, in the list of what an agent may do without asking. */
export function grantText(g: string): string {
    if (g.startsWith(COMMAND_GRANT)) return `runs commands: ${commandGrantLabel(g)}`;
    return g.startsWith(SIGNIN_GRANT) ? `signs in: ${signinGrantLabel(g)}` : siteOf(g);
}
