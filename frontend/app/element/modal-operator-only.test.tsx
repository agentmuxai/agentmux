// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A prompt addressed to the person marks itself `data-operator-only`, which agents' UI query
 * and click leave out and refuse (crates/cef/src/browser_api/scripts/query.js).
 */
import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/platform/pane-overlay", () => ({ usePaneOverlay: () => {} }));

import { ConfirmModal } from "./confirm-modal";
import { Modal } from "./modal";

afterEach(() => cleanup());

const roots = () => [...document.querySelectorAll(".modal-root")];

describe("operator-only prompts", () => {
    it("marks a Modal that asks for it, and no other", () => {
        render(() => (
            <>
                <Modal open={true} operatorOnly onClose={() => {}}>
                    <p>approve?</p>
                </Modal>
                <Modal open={true} onClose={() => {}}>
                    <p>just a dialog</p>
                </Modal>
            </>
        ));
        const [marked, plain] = roots();
        expect(marked.hasAttribute("data-operator-only")).toBe(true);
        expect(plain.hasAttribute("data-operator-only")).toBe(false);
        expect(marked.querySelector(".modal-panel")?.textContent).toContain("approve?");
    });

    it("passes the mark through a ConfirmModal, with its buttons inside the marked root", () => {
        render(() => (
            <ConfirmModal open={true} title="x wants to install y" operatorOnly onConfirm={() => {}} onCancel={() => {}} />
        ));
        const root = roots()[0];
        expect(root.hasAttribute("data-operator-only")).toBe(true);
        const buttons = [...root.querySelectorAll("button")].map((b) => b.closest("[data-operator-only]"));
        expect(buttons.length).toBeGreaterThan(1);
        expect(buttons.every((b) => b === root)).toBe(true);
    });

    it("leaves a plain ConfirmModal unmarked", () => {
        render(() => <ConfirmModal open={true} title="delete it?" onConfirm={() => {}} onCancel={() => {}} />);
        expect(roots()[0].hasAttribute("data-operator-only")).toBe(false);
    });
});
