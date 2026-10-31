// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A sign-in link another machine's login produced (`identity:signinlink`, srv's `signin_notice`):
 * a notice that stays until closed, with an Open button that opens the link in the browser. The
 * credentials stay on the machine that is signing in; only the link comes here.
 */

import { getApi } from "@/app/store/app-api";
import { pushNotification } from "@/app/store/flash-notifications";
import { setNotificationAction } from "@/app/notification/usenotification";
import type { SigninLinkEvent } from "@/types/rpc/SigninLinkEvent";

const PROVIDER_NAMES: Record<string, string> = { claude: "Claude", codex: "Codex" };

/** The link is the provider's own https page; srv checked it, and it is checked again before opening. */
export function isSigninLink(url: unknown): url is string {
    if (typeof url !== "string") return false;
    try {
        const u = new URL(url);
        return u.protocol === "https:" && !u.username && !u.password;
    } catch {
        return false;
    }
}

/** Show a link as a notice; ignores anything malformed. */
export function acceptSigninLink(event: Partial<SigninLinkEvent> | null | undefined): void {
    if (!event || !isSigninLink(event.url)) return;
    const url = event.url;
    const provider = PROVIDER_NAMES[event.provider ?? ""] ?? "your provider";
    const machine = typeof event.machine === "string" && event.machine ? event.machine : "another machine";
    const actionKey = `signin-link:${url}`;
    setNotificationAction(actionKey, () => getApi().openExternal(url));
    pushNotification({
        id: actionKey,
        icon: "fa-right-to-bracket",
        title: `Sign ${machine} in to ${provider}`,
        message: "Open the link, sign in, and the account is added on that machine.",
        timestamp: new Date().toISOString(),
        type: "info",
        persistent: true,
        actions: [{ label: "Open", actionKey, color: "green" }],
    });
}
