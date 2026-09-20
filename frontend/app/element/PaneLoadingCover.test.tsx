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

    /**
     * Phase 3. The cover must STATE its coverage rather than inherit it from
     * whichever ancestor happens to be positioned — that inheritance is what
     * broke in §3.2, where adding `.agent-view-zoomed` (position: relative)
     * silently moved the containing block and left the Shell drawer uncovered.
     *
     * A block-level cover has a window the agent-level one never had: before
     * its pane content mounts there is no `.block` to resolve `inset: 0`
     * against at all, and `<Block>`'s parent differs by render route (the
     * keep-alive path wraps it in an absolutely-positioned slot, the plain
     * path does not). So "is there content under me?" is an input, not a guess.
     */
    describe("coverage mode", () => {
        it("overlays by default, for callers whose content is already mounted", () => {
            render(() => <PaneLoadingCover phase={() => "assembling" as PaneReadinessPhase} />);
            expect(overlays()[0].classList.contains("is-in-flow")).toBe(false);
        });

        it("sits in flow when told there is nothing yet to overlay", () => {
            render(() => (
                <PaneLoadingCover phase={() => "assembling" as PaneReadinessPhase} overlay={() => false} />
            ));
            expect(overlays()[0].classList.contains("is-in-flow")).toBe(true);
        });

        it("switches to overlay the moment content mounts, on the same element", () => {
            const [contentMounted, setContentMounted] = createSignal(false);
            render(() => (
                <PaneLoadingCover phase={() => "assembling" as PaneReadinessPhase} overlay={contentMounted} />
            ));
            const before = overlays()[0];
            expect(before.classList.contains("is-in-flow")).toBe(true);

            setContentMounted(true);

            // Same node, reclassed — NOT an unmount/remount, which would drop
            // the cover for a frame and expose the content it is hiding.
            expect(overlays().length).toBe(1);
            expect(overlays()[0]).toBe(before);
            expect(overlays()[0].classList.contains("is-in-flow")).toBe(false);
        });
    });
});
