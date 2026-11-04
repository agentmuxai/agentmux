// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What an agent did to a Google account (`google:notice`, srv's `identity::accounts_changed`): a
 * grant turned on or off, an account disconnected. A short-lived notice that asks nothing; the
 * account's log in Connectors keeps the record.
 */
import { pushNotification } from "@/app/store/flash-notifications";
import type { GoogleNoticeEvent } from "@/types/rpc/GoogleNoticeEvent";

/** Show one line as a notice; ignores anything without text. */
export function acceptGoogleNotice(event: Partial<GoogleNoticeEvent> | null | undefined): void {
    const text = typeof event?.text === "string" ? event.text.trim() : "";
    if (!text) return;
    pushNotification({
        icon: "fa-brands fa-google",
        title: "Google",
        message: text,
        timestamp: new Date().toISOString(),
        type: "info",
    });
}
