// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The awaitable in-pane confirm that replaces `window.confirm` (docs/specs/SPEC_PANE_MODALS_NOT_WINDOW_MODALS_2026_11_02.md row 11). */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";
import { requestCancel } from "@/app/view/memory-editor/editor-keys";
import { ModalLayer } from "./ModalLayer";
import { createConfirm } from "./create-confirm";

afterEach(cleanup);

function mount() {
    const confirm = createConfirm();
    const { container } = render(() => (
        <ModalLayer scope="pane">
            <confirm.Dialog />
            <button>content</button>
        </ModalLayer>
    ));
    return { confirm, container };
}

describe("createConfirm", () => {
    it("draws nothing until asked, then asks inside the pane's layer", () => {
        const { confirm, container } = mount();
        expect(screen.queryByText("Remove it?")).toBeNull();
        void confirm.ask({ title: "Remove it?", description: "Gone for good.", confirmLabel: "Remove" });
        const title = screen.getByText("Remove it?");
        expect(container.querySelector(".modal-layer-mount")?.contains(title)).toBe(true);
        expect(screen.getByText("Gone for good.")).toBeTruthy();
    });

    it("resolves true on the confirm button and closes", async () => {
        const { confirm } = mount();
        const answer = confirm.ask({ title: "Remove it?", confirmLabel: "Remove" });
        fireEvent.click(screen.getByText("Remove"));
        expect(await answer).toBe(true);
        expect(screen.queryByText("Remove it?")).toBeNull();
    });

    it("resolves false on Cancel and on Escape", async () => {
        const { confirm } = mount();
        const first = confirm.ask({ title: "Remove it?" });
        fireEvent.click(screen.getByText("Cancel"));
        expect(await first).toBe(false);
        const second = confirm.ask({ title: "Remove it?" });
        fireEvent.keyDown(document.querySelector(".modal-panel")!, { key: "Escape" });
        expect(await second).toBe(false);
        expect(screen.queryByText("Remove it?")).toBeNull();
    });

    it("a question asked while one is open gets that one's answer instead of stacking another", async () => {
        const { confirm } = mount();
        const a = confirm.ask({ title: "First?" });
        const b = confirm.ask({ title: "Second?" });
        expect(screen.queryByText("Second?")).toBeNull();
        expect(document.querySelectorAll(".modal-panel")).toHaveLength(1);
        fireEvent.click(screen.getByText("OK"));
        expect(await a).toBe(true);
        expect(await b).toBe(true);
    });
});

describe("requestCancel with an in-pane answer", () => {
    it("a clean draft cancels at once without asking", () => {
        let cancelled = 0;
        const asked: string[] = [];
        expect(requestCancel({ isDirty: () => false, onCancel: () => cancelled++, confirm: (m) => (asked.push(m), true) })).toBe(true);
        expect(cancelled).toBe(1);
        expect(asked).toEqual([]);
    });

    it("a dirty draft waits for the answer: yes cancels, no keeps", async () => {
        let cancelled = 0;
        const base = { isDirty: () => true, onCancel: () => cancelled++ };
        expect(await requestCancel({ ...base, confirm: async () => false })).toBe(false);
        expect(cancelled).toBe(0);
        expect(await requestCancel({ ...base, confirm: async () => true })).toBe(true);
        expect(cancelled).toBe(1);
    });
});
