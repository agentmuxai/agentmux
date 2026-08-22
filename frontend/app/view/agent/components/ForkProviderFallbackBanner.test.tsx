// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Banner-visibility tests for ForkProviderFallbackBanner — reagent's review
 * of PR #2735 (this component's own introducing PR) noted every new test
 * there covered quick-fork.ts's meta-flag logic, not the banner's own
 * render/Show behavior. Mirrors AgentDisconnectedBanner.test.tsx, the
 * component this one was cloned from.
 *
 * `@/app/tab/quick-fork` is mocked to just the one string constant this
 * component actually needs — the real module pulls in RpcApi/WOS/layout/
 * tab-presets, none of which a pure render test should have to satisfy.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/tab/quick-fork", () => ({
    FORK_NO_HISTORY_FALLBACK_META_KEY: "quickfork:noHistoryFallback",
}));

import { ForkProviderFallbackBanner } from "./ForkProviderFallbackBanner";

afterEach(() => {
    cleanup();
});

describe("ForkProviderFallbackBanner", () => {
    it("renders nothing when the meta flag is absent", () => {
        const [meta] = createSignal<MetaType | undefined>({ view: "agent" });
        render(() => <ForkProviderFallbackBanner meta={meta} />);
        expect(screen.queryByRole("status")).toBeNull();
    });

    it("renders nothing when meta itself is undefined", () => {
        const [meta] = createSignal<MetaType | undefined>(undefined);
        render(() => <ForkProviderFallbackBanner meta={meta} />);
        expect(screen.queryByRole("status")).toBeNull();
    });

    it("renders the note when the fallback meta flag is true", () => {
        const [meta] = createSignal<MetaType | undefined>({
            view: "agent",
            "quickfork:noHistoryFallback": true,
        });
        render(() => <ForkProviderFallbackBanner meta={meta} />);
        expect(screen.getByRole("status")).toBeInTheDocument();
        expect(
            screen.getByText(/doesn't support forking mid-conversation/),
        ).toBeInTheDocument();
    });

    it("toggles visibility when the meta signal flips", () => {
        const [meta, setMeta] = createSignal<MetaType | undefined>({ view: "agent" });
        render(() => <ForkProviderFallbackBanner meta={meta} />);
        expect(screen.queryByRole("status")).toBeNull();

        setMeta({ view: "agent", "quickfork:noHistoryFallback": true });
        expect(screen.getByRole("status")).toBeInTheDocument();

        setMeta({ view: "agent", "quickfork:noHistoryFallback": false });
        expect(screen.queryByRole("status")).toBeNull();
    });
});
