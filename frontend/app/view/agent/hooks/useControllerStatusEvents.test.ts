// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { deriveTurnActive } from "./useControllerStatusEvents";

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
