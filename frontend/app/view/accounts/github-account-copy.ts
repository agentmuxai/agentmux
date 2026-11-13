// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What the GitHub account choices and details say. There are two ways to give agents GitHub:
// AgentMux's own GitHub App (muxagent, through the AgentMux Cloud sign-in), where agents act as
// muxagent[bot] on the repositories the person installed it on; or a personal access token the person
// adds, where agents act as that person. GitHub has no separate "API key", so a token is one choice
// whatever kind an older account was saved with.

import type { Account } from "@/app/view/identity/identity-model";

import { isBrokeredGithub } from "./oauth-account-state";

/** The two choices under the GitHub tile. */
export const GITHUB_APP_CHOICE = {
    title: "Use AgentMux's GitHub App",
    sub: "No GitHub credentials needed on this computer. You choose the repositories; agents push and open PRs and issues there as muxagent[bot]. Needs an AgentMux Cloud sign-in.",
};
export const GITHUB_TOKEN_CHOICE = {
    title: "Add a personal access token",
    sub: "Your own token, kept in your OS keychain. Agents you link it to act as you.",
};

/** What agents can and can't do through the App (the permissions `github_tokens::permissions_for` asks for). */
export const GITHUB_APP_CAN =
    "Agents can clone, push and open pull requests and issues on the repositories you installed it on, and change CI workflows only if you allow that agent.";
export const GITHUB_APP_CANNOT = "They can't reach other repositories, create repositories or change settings.";

/** The Kind select's two GitHub options. */
export const GITHUB_APP_KIND_LABEL = "AgentMux's GitHub App (muxagent)";
export const GITHUB_TOKEN_KIND_LABEL = "Personal access token";

/** Whether a GitHub account holds a token the person added (a token, or an older "API key"). */
export function isGithubToken(a: Pick<Account, "provider" | "kind">): boolean {
    return a.provider === "github" && (a.kind === "pat" || a.kind === "api_key");
}

/** The account's kind in words, or null when this account's kind keeps its general label. */
export function githubKindLabel(a: Pick<Account, "provider" | "kind" | "context">): string | null {
    if (isBrokeredGithub(a)) return "AgentMux's GitHub App";
    return isGithubToken(a) ? GITHUB_TOKEN_KIND_LABEL : null;
}

/** The short form for a card's footer, or null for the general one. */
export function githubKindShort(a: Pick<Account, "provider" | "kind" | "context">): string | null {
    if (isBrokeredGithub(a)) return "GitHub App";
    return isGithubToken(a) ? "Token" : null;
}

/** Whether a GitHub account is the App's and still works (srv gives agents the App's tokens only then). */
function appInUse(a: Pick<Account, "provider" | "kind" | "context" | "status">): boolean {
    return isBrokeredGithub(a) && a.status !== "needs_reauth";
}

/**
 * The one sentence at the top of the GitHub section: which way agents reach GitHub now. The App wins
 * when it works (srv routes git and gh through it and keeps the person's login from agents); a token
 * or the person's own OAuth sign-in reaches only the agents it is linked to; with none of these, an agent has the
 * person's own login. A computer with gh-agent is the exception to all of it: gh-agent gives agents their identity
 * there, so that is the status (`ghAgentHere`, from srv: the frontend can't tell).
 */
export function githubStatusLine(
    accounts: readonly Pick<Account, "provider" | "kind" | "context" | "status">[],
    ghAgentHere = false
): string {
    if (ghAgentHere) return GITHUB_GH_AGENT_STATUS;
    const github = accounts.filter((a) => a.provider === "github");
    if (github.some(appInUse)) {
        return "Connected through AgentMux's GitHub App. Agents act as muxagent[bot], only on the repositories you installed it on.";
    }
    if (github.some(isBrokeredGithub)) {
        return "AgentMux's GitHub App needs to be reconnected. Until then agents don't use it.";
    }
    if (github.some(isGithubToken)) {
        return "Using your personal access token. The agents you link it to act as you, with whatever the token allows.";
    }
    // A sign-in made with the person's own OAuth client: its access token reaches the agents it is linked to.
    if (github.some((a) => a.kind === "oauth")) {
        return "Connected with your own GitHub sign-in. The agents you link it to act as you, with whatever it allows.";
    }
    return "Not connected. Agents use your own GitHub login on this computer, if there is one, and act as you.";
}

/** The status on a computer with gh-agent: it gives agents their identity, whichever account is connected. */
export const GITHUB_GH_AGENT_STATUS = "This computer's gh-agent gives agents their GitHub identity.";

/** One sentence on who agents act as through this account, or null when it isn't one of the two. */
export function githubActsAs(a: Pick<Account, "provider" | "kind" | "context">): string | null {
    if (isBrokeredGithub(a)) {
        return "Agents act on GitHub as muxagent[bot], with short-lived tokens, only on the repositories you installed AgentMux's GitHub App on.";
    }
    if (isGithubToken(a)) {
        const login = a.context?.github_login;
        return `Agents act on GitHub as ${login ? `@${login}` : "you"} with this token.`;
    }
    return null;
}
