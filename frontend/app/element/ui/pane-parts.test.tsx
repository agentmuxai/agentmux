// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Notice, EmptyState, SectionHeader and PaneBody. The look lives in SCSS;
 * what these pin is the markup the SCSS and the panes' tests key off, and
 * what assistive tech reads: roles, headings and the dismiss button.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { EmptyState } from "./EmptyState";
import { Notice } from "./Notice";
import { PaneBody, PaneScroll } from "./PaneBody";
import { SectionHeader } from "./SectionHeader";

afterEach(cleanup);

describe("Notice", () => {
    it("announces a danger notice as an alert and the other tones as a status", () => {
        const { container } = render(() => (
            <>
                <Notice tone="danger">It broke</Notice>
                <Notice tone="warning">Careful</Notice>
                <Notice>Plain</Notice>
            </>
        ));
        const boxes = [...container.querySelectorAll(".ui-notice")];
        expect(boxes.map((b) => b.getAttribute("role"))).toEqual(["alert", "status", "status"]);
        expect(boxes.map((b) => b.className)).toEqual([
            "ui-notice ui-notice--danger",
            "ui-notice ui-notice--warning",
            "ui-notice ui-notice--accent",
        ]);
    });

    it("shows a title above the text and an icon that is hidden from assistive tech", () => {
        const { container } = render(() => (
            <Notice tone="success" title="Saved" icon="check">
                Nothing else to do.
            </Notice>
        ));
        expect(container.querySelector(".ui-notice-title")?.textContent).toBe("Saved");
        expect(container.querySelector(".ui-notice-body")?.textContent).toBe("SavedNothing else to do.");
        expect(container.querySelector("i.fa-check")?.getAttribute("aria-hidden")).toBe("true");
    });

    it("has no actions row unless it has actions or a dismiss", () => {
        const { container } = render(() => <Notice>Just text</Notice>);
        expect(container.querySelector(".ui-notice-actions")).toBeNull();
    });

    it("calls onDismiss from a Dismiss button after its own actions", () => {
        const onDismiss = vi.fn();
        const { container } = render(() => (
            <Notice tone="warning" actions={<button type="button">Retry</button>} onDismiss={onDismiss}>
                Offline
            </Notice>
        ));
        const names = [...container.querySelectorAll(".ui-notice-actions button")].map((b) => b.textContent);
        expect(names).toEqual(["Retry", "Dismiss"]);
        fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
        expect(onDismiss).toHaveBeenCalledTimes(1);
    });
});

describe("EmptyState", () => {
    it("is an inline status by default and a fill one on request", () => {
        const { container } = render(() => (
            <>
                <EmptyState>Nothing yet</EmptyState>
                <EmptyState variant="fill">Nothing at all</EmptyState>
            </>
        ));
        const states = [...container.querySelectorAll(".ui-empty-state")];
        expect(states.map((s) => s.className)).toEqual([
            "ui-empty-state ui-empty-state--inline",
            "ui-empty-state ui-empty-state--fill",
        ]);
        expect(states.every((s) => s.getAttribute("role") === "status")).toBe(true);
    });

    it("shows a spinner and marks itself busy while loading, ignoring the icon", () => {
        const { container } = render(() => (
            <EmptyState loading icon="inbox">
                Loading…
            </EmptyState>
        ));
        const state = container.querySelector(".ui-empty-state")!;
        expect(state.getAttribute("aria-busy")).toBe("true");
        expect(state.querySelector("i.fa-spinner.fa-spin")).not.toBeNull();
        expect(state.querySelector("i.fa-inbox")).toBeNull();
    });

    it("is not busy when it is not loading", () => {
        const { container } = render(() => <EmptyState icon="inbox">Empty</EmptyState>);
        const state = container.querySelector(".ui-empty-state")!;
        expect(state.hasAttribute("aria-busy")).toBe(false);
        expect(state.querySelector("i.fa-inbox")).not.toBeNull();
    });

    it("renders only the parts it is given", () => {
        const { container } = render(() => <EmptyState title="No remotes">Add one.</EmptyState>);
        expect(container.querySelector(".ui-empty-state-title")?.textContent).toBe("No remotes");
        expect(container.querySelector(".ui-empty-state-text")?.textContent).toBe("Add one.");
        expect(container.querySelector(".ui-empty-state-actions")).toBeNull();
        expect(container.querySelector("i")).toBeNull();
    });

    it("shows actions after the text", () => {
        render(() => <EmptyState actions={<button type="button">Add server</button>}>No servers.</EmptyState>);
        expect(screen.getByRole("button", { name: "Add server" }).closest(".ui-empty-state-actions")).not.toBeNull();
    });
});

describe("SectionHeader", () => {
    it("is a title row: an h2, the count, then the controls", () => {
        const { container } = render(() => (
            <SectionHeader title="Remotes" count={3} class="mine">
                <button type="button">Add</button>
            </SectionHeader>
        ));
        const row = container.querySelector(".ui-card-section-header")!;
        expect(row.classList.contains("mine")).toBe(true);
        expect(screen.getByRole("heading", { level: 2, name: "Remotes" }).className).toBe("ui-card-section-title");
        expect([...row.children].map((c) => c.textContent)).toEqual(["Remotes", "3", "Add"]);
    });

    it("shows a count of zero but no count when it is not given", () => {
        const { container, unmount } = render(() => <SectionHeader title="A" count={0} />);
        expect(container.querySelector(".ui-card-section-count")?.textContent).toBe("0");
        unmount();
        const second = render(() => <SectionHeader title="B" />);
        expect(second.container.querySelector(".ui-card-section-count")).toBeNull();
    });

    it("is a small group label as an h3 by default and takes a level", () => {
        render(() => (
            <>
                <SectionHeader variant="group" title="Connected" />
                <SectionHeader variant="group" level={4} title="Your servers" class="extra" />
            </>
        ));
        expect(screen.getByRole("heading", { level: 3, name: "Connected" }).className).toBe("ui-card-group-heading");
        expect(screen.getByRole("heading", { level: 4, name: "Your servers" }).className).toBe(
            "ui-card-group-heading extra"
        );
        expect(document.querySelector(".ui-card-section-header")).toBeNull();
    });
});

describe("PaneBody", () => {
    it("fills the pane and adds only what it is asked to", () => {
        const { container } = render(() => (
            <>
                <PaneBody>plain</PaneBody>
                <PaneBody padded scroll density="compact" class="mine">
                    all
                </PaneBody>
            </>
        ));
        const bodies = [...container.querySelectorAll(".ui-pane-body")];
        expect(bodies.map((b) => b.className)).toEqual([
            "ui-pane-body",
            "ui-pane-body ui-pane-body--padded ui-pane-body--scroll ui-density-compact mine",
        ]);
    });

    it("keeps a scrolling region for the part between a header and the end", () => {
        const { container } = render(() => (
            <PaneBody>
                <header>Head</header>
                <PaneScroll class="list">rows</PaneScroll>
            </PaneBody>
        ));
        const scroll = container.querySelector(".ui-pane-body > .ui-pane-scroll")!;
        expect(scroll.className).toBe("ui-pane-scroll list");
        expect(scroll.textContent).toBe("rows");
    });
});
