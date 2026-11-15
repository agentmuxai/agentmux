// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A browser that was never signed in to a server of its own (or whose sign-in has ended) must be told so. Without this
// it showed "Reconnecting to AgentMux" and then "AgentMux couldn't start this window ... Press Restore", which reads as
// a crash and offers a button that cannot help: only `agentmux open`, run in the server's terminal, signs a browser in.

import { FONT, mountRoot, StartupFailureHandled } from "@/app/init/error-display";
import { authHeaders } from "@/app/store/auth-headers";

/** Why this browser cannot use the server: it holds no sign-in, or the server no longer accepts the one it holds. */
export type SignInProblem = "none" | "ended";

/** A route that needs a signed-in browser and costs the server nothing: a 401 here means the sign-in is not accepted. */
const PROBE = "/api/bus/agents";

/**
 * What is wrong with this browser's sign-in, or `null` when nothing is known to be (including when the server cannot be
 * asked: that is a different failure, and the usual startup recovery handles it). The server is always asked, because a
 * page behind a proxy that adds the key holds no token and is signed in all the same.
 */
export async function signInProblem(
    hasToken: boolean,
    fetchImpl: (input: string, init: RequestInit) => Promise<{ status: number }> = (input, init) => fetch(input, init)
): Promise<SignInProblem | null> {
    try {
        const resp = await fetchImpl(PROBE, { headers: authHeaders(), credentials: "same-origin" });
        if (resp.status !== 401) return null;
        return hasToken ? "ended" : "none";
    } catch {
        return null;
    }
}

const COMMAND = "agentmux open";

/** The words of the card for each problem: what is so, then what to do. */
export function notSignedInText(problem: SignInProblem): { title: string; lead: string } {
    return {
        title: "This browser isn't signed in",
        lead:
            problem === "ended"
                ? "This browser's sign-in has ended. It was signed out, or the server was restarted."
                : "AgentMux is running, but this browser hasn't been signed in to it.",
    };
}

/** Show the card in place of the app. It has no Restore button: nothing the page can do signs it in. */
export function showNotSignedIn(problem: SignInProblem): void {
    const { title, lead } = notSignedInText(problem);
    const main = mountRoot();
    const card = document.createElement("div");
    card.setAttribute("role", "alert");
    card.style.cssText =
        "max-width:560px;margin:14vh auto 0;padding:28px 32px;border-radius:12px;background:#17181b;" +
        "border:1px solid rgba(255,255,255,0.08);box-shadow:0 8px 40px rgba(0,0,0,0.5);" +
        `font-family:${FONT};color:#e8e8e8;`;

    const h = document.createElement("h2");
    h.textContent = title;
    h.style.cssText = "margin:0 0 10px;font-size:18px;color:#ffd479;";

    const p = document.createElement("p");
    p.textContent = lead;
    p.style.cssText = "margin:0 0 14px;font-size:14px;line-height:1.5;color:rgba(255,255,255,0.8);";

    const how = document.createElement("p");
    how.style.cssText = "margin:0;font-size:14px;line-height:1.5;color:rgba(255,255,255,0.8);";
    const code = document.createElement("code");
    code.textContent = COMMAND;
    code.style.cssText = "padding:1px 6px;border-radius:5px;background:rgba(255,255,255,0.1);";
    how.append(
        "To sign in, run ",
        code,
        " in a terminal on the machine where AgentMux runs, as the user that started it, and open the address it prints in this browser."
    );

    card.append(h, p, how);
    main.appendChild(card);
}

/** Check this browser's sign-in before the app starts; when it is not one, say so and stop the startup quietly. */
export async function requireSignIn(hasToken: boolean): Promise<void> {
    const problem = await signInProblem(hasToken);
    if (problem == null) return;
    showNotSignedIn(problem);
    // `bootstrap` treats this as a failure already handled: no reload loop, no second card.
    throw new StartupFailureHandled(`this browser is not signed in (${problem})`);
}
