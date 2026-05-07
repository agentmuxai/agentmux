// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { LauncherEvent } from "@/util/launcher-events";
import { update } from "./reducer";
import { initialState, type WindowEntry } from "./types";

const evt = (e: Partial<LauncherEvent> & { event: string }): LauncherEvent => e as LauncherEvent;

const win = (label: string, windowId: string | null = null): WindowEntry => ({ label, windowId });

describe("launcher-event reducer", () => {
    describe("isInstanceLabel filter", () => {
        it("accepts window_opened for promoted pool labels", () => {
            // Promoted pool windows retain their `window-pool-*` prefix.
            // The host-side gates (list_window_instances + launcher_event_bridge)
            // exclude unpromoted pool labels upstream, so by the time
            // a `window-pool-*` label reaches the reducer, it has been
            // promoted to a user window and must be tracked.
            const r = update(initialState(), {
                type: "ApplyEvent",
                event: evt({ event: "window_opened", label: "window-pool-abc" }),
            });
            expect(r.state.knownEntries.size).toBe(1);
        });

        it("ignores window_opened for browser-pane labels", () => {
            const r = update(initialState(), {
                type: "ApplyEvent",
                event: evt({ event: "window_opened", label: "browser-pane-xyz" }),
            });
            expect(r.state.knownEntries.size).toBe(0);
        });

        it("accepts main + window-* labels", () => {
            const s1 = update(initialState(), {
                type: "ApplyEvent",
                event: evt({ event: "window_opened", label: "main" }),
            }).state;
            const s2 = update(s1, {
                type: "ApplyEvent",
                event: evt({ event: "window_opened", label: "window-abc" }),
            }).state;
            expect(s2.knownEntries.size).toBe(2);
        });
    });

    describe("instances derivation", () => {
        it("pins 'main' first then sorts alphabetically", () => {
            let s = initialState();
            for (const label of ["window-z", "window-a", "main", "window-m"]) {
                s = update(s, {
                    type: "ApplyEvent",
                    event: evt({ event: "window_opened", label }),
                }).state;
            }
            expect(s.instances.map((e) => e.label)).toEqual([
                "main",
                "window-a",
                "window-m",
                "window-z",
            ]);
        });
    });

    describe("window_opened preserves prior windowId", () => {
        it("when BackendWindowIdRegistered arrived first", () => {
            const s1 = update(initialState(), {
                type: "ApplyEvent",
                event: evt({
                    event: "backend_window_id_registered",
                    label: "window-x",
                    window_id: "w-123",
                }),
            }).state;
            expect(s1.knownEntries.get("window-x")).toEqual(
                win("window-x", "w-123"),
            );
            const r = update(s1, {
                type: "ApplyEvent",
                event: evt({ event: "window_opened", label: "window-x" }),
            });
            expect(r.state.knownEntries.get("window-x")?.windowId).toBe("w-123");
            expect(r.events[0]).toMatchObject({ type: "window-opened", preservedWindowId: true });
        });
    });

    describe("seed-vs-close race (codex P1/P2 #603/#604)", () => {
        it("close before seed records a tombstone; seed skips that label", () => {
            // Pre-seed close (no entry to delete — tombstone records it)
            const s1 = update(initialState(), {
                type: "ApplyEvent",
                event: evt({ event: "window_closed", label: "window-ghost" }),
            }).state;
            expect(s1.closedBeforeSeed?.has("window-ghost")).toBe(true);
            // Now seed includes the tombstoned label
            const r = update(s1, {
                type: "ApplySeed",
                entries: [win("window-ghost", "w-1"), win("window-real", "w-2")],
            });
            // Tombstoned label NOT re-added; real one is.
            expect(r.state.knownEntries.has("window-ghost")).toBe(false);
            expect(r.state.knownEntries.get("window-real")).toEqual(win("window-real", "w-2"));
            expect(r.state.seedHasHappened).toBe(true);
            expect(r.state.closedBeforeSeed).toBe(null);
            expect(r.events[0]).toMatchObject({
                type: "seeded",
                addedCount: 1,
                tombstonesSkipped: 1,
            });
        });

        it("close BEFORE seed for unknown label preserves instances reference (codex P2 PR #684)", () => {
            // Pre-seed close for a label that's NOT in knownEntries.
            // Tombstone gets added but knownEntries (and therefore the
            // derived instances array) is unchanged; the reducer must
            // preserve the instances reference so the projection layer
            // doesn't re-write atoms.
            const start = initialState();
            const r = update(start, {
                type: "ApplyEvent",
                event: evt({ event: "window_closed", label: "window-ghost" }),
            });
            expect(r.state.instances).toBe(start.instances);
            expect(r.state.knownEntries).toBe(start.knownEntries);
            expect(r.state.closedBeforeSeed?.has("window-ghost")).toBe(true);
        });

        it("close BEFORE seed for an entry that was opened pre-seed: deletes + tombstones + recomputes derived", () => {
            // Open before seed
            const s1 = update(initialState(), {
                type: "ApplyEvent",
                event: evt({ event: "window_opened", label: "window-x" }),
            }).state;
            expect(s1.instances).toHaveLength(1);
            // Close before seed — must delete from knownEntries AND tombstone
            const r = update(s1, {
                type: "ApplyEvent",
                event: evt({ event: "window_closed", label: "window-x" }),
            });
            expect(r.state.knownEntries.has("window-x")).toBe(false);
            expect(r.state.instances).toHaveLength(0);
            expect(r.state.closedBeforeSeed?.has("window-x")).toBe(true);
        });

        it("seed does NOT clobber existing entries (codex P1 #603)", () => {
            // Typed event arrived first, set windowId
            const s1 = update(initialState(), {
                type: "ApplyEvent",
                event: evt({
                    event: "backend_window_id_registered",
                    label: "window-x",
                    window_id: "w-typed",
                }),
            }).state;
            // Snapshot (taken earlier) shows null windowId
            const r = update(s1, {
                type: "ApplySeed",
                entries: [win("window-x", null)],
            });
            // Existing entry preserved — typed event wins.
            expect(r.state.knownEntries.get("window-x")?.windowId).toBe("w-typed");
            expect(r.events[0]).toMatchObject({ addedCount: 0 });
        });
    });

    describe("post-seed close path", () => {
        it("after seed, close events apply directly (no tombstoning)", () => {
            const seeded = update(initialState(), {
                type: "ApplySeed",
                entries: [win("window-x", "w-1")],
            }).state;
            expect(seeded.closedBeforeSeed).toBe(null);
            const r = update(seeded, {
                type: "ApplyEvent",
                event: evt({ event: "window_closed", label: "window-x" }),
            });
            expect(r.state.knownEntries.has("window-x")).toBe(false);
            expect(r.events[0]).toMatchObject({
                type: "window-closed",
                tombstoned: false,
                deletedFromKnown: true,
            });
        });
    });

    describe("backend_window_id changes", () => {
        it("registering a windowId on existing entry updates it", () => {
            const s1 = update(initialState(), {
                type: "ApplyEvent",
                event: evt({ event: "window_opened", label: "window-x" }),
            }).state;
            const r = update(s1, {
                type: "ApplyEvent",
                event: evt({
                    event: "backend_window_id_registered",
                    label: "window-x",
                    window_id: "w-123",
                }),
            });
            expect(r.state.knownEntries.get("window-x")?.windowId).toBe("w-123");
            expect(r.events[0]).toMatchObject({ changed: true });
        });

        it("registering same windowId again is a no-op (state ref preserved)", () => {
            const s1 = update(initialState(), {
                type: "ApplyEvent",
                event: evt({
                    event: "backend_window_id_registered",
                    label: "window-x",
                    window_id: "w-123",
                }),
            }).state;
            const r = update(s1, {
                type: "ApplyEvent",
                event: evt({
                    event: "backend_window_id_registered",
                    label: "window-x",
                    window_id: "w-123",
                }),
            });
            expect(r.state).toBe(s1);
            expect(r.events[0]).toMatchObject({ changed: false });
        });

        it("unregistering a non-existent label is a no-op", () => {
            const start = initialState();
            const r = update(start, {
                type: "ApplyEvent",
                event: evt({ event: "backend_window_id_unregistered", label: "window-ghost" }),
            });
            expect(r.state).toBe(start);
            expect(r.events).toEqual([]);
        });

        it("unregistering an existing label clears windowId", () => {
            const s1 = update(initialState(), {
                type: "ApplyEvent",
                event: evt({
                    event: "backend_window_id_registered",
                    label: "window-x",
                    window_id: "w-123",
                }),
            }).state;
            const r = update(s1, {
                type: "ApplyEvent",
                event: evt({ event: "backend_window_id_unregistered", label: "window-x" }),
            });
            expect(r.state.knownEntries.get("window-x")?.windowId).toBe(null);
        });
    });

    describe("instance_assigned / instance_released", () => {
        it("assigned creates entry only if missing", () => {
            const s1 = update(initialState(), {
                type: "ApplyEvent",
                event: evt({ event: "window_opened", label: "window-x" }),
            }).state;
            const r = update(s1, {
                type: "ApplyEvent",
                event: evt({ event: "window_instance_assigned", label: "window-x" }),
            });
            expect(r.state).toBe(s1); // entry already existed
            expect(r.events[0]).toMatchObject({ createdMissing: false });
        });

        it("released routes through close handler (deletes + audit event)", () => {
            const s1 = update(initialState(), {
                type: "ApplyEvent",
                event: evt({ event: "window_opened", label: "window-x" }),
            }).state;
            const seeded = update(s1, {
                type: "ApplySeed",
                entries: [],
            }).state;
            const r = update(seeded, {
                type: "ApplyEvent",
                event: evt({ event: "window_instance_released", label: "window-x" }),
            });
            expect(r.state.knownEntries.has("window-x")).toBe(false);
            expect(r.events[0]).toMatchObject({
                type: "window-instance-released",
                deletedFromKnown: true,
            });
        });
    });

    describe("ReconcileFromSnapshot", () => {
        it("removes labels absent from snapshot (closed windows actually disappear)", () => {
            // Codex P3 PR #732: ApplySeed is additive (doesn't remove
            // labels missing from the snapshot) so the previous
            // InstancePanel-on-open-refresh path left closed windows
            // visible in dev mode. Reconcile REPLACES wholesale.
            const seeded = update(initialState(), {
                type: "ApplySeed",
                entries: [
                    { label: "window-a", windowId: null },
                    { label: "window-b", windowId: null },
                    { label: "window-c", windowId: null },
                ],
            }).state;
            expect(seeded.knownEntries.size).toBe(3);

            // window-c was closed in the host; snapshot only has a, b.
            const r = update(seeded, {
                type: "ReconcileFromSnapshot",
                entries: [
                    { label: "window-a", windowId: null },
                    { label: "window-b", windowId: null },
                ],
            });
            expect(r.state.knownEntries.has("window-c")).toBe(false);
            expect(r.state.knownEntries.size).toBe(2);
            expect(r.events[0]).toMatchObject({
                type: "reconciled",
                addedCount: 0,
                removedCount: 1,
                totalAfter: 2,
            });
        });

        it("adds new labels and removes missing ones in one pass", () => {
            const seeded = update(initialState(), {
                type: "ApplySeed",
                entries: [{ label: "window-a", windowId: null }],
            }).state;
            const r = update(seeded, {
                type: "ReconcileFromSnapshot",
                entries: [
                    { label: "window-b", windowId: null },
                    { label: "window-c", windowId: null },
                ],
            });
            expect(r.state.knownEntries.has("window-a")).toBe(false);
            expect(r.state.knownEntries.has("window-b")).toBe(true);
            expect(r.state.knownEntries.has("window-c")).toBe(true);
            expect(r.events[0]).toMatchObject({
                type: "reconciled",
                addedCount: 2,
                removedCount: 1,
                totalAfter: 2,
            });
        });

        it("filters non-instance labels (parity with ApplySeed)", () => {
            // `window-pool-*` IS an instance label at this layer
            // (promoted pool windows keep the prefix and ARE user-
            // visible; unpromoted ones are filtered host-side in
            // list_window_instances). Only browser-pane-* is rejected.
            const r = update(initialState(), {
                type: "ReconcileFromSnapshot",
                entries: [
                    { label: "window-a", windowId: null },
                    { label: "window-pool-promoted", windowId: null },
                    { label: "browser-pane-x", windowId: null },
                ],
            });
            expect(r.state.knownEntries.size).toBe(2);
            expect(r.state.knownEntries.has("window-a")).toBe(true);
            expect(r.state.knownEntries.has("window-pool-promoted")).toBe(true);
            expect(r.state.knownEntries.has("browser-pane-x")).toBe(false);
        });
    });

    describe("saga + drift + unknown variants", () => {
        it("hwnd_drift_detected emits drift-detected audit event", () => {
            const start = initialState();
            const r = update(start, {
                type: "ApplyEvent",
                event: evt({ event: "hwnd_drift_detected" }),
            });
            expect(r.state).toBe(start);
            expect(r.events[0]).toMatchObject({ type: "drift-detected" });
        });

        it("corrective_window_move + host_should_quit emit saga audit events", () => {
            const start = initialState();
            const r1 = update(start, {
                type: "ApplyEvent",
                event: evt({ event: "corrective_window_move" }),
            });
            expect(r1.events[0]).toMatchObject({ type: "saga-event-observed" });
            const r2 = update(start, {
                type: "ApplyEvent",
                event: evt({ event: "host_should_quit" }),
            });
            expect(r2.events[0]).toMatchObject({ type: "saga-event-observed" });
        });

        it("unknown variant is ignored (forward-compat)", () => {
            const start = initialState();
            const r = update(start, {
                type: "ApplyEvent",
                event: evt({ event: "future_event_we_dont_know_yet" } as any),
            });
            expect(r.state).toBe(start);
            expect(r.events[0]).toMatchObject({ type: "unknown-variant-ignored" });
        });
    });

    // Master spec §8.14 — subscriber idempotency contract. Pure
    // reducer property: applying the same event N times to a stable
    // state produces the same state as applying it once. Per-event-kind
    // because some are inherently non-idempotent (e.g. window_closed
    // deletes; the first call deletes, the second is a known no-op).
    describe("§8.14 idempotency property tests", () => {
        const idempotentApplyEvent = (
            initial: ReturnType<typeof initialState>,
            event: Partial<LauncherEvent> & { event: string },
        ) => {
            const once = update(initial, { type: "ApplyEvent", event: evt(event) }).state;
            const twice = update(once, { type: "ApplyEvent", event: evt(event) }).state;
            const thrice = update(twice, { type: "ApplyEvent", event: evt(event) }).state;
            return { once, twice, thrice };
        };

        it("repeated window_opened for the same label folds to the first apply", () => {
            const r = idempotentApplyEvent(initialState(), {
                event: "window_opened",
                label: "window-x",
            });
            expect(r.twice.knownEntries).toEqual(r.once.knownEntries);
            expect(r.thrice.knownEntries).toEqual(r.once.knownEntries);
            expect(r.twice.instances).toEqual(r.once.instances);
        });

        it("repeated backend_window_id_registered with same id is a no-op past the first", () => {
            const r = idempotentApplyEvent(initialState(), {
                event: "backend_window_id_registered",
                label: "window-x",
                window_id: "w-123",
            });
            expect(r.twice.knownEntries.get("window-x")).toEqual(r.once.knownEntries.get("window-x"));
            expect(r.thrice.knownEntries.get("window-x")).toEqual(r.once.knownEntries.get("window-x"));
        });

        it("repeated hwnd_drift_detected does not mutate state (audit-only event)", () => {
            const start = initialState();
            const r = idempotentApplyEvent(start, { event: "hwnd_drift_detected" });
            expect(r.once).toBe(start); // ref-equal: audit-only events don't allocate
            expect(r.twice).toBe(start);
            expect(r.thrice).toBe(start);
        });

        it("repeated window_instance_assigned for an existing entry is a no-op past the first", () => {
            const seeded = update(initialState(), {
                type: "ApplyEvent",
                event: evt({ event: "window_opened", label: "window-x" }),
            }).state;
            const r = idempotentApplyEvent(seeded, {
                event: "window_instance_assigned",
                label: "window-x",
            });
            expect(r.twice).toBe(r.once);
            expect(r.thrice).toBe(r.once);
        });

        it("repeated unknown variant is forward-compat no-op every time", () => {
            const start = initialState();
            const r = idempotentApplyEvent(start, {
                event: "future_event_we_dont_know_yet",
            });
            expect(r.once).toBe(start); // ref-equal: unknown variants don't allocate
            expect(r.twice).toBe(start);
            expect(r.thrice).toBe(start);
        });

        it("a randomised duplicate-bursting sequence produces the same final state as the dedup'd sequence", () => {
            // Property: for any sequence of events S, applying S with each event
            // duplicated 1-5x produces the same final state as applying S once.
            // Holds for the events listed above (idempotent per spec §8.14).
            //
            // Labels MUST be accepted by `isInstanceLabel` (`main` or
            // `window-*`) — codex P2 PR #709 round 3 caught a prior version
            // using `a`/`b`/`c` which were filtered out, leaving both
            // sequences producing an empty state and the property holding
            // vacuously. The non-empty-knownEntries assertion below is the
            // anti-vacuity guard.
            const baseSequence: LauncherEvent[] = [
                evt({ event: "window_opened", label: "window-a" }),
                evt({ event: "backend_window_id_registered", label: "window-a", window_id: "w-1" }),
                evt({ event: "window_opened", label: "window-b" }),
                evt({ event: "window_instance_assigned", label: "window-b" }),
                evt({ event: "hwnd_drift_detected" }),
                evt({ event: "window_opened", label: "window-c" }),
                evt({ event: "backend_window_id_registered", label: "window-c", window_id: "w-3" }),
            ];

            const apply = (events: LauncherEvent[]) => {
                let s = initialState();
                for (const e of events) {
                    s = update(s, { type: "ApplyEvent", event: e }).state;
                }
                return s;
            };

            const expected = apply(baseSequence);

            // Anti-vacuity: if the reducer's label filter ever changes,
            // the empty-state-equals-empty-state trap returns. Assert the
            // base run actually mutates state.
            expect(expected.knownEntries.size).toBe(3);
            expect(expected.instances.length).toBe(3);

            // 30 deterministic seeds.
            for (let seed = 1; seed <= 30; seed++) {
                let t = seed >>> 0;
                const next = () => {
                    t = (t + 0x6d2b79f5) >>> 0;
                    let r = Math.imul(t ^ (t >>> 15), 1 | t);
                    r = (r + Math.imul(r ^ (r >>> 7), 61 | r)) ^ r;
                    return ((r ^ (r >>> 14)) >>> 0) / 4294967296;
                };
                const inflated: LauncherEvent[] = [];
                for (const e of baseSequence) {
                    const n = 1 + Math.floor(next() * 5);
                    for (let i = 0; i < n; i++) inflated.push(e);
                }
                const got = apply(inflated);
                expect(got.knownEntries).toEqual(expected.knownEntries);
                expect(got.instances).toEqual(expected.instances);
                expect(got.seedHasHappened).toEqual(expected.seedHasHappened);
            }
        });
    });

    describe("Purity", () => {
        it("does not mutate input state", () => {
            const start = update(initialState(), {
                type: "ApplyEvent",
                event: evt({ event: "window_opened", label: "window-x" }),
            }).state;
            const snapshot = {
                knownSize: start.knownEntries.size,
                instancesLen: start.instances.length,
                closedSize: start.closedBeforeSeed?.size ?? -1,
            };
            update(start, {
                type: "ApplyEvent",
                event: evt({ event: "window_closed", label: "window-x" }),
            });
            expect(start.knownEntries.size).toBe(snapshot.knownSize);
            expect(start.instances.length).toBe(snapshot.instancesLen);
            expect(start.closedBeforeSeed?.size ?? -1).toBe(snapshot.closedSize);
        });
    });
});
