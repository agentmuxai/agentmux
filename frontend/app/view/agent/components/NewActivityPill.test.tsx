// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { NewActivityPill } from "./NewActivityPill";

afterEach(cleanup);

describe("NewActivityPill", () => {
    it("shows only while there is new activity, and clicking it asks to follow the bottom", () => {
        const [visible, setVisible] = createSignal(false);
        const onJump = vi.fn();
        const { container } = render(() => <NewActivityPill visible={visible} onJump={onJump} />);
        expect(container.querySelector(".agent-new-activity-pill")).toBeNull();
        setVisible(true);
        const pill = container.querySelector(".agent-new-activity-pill")!;
        expect(pill.textContent).toContain("New activity");
        fireEvent.click(pill);
        expect(onJump).toHaveBeenCalledTimes(1);
        setVisible(false);
        expect(container.querySelector(".agent-new-activity-pill")).toBeNull();
    });
});
