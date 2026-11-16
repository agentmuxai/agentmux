// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Which panes have their picker open: one at a time. */

import { createRoot } from "solid-js";
import { afterEach, describe, expect, it } from "vitest";
import { closeBackdropPicker, isBackdropPickerOpen, openBackdropPicker, releaseBackdropPicker } from "./picker-state";

afterEach(() => {
    for (const id of ["ps-a", "ps-b", "ps-c"]) releaseBackdropPicker(id);
});

describe("picker-state", () => {
    it("is closed until opened, for a block it has never seen", () => {
        expect(isBackdropPickerOpen("ps-a")()).toBe(false);
    });

    it("opens and closes one block's picker", () => {
        openBackdropPicker("ps-a");
        expect(isBackdropPickerOpen("ps-a")()).toBe(true);
        closeBackdropPicker("ps-a");
        expect(isBackdropPickerOpen("ps-a")()).toBe(false);
    });

    it("opening one pane's picker closes every other pane's", () => {
        openBackdropPicker("ps-a");
        openBackdropPicker("ps-b");
        expect(isBackdropPickerOpen("ps-a")()).toBe(false);
        expect(isBackdropPickerOpen("ps-b")()).toBe(true);
        openBackdropPicker("ps-c");
        expect([isBackdropPickerOpen("ps-a")(), isBackdropPickerOpen("ps-b")(), isBackdropPickerOpen("ps-c")()]).toEqual([false, false, true]);
    });

    it("tells a reader that was watching a pane when another's picker opens", () => {
        const seen: boolean[] = [];
        createRoot((dispose) => {
            const open = isBackdropPickerOpen("ps-a");
            openBackdropPicker("ps-a");
            seen.push(open());
            openBackdropPicker("ps-b");
            seen.push(open());
            dispose();
        });
        expect(seen).toEqual([true, false]);
    });

    it("forgets a released block", () => {
        openBackdropPicker("ps-a");
        releaseBackdropPicker("ps-a");
        expect(isBackdropPickerOpen("ps-a")()).toBe(false);
    });
});
