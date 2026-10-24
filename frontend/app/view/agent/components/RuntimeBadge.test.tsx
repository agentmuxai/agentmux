// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The runtime badge says Native and Container: the stored values stay "host" and "container", and
 * the person never reads "Host" or "Sandbox" for an agent's runtime.
 */
import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";
import { RuntimeBadge } from "./RuntimeBadge";

afterEach(cleanup);

describe("RuntimeBadge", () => {
    it("shows Native for a host runtime and Container for a container one, in the card and pane sizes", () => {
        render(() => (
            <>
                <RuntimeBadge runtime="host" />
                <RuntimeBadge runtime="container" size="md" />
            </>
        ));
        expect(screen.getByText("Native")).toBeTruthy();
        expect(screen.getByText("Container")).toBeTruthy();
        expect(screen.queryByText("Host")).toBeNull();
    });

    it("shows NATIVE and CONTAINER in the compact tag, never HOST or SANDBOX", () => {
        render(() => (
            <>
                <RuntimeBadge runtime="host" size="tag" />
                <RuntimeBadge runtime="container" size="tag" />
            </>
        ));
        expect(screen.getByText("NATIVE")).toBeTruthy();
        expect(screen.getByText("CONTAINER")).toBeTruthy();
        expect(screen.queryByText("HOST")).toBeNull();
        expect(screen.queryByText("SANDBOX")).toBeNull();
    });

    it("explains each in its tooltip, with the two terms side by side", () => {
        const { container } = render(() => (
            <>
                <RuntimeBadge runtime="host" />
                <RuntimeBadge runtime="container" />
            </>
        ));
        const titles = Array.from(container.querySelectorAll(".runtime-badge")).map((e) => e.getAttribute("title"));
        expect(titles).toEqual(["Native (runs on this computer, with full access)", "Container (runs in Docker, isolated)"]);
    });

    it("shows an unknown runtime as it is stored", () => {
        render(() => <RuntimeBadge runtime="standalone" />);
        expect(screen.getByText("standalone")).toBeTruthy();
    });
});
