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
    sub: "muxagent acts on GitHub for your agents, only on the repositories you install it on. No token to manage.",
};
export const GITHUB_TOKEN_CHOICE = {
    title: "Add a personal access token",
    sub: "Your own token, stored in your OS keychain. Agents act as you.",
};

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
