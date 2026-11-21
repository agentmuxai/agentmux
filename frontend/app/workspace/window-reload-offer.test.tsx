// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const reloadWindow = vi.fn();
vi.mock("@/store/global", () => ({ getApi: () => ({ reloadWindow }) }));

import { chosenReloadOffer, WindowReloadOffer, type ReloadOfferReason } from "./window-reload-offer";

afterEach(() => {
    cleanup();
    reloadWindow.mockClear();
});

describe("chosenReloadOffer", () => {
    it("is the tab that is wrong before the panes that are missing before the errors", () => {
        expect(chosenReloadOffer({ tabs: false, panes: false, errors: false })).toBeNull();
        expect(chosenReloadOffer({ tabs: false, panes: false, errors: true })).toBe("errors");
        expect(chosenReloadOffer({ tabs: false, panes: true, errors: true })).toBe("panes");
        expect(chosenReloadOffer({ tabs: true, panes: true, errors: true })).toBe("tabs");
    });
});

describe("WindowReloadOffer", () => {
    it("shows nothing without a reason, and says what is wrong for each", () => {
        const [reason, setReason] = createSignal<ReloadOfferReason | null>(null);
        render(() => <WindowReloadOffer reason={reason} onDismiss={() => {}} />);
        expect(screen.queryByRole("alert")).toBeNull();
        setReason("tabs");
        expect(screen.getByRole("alert").textContent).toContain("This window stopped switching tabs.");
        setReason("panes");
        expect(screen.getByRole("alert").textContent).toContain("Some panes of this tab are not showing.");
        setReason("errors");
        expect(screen.getByRole("alert").textContent).toContain("repeated errors");
        setReason(null);
        expect(screen.queryByRole("alert")).toBeNull();
    });

    it("reloads the window only when the person presses Reload", () => {
        render(() => <WindowReloadOffer reason={() => "panes"} onDismiss={() => {}} />);
        expect(reloadWindow).not.toHaveBeenCalled();
        fireEvent.click(screen.getByRole("button", { name: "Reload" }));
        expect(reloadWindow).toHaveBeenCalledTimes(1);
    });

    it("offers Dismiss only for the errors, which do not take themselves back", () => {
        const onDismiss = vi.fn();
        const [reason, setReason] = createSignal<ReloadOfferReason | null>("tabs");
        render(() => <WindowReloadOffer reason={reason} onDismiss={onDismiss} />);
        expect(screen.queryByRole("button", { name: "Dismiss" })).toBeNull();
        setReason("panes");
        expect(screen.queryByRole("button", { name: "Dismiss" })).toBeNull();
        setReason("errors");
        fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
        expect(onDismiss).toHaveBeenCalledTimes(1);
    });
});
