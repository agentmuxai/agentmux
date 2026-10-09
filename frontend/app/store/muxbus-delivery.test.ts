// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The cloud delivery state's wording and the status bar's thresholds. */

import { describe, expect, it } from "vitest";

import {
    acceptMuxbusDelivery,
    clockTime,
    muxbusDelivery,
    muxbusDeliveryLine,
    muxbusIndicator,
    muxbusOffersSignIn,
    RECONNECTING_QUIET_MS,
    type MuxBusDeliveryStatus,
} from "./muxbus-delivery";

const NOW = 1_791_000_000_000;

function at(state: MuxBusDeliveryStatus["state"], extra: Partial<MuxBusDeliveryStatus> = {}): MuxBusDeliveryStatus {
    return { state, since_ms: NOW - 60_000, ...extra };
}

describe("muxbusIndicator", () => {
    it("is quiet while connected and for a channel that isn't signed in", () => {
        expect(muxbusIndicator(at("connected"), NOW)).toBeNull();
        expect(muxbusIndicator(at("signed_out"), NOW)).toBeNull();
        expect(muxbusIndicator(null, NOW)).toBeNull();
    });

    it("shows reconnecting only after two minutes of it", () => {
        const s = at("reconnecting", { since_ms: NOW, last_error: "timeout" });
        expect(muxbusIndicator(s, NOW + RECONNECTING_QUIET_MS - 1)).toBeNull();
        const ind = muxbusIndicator(s, NOW + RECONNECTING_QUIET_MS)!;
        expect(ind.tone).toBe("warn");
        expect(ind.label).toBe("MuxBus reconnecting…");
        expect(ind.signIn).toBe(false);
        expect(ind.tip).toContain("timeout");
    });

    it("asks for a sign-in at once when the sign-in stopped working", () => {
        const ind = muxbusIndicator(at("needs_sign_in", { since_ms: NOW }), NOW)!;
        expect(ind.tone).toBe("error");
        expect(ind.label).toBe("MuxBus: sign in");
        expect(ind.signIn).toBe(true);
        expect(ind.tip).toContain(clockTime(NOW));
    });
});

describe("muxbusDeliveryLine", () => {
    it("says each state in plain words", () => {
        expect(muxbusDeliveryLine(at("connected", { account_email: "a@b.c" }))).toBe(
            "Connected as a@b.c. Agents get messages from other computers and GitHub."
        );
        expect(muxbusDeliveryLine(at("signed_out"))).toBe(
            "Not signed in. Agents get no messages from other computers or GitHub."
        );
        expect(muxbusDeliveryLine(at("reconnecting", { since_ms: NOW, last_error: "timeout" }))).toBe(
            `Reconnecting since ${clockTime(NOW)} (timeout). Messages wait at MuxBus until it is back.`
        );
        expect(muxbusDeliveryLine(at("needs_sign_in", { since_ms: NOW, account_email: "a@b.c" }))).toBe(
            `Signed out of MuxBus since ${clockTime(NOW)} as a@b.c. Agents won't get messages from other computers or GitHub until you sign in.`
        );
        expect(muxbusDeliveryLine(null)).toBe("Loading…");
    });

    it("offers Sign in when not signed in, Sign out otherwise", () => {
        expect(muxbusOffersSignIn(at("signed_out"))).toBe(true);
        expect(muxbusOffersSignIn(at("needs_sign_in"))).toBe(true);
        expect(muxbusOffersSignIn(at("connected"))).toBe(false);
        expect(muxbusOffersSignIn(at("reconnecting"))).toBe(false);
    });
});

describe("acceptMuxbusDelivery", () => {
    it("takes a well-formed status and ignores anything else", () => {
        acceptMuxbusDelivery(at("connected"));
        expect(muxbusDelivery()?.state).toBe("connected");
        acceptMuxbusDelivery({ state: "bogus", since_ms: 1 });
        acceptMuxbusDelivery({ state: "needs_sign_in" });
        acceptMuxbusDelivery(null);
        expect(muxbusDelivery()?.state).toBe("connected");
        acceptMuxbusDelivery(at("needs_sign_in"));
        expect(muxbusDelivery()?.state).toBe("needs_sign_in");
    });
});

describe("clockTime", () => {
    it("is HH:MM", () => {
        expect(clockTime(NOW)).toMatch(/^\d{2}:\d{2}$/);
    });
});
