// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether cloud (MuxBus) messages are reaching this channel's agents, as srv
 * derives it (`muxbus::delivery_status`): `muxbus.status`'s `delivery`, kept
 * live by the `muxbus:status` event. One signal for every surface (the status
 * bar indicator, Settings), and the pure wording they share.
 */

import { createSignal } from "solid-js";

import type { MuxBusDeliveryStatus } from "@/types/rpc/MuxBusDeliveryStatus";

export type { MuxBusDeliveryStatus } from "@/types/rpc/MuxBusDeliveryStatus";
export type { MuxBusDeliveryState } from "@/types/rpc/MuxBusDeliveryState";

/** The `muxbus:status` WebSocket event. */
export const MUXBUS_STATUS_EVENT = "muxbus:status";

/**
 * How long "reconnecting" stays quiet before the status bar shows it: a
 * reconnect or token refresh that recovers sooner is not worth a glance.
 * srv tells the agents after the same time.
 */
export const RECONNECTING_QUIET_MS = 2 * 60 * 1000;

const [muxbusDelivery, setMuxbusDelivery] = createSignal<MuxBusDeliveryStatus | null>(null);

/** The latest delivery status, or null before srv has said. */
export { muxbusDelivery };

/** Take a status from `muxbus.status` or the event; ignores anything malformed. */
export function acceptMuxbusDelivery(next: unknown): void {
    const s = next as Partial<MuxBusDeliveryStatus> | null | undefined;
    if (
        s?.state !== "signed_out" &&
        s?.state !== "connected" &&
        s?.state !== "reconnecting" &&
        s?.state !== "needs_sign_in"
    ) {
        return;
    }
    if (typeof s.since_ms !== "number") return;
    setMuxbusDelivery(s as MuxBusDeliveryStatus);
}

/** `ms` as local `HH:MM`. */
export function clockTime(ms: number): string {
    return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false });
}

export type MuxBusIndicatorTone = "warn" | "error";

export interface MuxBusIndicator {
    tone: MuxBusIndicatorTone;
    label: string;
    tip: string;
    /** Clicking it starts the sign-in. */
    signIn: boolean;
}

/**
 * What the status bar shows, or null for nothing: connected is quiet, a
 * channel that isn't signed in shows nothing new, and reconnecting stays
 * quiet for its first {@link RECONNECTING_QUIET_MS}.
 */
export function muxbusIndicator(s: MuxBusDeliveryStatus | null | undefined, nowMs: number): MuxBusIndicator | null {
    if (!s) return null;
    switch (s.state) {
        case "needs_sign_in":
            return {
                tone: "error",
                label: "MuxBus: sign in",
                tip: `Signed out of MuxBus since ${clockTime(s.since_ms)}: agents get no messages from other computers or GitHub. Click to sign in.`,
                signIn: true,
            };
        case "reconnecting":
            if (nowMs - s.since_ms < RECONNECTING_QUIET_MS) return null;
            return {
                tone: "warn",
                label: "MuxBus reconnecting…",
                tip: `Can't reach MuxBus since ${clockTime(s.since_ms)}${s.last_error ? ` (${s.last_error})` : ""}. Retrying on its own.`,
                signIn: false,
            };
        default:
            return null;
    }
}

/** The state in one line, for Settings. */
export function muxbusDeliveryLine(s: MuxBusDeliveryStatus | null | undefined): string {
    if (!s) return "Loading…";
    const as = s.account_email ? ` as ${s.account_email}` : "";
    switch (s.state) {
        case "connected":
            return `Connected${as}. Agents get messages from other computers and GitHub.`;
        case "reconnecting":
            return `Reconnecting since ${clockTime(s.since_ms)}${s.last_error ? ` (${s.last_error})` : ""}. Messages wait at MuxBus until it is back.`;
        case "needs_sign_in":
            return `Signed out of MuxBus since ${clockTime(s.since_ms)}${as}. Agents won't get messages from other computers or GitHub until you sign in.`;
        case "signed_out":
            return "Not signed in. Agents get no messages from other computers or GitHub.";
    }
}

/** Settings offers "Sign in" in these states, "Sign out" in the others. */
export function muxbusOffersSignIn(s: MuxBusDeliveryStatus | null | undefined): boolean {
    return !s || s.state === "signed_out" || s.state === "needs_sign_in";
}
