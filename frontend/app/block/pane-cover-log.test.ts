// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot, createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { PaneReadinessPhase } from "@/app/store/pane-readiness";
import { coverLogLine, logCoverPhases } from "./pane-cover-log";

afterEach(() => vi.restoreAllMocks());

describe("coverLogLine", () => {
    it("names the block, the change, the source and the time since mount", () => {
        expect(coverLogLine("0123456789", "assembling", "revealing", "controller", 1102.6)).toBe(
            "block=0123456 assembling→revealing src=controller t=1103ms"
        );
    });

    it("says mount for the first phase", () => {
        expect(coverLogLine("abcdefgh", undefined, "assembling", "controller", 0)).toBe(
            "block=abcdefg mount→assembling src=controller t=0ms"
        );
    });
});

describe("logCoverPhases", () => {
    it("writes one [pane-cover] line per change of phase, with the source at that moment", () => {
        const info = vi.spyOn(console, "info").mockImplementation(() => {});
        const [phase, setPhase] = createSignal<PaneReadinessPhase>("assembling");
        const [source, setSource] = createSignal<"controller" | "re-cover">("controller");
        const dispose = createRoot((d) => {
            logCoverPhases("0123456789", phase, source);
            return d;
        });
        setPhase("revealing");
        setPhase("live");
        setSource("re-cover");
        setPhase("assembling"); // a subagent "started" re-covers the pane
        dispose();
        const lines = info.mock.calls.filter((c) => c[0] === "[pane-cover]").map((c) => String(c[1]).replace(/ t=\d+ms$/, ""));
        expect(lines).toEqual([
            "block=0123456 mount→assembling src=controller",
            "block=0123456 assembling→revealing src=controller",
            "block=0123456 revealing→live src=controller",
            "block=0123456 live→assembling src=re-cover",
        ]);
    });
});
