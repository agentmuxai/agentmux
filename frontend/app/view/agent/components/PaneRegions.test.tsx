// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for PaneRegions — the declarative agent-pane region container.
 * Spec: docs/specs/SPEC_AGENT_PANE_FORKS_AND_AUX_PINS_2026_06_15.md §5.1.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import { createSignal, type JSX } from "solid-js";
import { afterEach, describe, expect, it } from "vitest";

import { PaneRegions, PANE_REGION_ORDER, type PaneRegionName } from "./PaneRegions";

afterEach(() => cleanup());

describe("PaneRegions", () => {
    it("wraps each provided region with its modifier class + data-region", () => {
        const { container } = render(() => (
            <PaneRegions regions={{ "top-fixed": <span>banner</span>, stream: <span>convo</span> }} />
        ));
        const top = container.querySelector('[data-region="top-fixed"]');
        const stream = container.querySelector('[data-region="stream"]');
        expect(top).toHaveClass("pane-region", "pane-region--top-fixed");
        expect(stream).toHaveClass("pane-region", "pane-region--stream");
        expect(top).toHaveTextContent("banner");
        expect(stream).toHaveTextContent("convo");
    });

    it("renders regions in the canonical top→bottom order", () => {
        const { container } = render(() => (
            <PaneRegions
                regions={{
                    // Provide them out of order; the component must reorder.
                    forks: <span>forks</span>,
                    "top-fixed": <span>top</span>,
                    input: <span>input</span>,
                    stream: <span>stream</span>,
                }}
            />
        ));
        const order = [...container.querySelectorAll("[data-region]")].map(
            (el) => el.getAttribute("data-region"),
        );
        expect(order).toEqual(["top-fixed", "stream", "input", "forks"]);
        // And they follow the canonical sequence.
        const canonicalIdx = (r: string) => PANE_REGION_ORDER.indexOf(r as PaneRegionName);
        expect(order.map(canonicalIdx)).toEqual([...order.map(canonicalIdx)].sort((a, b) => a - b));
    });

    it("renders no wrapper for an absent region", () => {
        const { container } = render(() => (
            <PaneRegions regions={{ stream: <span>only</span> }} />
        ));
        expect(container.querySelector('[data-region="dock"]')).toBeNull();
        expect(container.querySelectorAll("[data-region]")).toHaveLength(1);
    });

    it("treats an empty array (and all-nullish content) as no content", () => {
        const { container } = render(() => (
            <PaneRegions regions={{ dock: [] as JSX.Element[], alert: [null, false] as unknown as JSX.Element[], stream: <span>x</span> }} />
        ));
        expect(container.querySelector('[data-region="dock"]')).toBeNull();
        expect(container.querySelector('[data-region="alert"]')).toBeNull();
        expect(container.querySelector('[data-region="stream"]')).not.toBeNull();
    });

    it("renders an array of nodes within one region", () => {
        render(() => (
            <PaneRegions regions={{ forks: [<span>a</span>, <span>b</span>] }} />
        ));
        expect(screen.getByText("a")).toBeInTheDocument();
        expect(screen.getByText("b")).toBeInTheDocument();
    });

    it("reactively shows/hides a region as its content toggles", () => {
        const [show, setShow] = createSignal(false);
        const { container } = render(() => (
            <PaneRegions regions={{ stream: <span>s</span>, forks: show() ? <span>bar</span> : undefined }} />
        ));
        expect(container.querySelector('[data-region="forks"]')).toBeNull();
        setShow(true);
        expect(container.querySelector('[data-region="forks"]')).not.toBeNull();
        setShow(false);
        expect(container.querySelector('[data-region="forks"]')).toBeNull();
    });
});
