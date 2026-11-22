// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { AtlasMachine } from "./atlas-graph";
import { distinctHosts, HOST_SLOTS, hostIdOf, hostNameOf, hostStyles, ringsDrawn } from "./atlas-hosts";

const machine = (key: string, title: string, extra: Partial<AtlasMachine> = {}): AtlasMachine => ({
    key,
    title,
    badge: "LAN",
    platform: null,
    stale: false,
    seenAtMs: null,
    ...extra,
});
const names = (n: number) => Array.from({ length: n }, (_, i) => `host-${i}`);

describe("a node's host", () => {
    it("is the computer, whichever of its channels the machine is", () => {
        expect(hostNameOf(machine("a", "alpha"))).toBe("alpha");
        expect(hostNameOf(machine("b", "alpha · dev"))).toBe("alpha");
        expect(hostIdOf(machine("b", "Alpha · dev"))).toBe("alpha");
    });

    it("is the machine itself while this computer has no name yet", () => {
        const unnamed = machine("local", "This computer", { unnamed: true });
        expect(hostNameOf(unnamed)).toBe("This computer");
        expect(hostIdOf(unnamed)).toBe("unnamed:local");
    });
});

describe("the ring of each host", () => {
    it("is the same for two nodes of one host, and differs between hosts", () => {
        const styles = hostStyles([machine("m1", "alpha"), machine("m2", "alpha · dev"), machine("m3", "bravo")]);
        expect(styles.get("m1")).toBe(styles.get("m2"));
        const [a, b] = [styles.get("m1")!, styles.get("m3")!];
        expect(`${a.colour}:${a.line}`).not.toBe(`${b.colour}:${b.line}`);
    });

    it("does not depend on the order the machines are listed in", () => {
        const list = names(7).map((n) => machine(`k-${n}`, n));
        const forward = hostStyles(list);
        const backward = hostStyles([...list].reverse());
        for (const m of list) expect(backward.get(m.key)).toEqual(forward.get(m.key));
    });

    it("is the same on another computer that sees the same hosts under other keys", () => {
        const here = hostStyles([machine("local", "alpha", { badge: "this machine" }), machine("lan/bravo", "bravo")]);
        const there = hostStyles([machine("lan/alpha", "alpha"), machine("local", "bravo", { badge: "this machine" })]);
        expect(here.get("local")).toEqual(there.get("lan/alpha"));
        expect(here.get("lan/bravo")).toEqual(there.get("local"));
    });

    it("gives each of up to nine hosts a ring of its own", () => {
        const styles = hostStyles(names(HOST_SLOTS).map((n) => machine(n, n)));
        const rings = new Set([...styles.values()].map((s) => `${s.colour}:${s.line}`));
        expect(rings.size).toBe(HOST_SLOTS);
        expect([...styles.values()].some((s) => s.shared)).toBe(false);
    });

    it("reuses a ring past nine hosts, and says which hosts share one", () => {
        const styles = hostStyles(names(HOST_SLOTS + 3).map((n) => machine(n, n)));
        expect(distinctHosts(styles)).toHaveLength(HOST_SLOTS + 3);
        const shared = [...styles.values()].filter((s) => s.shared);
        expect(shared.length).toBeGreaterThanOrEqual(2);
    });

    it("is drawn for two hosts or more, and not for one", () => {
        expect(ringsDrawn(hostStyles([machine("a", "alpha"), machine("b", "alpha · dev")]))).toBe(false);
        expect(ringsDrawn(hostStyles([machine("a", "alpha"), machine("b", "bravo")]))).toBe(true);
    });
});
