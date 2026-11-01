// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * How a toast is announced and how long it stays
 * (docs/reports/REPORT_ACTION_RESULTS_ALWAYS_VISIBLE_ACROSS_WINDOWS_2026_10_31.md §4 item 11):
 * a result is exposed to a screen reader without taking focus (`role=status`, `role=alert` for an error),
 * a toast with an action stays until it is acted on or closed, and any other lasts at least six seconds.
 */

/** The shortest a toast that expires is shown. */
export const MIN_TOAST_MS = 6000;

/** The live-region role of `n`: what it says, else `alert` for an error and `status` for the rest. */
export function liveRole(n: Pick<NotificationType, "type" | "live">): "status" | "alert" {
    return n.live ?? (n.type === "error" ? "alert" : "status");
}

/** `n` as it is kept: an action toast never expires, any other toast's expiry is at least `MIN_TOAST_MS` away. */
export function withToastLifetime(n: NotificationType, now: number): NotificationType {
    if (n.actions?.length) return n.expiration == null ? n : { ...n, expiration: undefined };
    if (n.expiration != null && n.expiration < now + MIN_TOAST_MS) return { ...n, expiration: now + MIN_TOAST_MS };
    return n;
}
