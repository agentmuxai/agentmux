// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** A bundle's read-only view: its header, Edit and Delete, and the Instructions card that opens the edit form. */

import { cleanup, fireEvent, render, screen, within } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));
vi.mock("./BundleSkillsSection", () => ({
    BundleSkillsSection: (p: { bundleId: string }) => <div data-testid="skills">skills for {p.bundleId}</div>,
}));

import type { Bundle } from "@/app/store/rpc-api";
import type { BundleActions } from "./bundle-manager-actions";
import { BundleReadOnly } from "./bundle-manager-readonly";
import type { BundleViewModel } from "./bundle-model";

afterEach(cleanup);

function bundle(over: Partial<Bundle> = {}): Bundle {
    return {
        id: "b1",
        name: "Review kit",
        description: "Reviews code",
        is_blank: false,
        is_global: false,
        instructions: "Review it\nthen say so",
        instructions_by_provider: "{}",
        ...over,
    } as Bundle;
}

function mount(b: Bundle) {
    const model = { startEdit: vi.fn() } as unknown as BundleViewModel & { startEdit: ReturnType<typeof vi.fn> };
    const act = { handleDelete: vi.fn() } as unknown as BundleActions & { handleDelete: ReturnType<typeof vi.fn> };
    render(() => <BundleReadOnly model={model} act={act} memory={() => b} />);
    return { model, act };
}

const instructionsCard = () => document.querySelector(".bundle-instructions-card") as HTMLElement;
const meta = (el: HTMLElement) => [...el.querySelectorAll(".ui-card-meta > *")].map((e) => e.textContent);

describe("a bundle's read-only view", () => {
    it("names the bundle and says what it is for", () => {
        mount(bundle());
        expect(screen.getByRole("heading", { level: 2, name: "Review kit" })).toBeTruthy();
        expect(document.querySelector(".bundle-view-description")?.textContent).toBe("Reviews code");
    });

    it("has no description line when there is none", () => {
        mount(bundle({ description: "" }));
        expect(document.querySelector(".bundle-view-description")).toBeNull();
    });

    it("says a global bundle goes to every agent, and says nothing for another", () => {
        mount(bundle({ is_global: true }));
        expect(document.querySelector(".bundle-view-scope")?.textContent).toContain(
            "injected into every agent at launch"
        );
        cleanup();
        mount(bundle());
        expect(document.querySelector(".bundle-view-scope")).toBeNull();
    });

    it("edits and deletes the bundle shown", () => {
        const { model, act } = mount(bundle());
        fireEvent.click(screen.getByRole("button", { name: "Edit" }));
        expect(model.startEdit).toHaveBeenCalledWith(expect.objectContaining({ id: "b1" }));
        fireEvent.click(screen.getByRole("button", { name: "Delete" }));
        expect(act.handleDelete).toHaveBeenCalledWith("b1");
    });

    it("shows the Skills of a bundle for its id, under the Instructions", () => {
        mount(bundle());
        expect(screen.getByTestId("skills").textContent).toBe("skills for b1");
        const headings = screen.getAllByRole("heading", { level: 3 }).map((h) => h.textContent);
        expect(headings).toEqual(["Instructions"]);
    });
});

describe("the Instructions card", () => {
    it("shows the first line of the instructions and how long they are", () => {
        mount(bundle());
        expect(within(instructionsCard()).getByText("Instructions")).toBeTruthy();
        expect(instructionsCard().querySelector(".ui-card-subtitle")?.textContent).toBe("Review it");
        expect(meta(instructionsCard())).toEqual(["21 characters"]);
    });

    it("counts the characters of the trimmed instructions", () => {
        mount(bundle({ instructions: "  abc \n" }));
        expect(meta(instructionsCard())).toEqual(["3 characters"]);
    });

    it("says there are none when the instructions are empty or missing", () => {
        mount(bundle({ instructions: "   " }));
        expect(instructionsCard().querySelector(".ui-card-subtitle")?.textContent).toBe("(none)");
        cleanup();
        mount(bundle({ instructions: undefined as unknown as string }));
        expect(instructionsCard().querySelector(".ui-card-subtitle")?.textContent).toBe("(none)");
        expect(meta(instructionsCard())).toEqual(["0 characters"]);
    });

    it("counts the provider variants", () => {
        mount(bundle({ instructions_by_provider: '{"codex":"a","gemini":"b"}' }));
        expect(meta(instructionsCard())).toEqual(["21 characters", "2 provider variants"]);
        cleanup();
        mount(bundle({ instructions_by_provider: '{"codex":"a"}' }));
        expect(meta(instructionsCard())).toEqual(["21 characters", "1 provider variant"]);
    });

    it("counts none for variants that are empty, missing or not JSON", () => {
        for (const v of ["{}", "", "not json"]) {
            mount(bundle({ instructions_by_provider: v }));
            expect(meta(instructionsCard())).toEqual(["21 characters"]);
            cleanup();
        }
    });

    it("opens the edit form for the bundle on a click", () => {
        const { model } = mount(bundle());
        fireEvent.click(instructionsCard());
        expect(model.startEdit).toHaveBeenCalledTimes(1);
        expect(model.startEdit).toHaveBeenCalledWith(expect.objectContaining({ id: "b1" }));
    });
});

describe("the blank bundle", () => {
    it("has no Edit, Delete or Skills, and its Instructions card opens nothing", () => {
        const { model } = mount(bundle({ is_blank: true, name: "Blank", instructions: "" }));
        expect(screen.queryByRole("button", { name: "Edit" })).toBeNull();
        expect(screen.queryByRole("button", { name: "Delete" })).toBeNull();
        expect(screen.queryByTestId("skills")).toBeNull();
        fireEvent.click(instructionsCard());
        expect(model.startEdit).not.toHaveBeenCalled();
        expect(instructionsCard().querySelector(".ui-card-subtitle")?.textContent).toBe("(none)");
    });
});
