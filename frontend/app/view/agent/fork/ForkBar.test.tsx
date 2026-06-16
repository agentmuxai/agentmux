// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for ForkBar — the bottom-of-pane fork switcher.
 * Spec: docs/specs/SPEC_AGENT_PANE_FORKS_AND_AUX_PINS_2026_06_15.md §7.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ForkBar } from "./ForkBar";
import type { ForkSetEntry } from "./fork-set";

const entry = (over: Partial<ForkSetEntry> & { definitionId: string }): ForkSetEntry => ({
    title: over.definitionId,
    isRoot: false,
    isActive: false,
    depth: 0,
    ...over,
});

afterEach(() => cleanup());

describe("ForkBar", () => {
    it("renders nothing for a single-conversation pane (root only)", () => {
        const { container } = render(() => (
            <ForkBar forks={[entry({ definitionId: "root", isRoot: true, isActive: true })]} onSwitch={() => {}} />
        ));
        expect(container.querySelector(".fork-bar")).toBeNull();
    });

    it("renders one row per fork once a second fork exists", () => {
        render(() => (
            <ForkBar
                forks={[
                    entry({ definitionId: "root", title: "main", isRoot: true, isActive: true }),
                    entry({ definitionId: "f1", title: "side-thread", depth: 1 }),
                ]}
                onSwitch={() => {}}
            />
        ));
        expect(screen.getByText("main")).toBeInTheDocument();
        expect(screen.getByText("side-thread")).toBeInTheDocument();
    });

    it("marks the active fork's row aria-selected", () => {
        const { container } = render(() => (
            <ForkBar
                forks={[
                    entry({ definitionId: "root", isRoot: true }),
                    entry({ definitionId: "f1", title: "active-one", isActive: true, depth: 1 }),
                ]}
                onSwitch={() => {}}
            />
        ));
        const tabs = [...container.querySelectorAll('[role="tab"]')];
        const active = tabs.find((t) => t.getAttribute("aria-selected") === "true");
        expect(active).toHaveTextContent("active-one");
    });

    it("fires onSwitch with the fork's definitionId when a row is clicked", async () => {
        const onSwitch = vi.fn();
        render(() => (
            <ForkBar
                forks={[
                    entry({ definitionId: "root", title: "main", isRoot: true, isActive: true }),
                    entry({ definitionId: "f1", title: "side", depth: 1 }),
                ]}
                onSwitch={onSwitch}
            />
        ));
        await userEvent.setup().click(screen.getByText("side"));
        expect(onSwitch).toHaveBeenCalledWith("f1");
    });

    it("offers a close action on non-root forks only", async () => {
        const onClose = vi.fn();
        render(() => (
            <ForkBar
                forks={[
                    entry({ definitionId: "root", title: "main", isRoot: true, isActive: true }),
                    entry({ definitionId: "f1", title: "side", depth: 1 }),
                ]}
                onSwitch={() => {}}
                onClose={onClose}
            />
        ));
        // Only one close button — for the non-root fork.
        const closeBtns = screen.getAllByRole("button", { name: /^Close / });
        expect(closeBtns).toHaveLength(1);
        await userEvent.setup().click(closeBtns[0]);
        expect(onClose).toHaveBeenCalledWith("f1");
    });

    it("fires onFork when the + affordance is clicked", async () => {
        const onFork = vi.fn();
        render(() => (
            <ForkBar
                forks={[
                    entry({ definitionId: "root", isRoot: true, isActive: true }),
                    entry({ definitionId: "f1", depth: 1 }),
                ]}
                onSwitch={() => {}}
                onFork={onFork}
            />
        ));
        await userEvent.setup().click(screen.getByRole("button", { name: "Fork this conversation" }));
        expect(onFork).toHaveBeenCalledTimes(1);
    });
});
