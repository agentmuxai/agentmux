// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Driving bar shows an agent's desktop grants for its own pane, and Stop goes through the host. */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const stop = vi.fn();
const [grants, setGrants] = createSignal<unknown[]>([]);
vi.mock("@/app/block/desktop-requests", () => ({ desktopGrants: () => grants() }));
vi.mock("@/app/store/app-api", () => ({ getApi: () => ({ approvals: { stopDesktop: (...a: unknown[]) => stop(...a) } }) }));

import { DesktopDrivingBar } from "./DesktopDrivingBar";

afterEach(() => cleanup());

const grant = (over: object) => ({ agent: "QTest", app: "notepad", block_id: "b1", until_ms: 1, active: false, ...over });

describe("DesktopDrivingBar", () => {
    it("shows nothing when this pane has no grant", () => {
        setGrants([grant({ block_id: "other" })]);
        const { container } = render(() => <DesktopDrivingBar blockId="b1" />);
        expect(container.querySelector(".agent-desktop-bar")).toBeNull();
    });

    it("says Driving while an action is under way, and May drive otherwise", () => {
        setGrants([grant({ active: true })]);
        render(() => <DesktopDrivingBar blockId="b1" />);
        expect(screen.getByText(/Driving/)).toBeTruthy();
        setGrants([grant({ active: false })]);
        expect(screen.getByText(/May drive/)).toBeTruthy();
    });

    it("Stop is relayed through the host for this pane", async () => {
        setGrants([grant({})]);
        stop.mockResolvedValue(undefined);
        render(() => <DesktopDrivingBar blockId="b1" />);
        fireEvent.click(screen.getByRole("button", { name: "Stop" }));
        await Promise.resolve();
        expect(stop).toHaveBeenCalledWith("b1");
    });
});
