// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Drift-storm regression guard tests for the launcher-event bridge.
// See `docs/specs/ANALYSIS_DRIFT_STORM_RENDERER_CRASH_2026-05-06.md`.

import { beforeEach, describe, expect, it } from "vitest";
import {
    __resetDedupForTests,
    launcherEventDedupStats,
    shouldDispatchLauncherEvent,
    type LauncherEvent,
} from "./launcher-events";

const evt = (over: Partial<LauncherEvent> & { event: string; version: number }): LauncherEvent =>
    over as LauncherEvent;

describe("launcher-event per-key dedup", () => {
    beforeEach(() => __resetDedupForTests());

    it("admits the first event for a given (kind, label, hwnd)", () => {
        const e = evt({ event: "hwnd_drift_detected", version: 13, label: "window-pool-x", hwnd: 100 });
        expect(shouldDispatchLauncherEvent(e)).toBe(true);
        expect(launcherEventDedupStats()).toEqual({ tracked: 1, suppressed: 0 });
    });

    it("suppresses re-emission of the same (kind, label, hwnd) at the same version", () => {
        const e = evt({ event: "hwnd_drift_detected", version: 13, label: "window-pool-x", hwnd: 100 });
        expect(shouldDispatchLauncherEvent(e)).toBe(true);
        // The drift-storm bug: same event re-arrives many times with same version.
        for (let i = 0; i < 100; i++) {
            expect(shouldDispatchLauncherEvent(e)).toBe(false);
        }
        expect(launcherEventDedupStats()).toEqual({ tracked: 1, suppressed: 100 });
    });

    it("admits a higher-versioned event for the same key", () => {
        const a = evt({ event: "hwnd_drift_detected", version: 13, label: "x", hwnd: 1 });
        const b = evt({ event: "hwnd_drift_detected", version: 17, label: "x", hwnd: 1 });
        expect(shouldDispatchLauncherEvent(a)).toBe(true);
        expect(shouldDispatchLauncherEvent(b)).toBe(true);
    });

    it("treats different event kinds at the same version as distinct keys", () => {
        // Launcher versions are global-monotonic; same version on different
        // event kinds is impossible upstream, but the dedup must not couple
        // unrelated events even hypothetically.
        expect(shouldDispatchLauncherEvent(evt({ event: "hwnd_drift_detected", version: 13, label: "x" }))).toBe(true);
        expect(shouldDispatchLauncherEvent(evt({ event: "window_opened", version: 13, label: "x" }))).toBe(true);
        expect(shouldDispatchLauncherEvent(evt({ event: "window_closed", version: 13, label: "x" }))).toBe(true);
    });

    it("treats different labels at the same version as distinct keys", () => {
        expect(shouldDispatchLauncherEvent(evt({ event: "hwnd_drift_detected", version: 13, label: "a" }))).toBe(true);
        expect(shouldDispatchLauncherEvent(evt({ event: "hwnd_drift_detected", version: 13, label: "b" }))).toBe(true);
    });

    it("treats missing label/hwnd as a distinct key from present ones", () => {
        expect(shouldDispatchLauncherEvent(evt({ event: "host_should_quit", version: 5 }))).toBe(true);
        expect(shouldDispatchLauncherEvent(evt({ event: "host_should_quit", version: 5, label: "x" }))).toBe(true);
    });

    it("bounds memory under a flood of unique keys", () => {
        // 2000 distinct labels at v=1 — should evict oldest, keeping the cap.
        for (let i = 0; i < 2000; i++) {
            shouldDispatchLauncherEvent(evt({ event: "window_opened", version: 1, label: `w-${i}` }));
        }
        expect(launcherEventDedupStats().tracked).toBeLessThanOrEqual(1024);
    });

    it("after eviction, an evicted key's re-arrival is admitted regardless of version", () => {
        // Saturate the cache to evict the early entry.
        shouldDispatchLauncherEvent(evt({ event: "hwnd_drift_detected", version: 13, label: "evicted" }));
        for (let i = 0; i < 1100; i++) {
            shouldDispatchLauncherEvent(evt({ event: "window_opened", version: 1, label: `filler-${i}` }));
        }
        // "evicted" is gone. A re-arrival at the SAME version (13) is admitted
        // because the per-key version is unknown post-eviction. PerSourceTracker
        // behind this still enforces global version monotonicity, so a
        // genuinely-duplicate same-version event is dropped one layer down.
        expect(shouldDispatchLauncherEvent(evt({ event: "hwnd_drift_detected", version: 13, label: "evicted" }))).toBe(true);
    });

    it("clears the cache on launcher-restart sentinel (version=1 after prior versions)", () => {
        // Establish prior incarnation: events at v=12, v=13.
        shouldDispatchLauncherEvent(evt({ event: "window_opened", version: 12, label: "main" }));
        shouldDispatchLauncherEvent(evt({ event: "hwnd_drift_detected", version: 13, label: "main", hwnd: 99 }));
        expect(launcherEventDedupStats().tracked).toBe(2);

        // Launcher restarts: event_version resets to 1. Per the codex P1 finding,
        // without bridge-level reset the sentinel hits a cached "main" at v=12
        // and gets suppressed → PerSourceTracker never sees v=1 → its
        // restart-detection never fires → all post-restart events are stuck
        // behind the dead launcher's lastVersion=13. The bridge MUST clear
        // its cache and admit the sentinel.
        const sentinel = evt({ event: "window_opened", version: 1, label: "main" });
        expect(shouldDispatchLauncherEvent(sentinel)).toBe(true);
        expect(launcherEventDedupStats().tracked).toBe(1);

        // Subsequent low-version events from the new launcher should also flow.
        expect(shouldDispatchLauncherEvent(evt({ event: "window_opened", version: 2, label: "other" }))).toBe(true);
        expect(shouldDispatchLauncherEvent(evt({ event: "hwnd_drift_detected", version: 3, label: "main", hwnd: 99 }))).toBe(true);
    });

    it("passes malformed events through without throwing or polluting the cache", () => {
        // Codex P2 PR #708 round 2: the bridge dedup runs before
        // PerSourceTracker, so accessing evt.version/evt.event on a
        // null/non-object would throw out of __agentmux_launcher_event.
        // Guard returns true (let the tracker's canonical
        // log-and-discard handle it) and doesn't touch the cache.
        const malformed: unknown[] = [
            null,
            undefined,
            "string",
            42,
            {},
            { event: "ok-but-no-version" },
            { version: 5 }, // missing event
            { event: "ok", version: "not-a-number" },
        ];
        for (const m of malformed) {
            expect(shouldDispatchLauncherEvent(m as LauncherEvent)).toBe(true);
        }
        expect(launcherEventDedupStats()).toEqual({ tracked: 0, suppressed: 0 });
    });

    it("does NOT treat the very first v=1 event as a restart (cold start)", () => {
        // No prior versions → v=1 is just the first event of a fresh launcher,
        // not a restart sentinel. Cache should NOT be cleared (it's already empty)
        // and the event admits normally.
        const e = evt({ event: "window_opened", version: 1, label: "main" });
        expect(shouldDispatchLauncherEvent(e)).toBe(true);
        expect(launcherEventDedupStats().tracked).toBe(1);
    });
});

