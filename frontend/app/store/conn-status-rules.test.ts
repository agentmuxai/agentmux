// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { connBarShows, resyncsOnConnChange } from "./conn-status-rules";

const cs = (status: string, extra: Partial<ConnStatus> = {}): ConnStatus => ({
    connection: "area54",
    status,
    connected: status === "connected",
    hasconnected: false,
    activeconnnum: 0,
    error: null,
    ...extra,
});

describe("connBarShows", () => {
    it("shows what went wrong", () => {
        expect(connBarShows(cs("error", { error: "Connection refused" }))).toBe(true);
        expect(connBarShows(cs("disconnected", { hasconnected: true }))).toBe(true);
    });

    // On SSH the terminal shows the connecting itself, ssh's password prompt
    // included, at the top of the pane, where the bar covered it.
    it("leaves a connecting terminal to ssh's own output", () => {
        expect(connBarShows(cs("connecting"))).toBe(false);
        expect(connBarShows(cs("connected", { hasconnected: true }))).toBe(false);
    });

    // A new pane, or a Files pane before its first listing: nothing has
    // happened yet, so there is nothing to report.
    it("says nothing before anything has happened", () => {
        expect(connBarShows(cs("disconnected"))).toBe(false);
        expect(connBarShows(null)).toBe(false);
        expect(connBarShows(undefined)).toBe(false);
    });
});

describe("resyncsOnConnChange", () => {
    // A finished pane on the connection starts again over it: after a
    // Reconnect, or another pane's login.
    it("re-syncs when the connection comes up", () => {
        expect(resyncsOnConnChange(cs("error"), cs("connected"))).toBe(true);
        expect(resyncsOnConnChange(cs("connecting"), cs("connected"))).toBe(true);
    });

    // Re-syncing when it went restarted ssh at once: against a host that is
    // down, a loop of connection attempts (#4513).
    it("never re-syncs when the connection goes", () => {
        expect(resyncsOnConnChange(cs("connected"), cs("error"))).toBe(false);
        expect(resyncsOnConnChange(cs("connected"), cs("disconnected"))).toBe(false);
        expect(resyncsOnConnChange(cs("error"), cs("connecting"))).toBe(false);
        expect(resyncsOnConnChange(cs("connected"), cs("connected"))).toBe(false);
    });

    it("re-syncs when the pane moves to another connection", () => {
        expect(resyncsOnConnChange(cs("connected"), cs("connected", { connection: "db1" }))).toBe(true);
        expect(resyncsOnConnChange(null, cs("disconnected"))).toBe(true);
    });
});
