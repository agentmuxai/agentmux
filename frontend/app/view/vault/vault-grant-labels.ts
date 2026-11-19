// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// How a standing grant reads in the Vault pane: a site, an agent's commands, or
// an agent's sign-ins on a site.

import { t } from "@/app/i18n/i18n";
import { siteOf } from "./vault-use-question";

/** A grant for commands is stored as `command:<agent>:<part>`; a site's is its origin. */
export const COMMAND_GRANT = "command:";

/** An agent's "Always" for signing in with a saved sign-in is stored as
 *  `signin:<agent>|<profile>|<origin>`: the site and profile the person approved. For a login that is shared with
 *  the agent, `<agent>` is the agent's permanent id and more follows the origin: for a sign-in the browser gives to
 *  a site's HTTP prompt, `prompt:site|<realm>`, and for a proxy's, `prompt:proxy|<realm>`. */
export const SIGNIN_GRANT = "signin:";

/** The name of the agent a grant was stored under, where the pane knows it (a shared login's grant names the
 *  agent by its id). */
export type AgentNames = (agent: string) => string | undefined;

/** The agent a sign-in grant was stored under: its name, or for a shared login its id. */
export function signinGrantAgent(grant: string): string {
    return grant.slice(SIGNIN_GRANT.length).split("|")[0];
}

/** A sign-in grant as a person reads it: `Agent1 on example.com`, with the prompt it answers when it is one for
 *  an HTTP sign-in prompt. `names` gives the agent's name for an id; without one the stored text is shown. */
export function signinGrantLabel(grant: string, names?: AgentNames): string {
    const [stored, , origin, kind, ...rest] = grant.slice(SIGNIN_GRANT.length).split("|");
    const agent = names?.(stored) ?? stored;
    if (!origin) return agent;
    const site = siteOf(origin);
    if (kind === "prompt:proxy") return t("vault.grant.signinProxy", { agent, site });
    if (kind === "prompt:site") {
        // A realm may hold a `|` of its own.
        const realm = rest.join("|");
        return realm ? t("vault.grant.signinPromptRealm", { agent, site, realm }) : t("vault.grant.signinPrompt", { agent, site });
    }
    return t("vault.grant.signin", { agent, site });
}

/** A setting's right to read the entry (an API key kept for it) is stored as `setting:<settings key>`. */
export const SETTING_GRANT = "setting:";

const SETTING_NAMES: Record<string, string> = { "voice:groqApiKey": "Voice input's Groq key" };

/** `setting:voice:groqApiKey` as a person reads it: `Voice input's Groq key`. */
export function settingGrantLabel(grant: string): string {
    const key = grant.slice(SETTING_GRANT.length);
    return SETTING_NAMES[key] ?? key;
}

/** A grant for a site, not one for an agent's commands or sign-ins, or for a setting. */
export function isSiteGrant(grant: string): boolean {
    return !grant.startsWith(COMMAND_GRANT) && !grant.startsWith(SIGNIN_GRANT) && !grant.startsWith(SETTING_GRANT);
}

/** `command:<agent>:<part>` as a person reads it: `Agent1 (value)`. */
export function commandGrantLabel(grant: string): string {
    const rest = grant.slice(COMMAND_GRANT.length);
    const cut = rest.lastIndexOf(":");
    return cut < 0 ? rest : `${rest.slice(0, cut)} (${rest.slice(cut + 1)})`;
}

/** A grant as a person reads it, in the list of what an agent may do without asking. */
export function grantText(g: string, names?: AgentNames): string {
    if (g.startsWith(COMMAND_GRANT)) return `runs commands: ${commandGrantLabel(g)}`;
    if (g.startsWith(SETTING_GRANT)) return `read by a setting: ${settingGrantLabel(g)}`;
    return g.startsWith(SIGNIN_GRANT) ? `signs in: ${signinGrantLabel(g, names)}` : siteOf(g);
}
