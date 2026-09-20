// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PaneLoadingCover — phase 2 of SPEC_PANE_LOADING_CONSOLIDATION_2026_09_20.md.
 *
 * The property worth pinning is not the markup, it is the CONSOLIDATION: exactly
 * one element carrying `.agent-pane-loading-overlay` per cover, driven by a single
 * phase, so "covered", "fading" and "gone" cannot disagree. Before this, the class
 * was rendered by both agent-view.tsx and AgentPicker.tsx with independent ready
 * signals and unmount timers, and two BrainSpinners were measured on screen at once.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PaneLoadingCover } from "./PaneLoadingCover";
import type { PaneReadinessPhase } from "@/app/store/pane-readiness";

vi.mock("@/app/store/global", () => ({
    atoms: { prefersReducedMotionAtom: () => false },
}));

afterEach(cleanup);

const overlays = () => document.querySelectorAll(".agent-pane-loading-overlay");

describe("PaneLoadingCover", () => {
    it("covers opaquely while assembling", () => {
        render(() => <PaneLoadingCover phase={() => "assembling" as PaneReadinessPhase} />);
        expect(overlays().length).toBe(1);
        expect(overlays()[0].classList.contains("is-fading")).toBe(false);
    });

    it("keeps the cover mounted but fading while revealing", () => {
        render(() => <PaneLoadingCover phase={() => "revealing" as PaneReadinessPhase} />);
        expect(overlays().length).toBe(1);
        expect(overlays()[0].classList.contains("is-fading")).toBe(true);
    });

    it("unmounts once live", () => {
        render(() => <PaneLoadingCover phase={() => "live" as PaneReadinessPhase} />);
        expect(overlays().length).toBe(0);
    });

    it("transitions through the phases without ever showing two covers", () => {
        const [phase, setPhase] = createSignal<PaneReadinessPhase>("assembling");
        render(() => <PaneLoadingCover phase={phase} />);

        expect(overlays().length).toBe(1);
        setPhase("revealing");
        expect(overlays().length).toBe(1);
        expect(overlays()[0].classList.contains("is-fading")).toBe(true);
        setPhase("live");
        expect(overlays().length).toBe(0);
    });

    /**
     * The regression this consolidation exists to prevent: two components each
     * rendering the class for the same pane. Mounting two covers driven by the
     * SAME phase must still be the caller's bug, not something the component
     * papers over — but the realistic case (one cover per pane) must never
     * produce a second element on its own.
     */
    it("renders exactly one overlay element per cover instance", () => {
        const [phase] = createSignal<PaneReadinessPhase>("assembling");
        render(() => <PaneLoadingCover phase={phase} />);
        expect(overlays().length).toBe(1);
    });
});
