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
