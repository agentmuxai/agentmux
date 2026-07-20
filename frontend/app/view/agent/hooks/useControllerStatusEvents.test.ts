// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { deriveTurnActive, didTurnJustEnd } from "./useControllerStatusEvents";

// Guards the exact wire contract that a P0 review caught: BlockControllerRuntime
// Status.is_agent_pane and .turn_active are both serialized
// `#[serde(skip_serializing_if = "is_false")]`, so a `false` is OMITTED rather
// than sent. The turn-END event (the whole point of the demote path) therefore
// arrives with turn_active ABSENT — it must read as false, never be dropped.
describe("deriveTurnActive (controllerstatus wire-shape guard)", () => {
    it("agent pane, turn in flight → true", () => {
        expect(deriveTurnActive({ is_agent_pane: true, turn_active: true })).toBe(true);
    });

    it("agent pane, turn ENDED (turn_active omitted as false) → false, NOT null", () => {
        // This is the case the demote path depends on and the one the old
        // `typeof === "boolean"` guard silently dropped.
        expect(deriveTurnActive({ is_agent_pane: true })).toBe(false);
    });

    it("agent pane, explicit turn_active:false (defensive — some deserializers rehydrate the default) → false", () => {
        expect(deriveTurnActive({ is_agent_pane: true, turn_active: false })).toBe(false);
    });

    it("non-agent (shell/PTY) pane — both fields omitted → null (no signal, don't reconcile)", () => {
        expect(deriveTurnActive({})).toBe(null);
        expect(deriveTurnActive({ shellprocstatus: "running" })).toBe(null);
    });

    it("non-agent pane that explicitly serialized is_agent_pane:false → null", () => {
        expect(deriveTurnActive({ is_agent_pane: false, turn_active: false })).toBe(null);
    });

    it("missing / non-object data → null", () => {
        expect(deriveTurnActive(undefined)).toBe(null);
        expect(deriveTurnActive(null)).toBe(null);
        expect(deriveTurnActive("nope")).toBe(null);
    });
});

// Guards the trigger for the Haiku ambient-summary/next-prompt-suggestion
// hooks (docs/specs/REPORT_AMBIENT_SUMMARY_OVERTRIGGER_2026_07_20.md) — must
// fire on a genuine busy→idle edge only, never on other transitions or on a
// pane's first-ever turn_active reading.
describe("didTurnJustEnd (ambient-summary trigger edge)", () => {
    it("true -> false is a genuine turn-end", () => {
        expect(didTurnJustEnd(true, false)).toBe(true);
    });

    it("false -> true (turn starting) is not a turn-end", () => {
        expect(didTurnJustEnd(false, true)).toBe(false);
    });

    it("true -> true (still busy, e.g. a mid-turn tool-call round) is not a turn-end", () => {
        expect(didTurnJustEnd(true, true)).toBe(false);
    });

    it("false -> false (still idle) is not a turn-end", () => {
        expect(didTurnJustEnd(false, false)).toBe(false);
    });

    it("undefined -> false (first reading ever, pane opened onto an already-idle agent) is NOT a turn-end", () => {
        // No prior "busy" was ever observed this mount, so there's nothing
        // new to summarize — must not fire on every pane open/tab switch.
        expect(didTurnJustEnd(undefined, false)).toBe(false);
    });

    it("undefined -> true (first reading ever, agent already mid-turn) is not a turn-end", () => {
        expect(didTurnJustEnd(undefined, true)).toBe(false);
    });
});
