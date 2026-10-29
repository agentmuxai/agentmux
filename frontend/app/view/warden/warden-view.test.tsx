// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Warden is the Jobs pane now: the Vault, History and Audit are panes of their own and the rest
 * moved to the Swarm and the status bar (pane-sections.ts).
 */
import { cleanup, render, screen } from "@solidjs/testing-library";
import { createRoot, createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const jobsFailing = vi.hoisted(() => ({ n: 2 }));
vi.mock("@/app/view/warden-jobs/warden-jobs-manager", () => ({
    WardenJobsManager: (props: { visible: boolean; onFailingCount?: (n: number) => void }) => {
        props.onFailingCount?.(jobsFailing.n);
        return <div data-testid="jobs-manager" data-visible={String(props.visible)} />;
    },
}));

const [blockMeta, setBlockMeta] = createSignal<Record<string, unknown>>({});
const [visibility, setVisibility] = createSignal<"active" | "dormant">("active");
const host = () => ({
    blockId: "test-block",
    meta: blockMeta as any,
    setMeta: vi.fn(() => Promise.resolve()),
    isFocused: () => false,
    visibility,
});

import { create as createWarden } from "./warden-instance";
import { WARDEN_SECTIONS } from "./warden-sections";

describe("the Warden (Jobs) pane", () => {
    afterEach(() => {
        cleanup();
        setBlockMeta({});
        setVisibility("active");
        jobsFailing.n = 2;
    });

    it("has Jobs as its only section", () => {
        expect(WARDEN_SECTIONS.items.map((s) => s.id)).toEqual(["jobs"]);
        expect(WARDEN_SECTIONS.default).toBe("jobs");
    });

    it("shows the Jobs manager, visible while the pane is active", () => {
        const instance = createRoot(() => createWarden(host()));
        const Component = instance.component;
        render(() => <Component ctx={host()} />);
        expect(screen.getByTestId("jobs-manager").dataset.visible).toBe("true");
        setVisibility("dormant");
        expect(screen.getByTestId("jobs-manager").dataset.visible).toBe("false");
    });

    it("names the pane Jobs, with how many jobs are failing", () => {
        const instance = createRoot(() => createWarden(host()));
        const Component = instance.component;
        render(() => <Component ctx={host()} />);
        expect(instance.liveTitle?.().text).toBe("Jobs · 2 failing");
    });

    it("names the pane plain Jobs when none is failing", () => {
        jobsFailing.n = 0;
        const instance = createRoot(() => createWarden(host()));
        const Component = instance.component;
        render(() => <Component ctx={host()} />);
        expect(instance.liveTitle?.().text).toBe("Jobs");
    });

    it("zooms the whole pane with term:zoom", () => {
        setBlockMeta({ "term:zoom": 1.5 });
        const instance = createRoot(() => createWarden(host()));
        const Component = instance.component;
        const { container } = render(() => <Component ctx={host()} />);
        expect((container.querySelector(".warden-view") as HTMLElement).style.zoom).toBe("1.5");
    });
});