// Master spec §8.14 — subscriber idempotency contract. The bridge guard
// is the renderer-side enforcement point; this property test exercises
// it under randomised duplicate-arrival sequences. Uses a seeded Mulberry32
// PRNG so failures reproduce deterministically without adding fast-check
// as a dependency.
function mulberry32(seed: number): () => number {
    let t = seed >>> 0;
    return () => {
        t = (t + 0x6d2b79f5) >>> 0;
        let r = Math.imul(t ^ (t >>> 15), 1 | t);
        r = (r + Math.imul(r ^ (r >>> 7), 61 | r)) ^ r;
        return ((r ^ (r >>> 14)) >>> 0) / 4294967296;
    };
}

describe("launcher-event idempotency (§8.14 contract, property tests)", () => {
    beforeEach(() => __resetDedupForTests());

    // NOTE: these property tests use versions ≥2 to avoid the
    // launcher-restart sentinel (v=1 with prior cache → reset). The
    // monotonicity property holds *within a launcher incarnation*; the
    // restart path is exercised separately in the unit tests above.

    it("dispatch count for a (kind,label,hwnd) key never exceeds the count of distinct max-versions seen", () => {
        // Spec §8.14: subscribers must be idempotent under (kind,label,version).
        // Bridge guard's contract: for a given key, only events with strictly
        // higher version than any seen pass through. Random shuffle of
        // duplicates + arbitrary versions per key — admitted count == count
        // of strictly-increasing version watermarks.
        for (let seed = 1; seed <= 50; seed++) {
            __resetDedupForTests();
            const rng = mulberry32(seed);
            const keys = ["a", "b", "c"];
            const events: LauncherEvent[] = [];
            // Build 200 events: random key, random version 2..21, mostly
            // duplicates (low version range vs high event count).
            for (let i = 0; i < 200; i++) {
                events.push(evt({
                    event: "hwnd_drift_detected",
                    version: 2 + Math.floor(rng() * 20),
                    label: keys[Math.floor(rng() * keys.length)],
                    hwnd: 1,
                }));
            }
            // Per-key max version that ever passed through.
            const admittedMaxByKey = new Map<string, number>();
            for (const e of events) {
                const passed = shouldDispatchLauncherEvent(e);
                const k = `${(e as { label: string }).label}`;
                if (passed) {
                    const prev = admittedMaxByKey.get(k) ?? 0;
                    expect(e.version).toBeGreaterThan(prev);
                    admittedMaxByKey.set(k, e.version);
                }
            }
        }
    });

    it("is monotonic per key: once version V is admitted, no version <=V for that key admits", () => {
        for (let seed = 100; seed < 130; seed++) {
            __resetDedupForTests();
            const rng = mulberry32(seed);
            const admitted = new Map<string, number>();
            for (let i = 0; i < 100; i++) {
                const ev = evt({
                    event: "window_opened",
                    version: 2 + Math.floor(rng() * 30),
                    label: ["x", "y"][Math.floor(rng() * 2)],
                });
                const k = (ev as { label: string }).label;
                const wasAdmitted = shouldDispatchLauncherEvent(ev);
                if (wasAdmitted) {
                    const prev = admitted.get(k);
                    if (prev !== undefined) {
                        expect(ev.version).toBeGreaterThan(prev);
                    }
                    admitted.set(k, ev.version);
                }
            }
        }
    });

    it("after a launcher restart, the cache is cleared and post-restart sequence admits normally", () => {
        // Pre-restart: random sequence of 30 events at v=10..30.
        const rng = mulberry32(7);
        for (let i = 0; i < 30; i++) {
            shouldDispatchLauncherEvent(evt({
                event: "hwnd_drift_detected",
                version: 10 + Math.floor(rng() * 21),
                label: ["a", "b"][Math.floor(rng() * 2)],
                hwnd: 1,
            }));
        }
        const preRestartTracked = launcherEventDedupStats().tracked;
        expect(preRestartTracked).toBeGreaterThan(0);

        // Restart sentinel: v=1 with prior versions cached.
        const sentinel = evt({ event: "window_opened", version: 1, label: "a" });
        expect(shouldDispatchLauncherEvent(sentinel)).toBe(true);
        expect(launcherEventDedupStats().tracked).toBe(1);

        // Post-restart: v=2..N admit if increasing per key. The "a" key
        // already saw v=1 from the sentinel; subsequent v=2..N events
        // for "a" must strictly increase. "b" wasn't reset since the
        // sentinel was for "a"... wait — the sentinel CLEARS the entire
        // cache, so all keys reset. Track from scratch.
        const seenPostRestart = new Map<string, number>([["a", 1]]);
        for (let i = 0; i < 30; i++) {
            // Avoid v=1 in the post-restart sequence so we don't trip
            // the sentinel a second time.
            const ev = evt({
                event: "hwnd_drift_detected",
                version: 2 + Math.floor(rng() * 20),
                label: ["a", "b"][Math.floor(rng() * 2)],
                hwnd: 1,
            });
            const k = (ev as { label: string }).label;
            const passed = shouldDispatchLauncherEvent(ev);
            if (passed) {
                const prev = seenPostRestart.get(k);
                if (prev !== undefined) expect(ev.version).toBeGreaterThan(prev);
                seenPostRestart.set(k, ev.version);
            }
        }
    });
});
