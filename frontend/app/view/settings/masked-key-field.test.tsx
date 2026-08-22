// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for MaskedKeyField (settings-controls.tsx) — the masked-credential
 * primitive introduced for Settings -> Recording's Groq API key field
 * (docs/specs/SPEC_SETTINGS_RECORDING_INPUT_SECTION_2026_08_19.md §2), also
 * intended for future messaging-bridge bot tokens. No prior test coverage
 * existed for any settings-controls.tsx primitive.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import { MaskedKeyField } from "./settings-controls";

afterEach(() => cleanup());

describe("MaskedKeyField", () => {
    it("shows a dot mask + Replace button when a value is already set", () => {
        const { container } = render(() => <MaskedKeyField value="sk-existing-key" onSave={() => {}} />);
        expect(screen.getByText("••••••••")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Replace" })).toBeInTheDocument();
        expect(container.querySelector("input")).not.toBeInTheDocument();
    });

    it("shows an entry field with no value set (nothing to mask yet)", () => {
        render(() => <MaskedKeyField value={undefined} onSave={() => {}} placeholder="paste key" />);
        expect(screen.queryByText("••••••••")).not.toBeInTheDocument();
        expect(screen.getByPlaceholderText("paste key")).toBeInTheDocument();
        // No existing value to fall back to, so Cancel shouldn't be offered.
        expect(screen.queryByRole("button", { name: "Cancel" })).not.toBeInTheDocument();
    });

    it("clicking Replace reveals the entry field with Save + Cancel", () => {
        render(() => <MaskedKeyField value="sk-existing-key" onSave={() => {}} />);
        fireEvent.click(screen.getByRole("button", { name: "Replace" }));
        expect(screen.getByRole("button", { name: "Save" })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
    });

    it("Cancel returns to the locked/masked state without calling onSave", () => {
        const onSave = vi.fn();
        render(() => <MaskedKeyField value="sk-existing-key" onSave={onSave} />);
        fireEvent.click(screen.getByRole("button", { name: "Replace" }));
        fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
        expect(screen.getByText("••••••••")).toBeInTheDocument();
        expect(onSave).not.toHaveBeenCalled();
    });

    it("Save calls onSave with the typed value and clears the draft", () => {
        const onSave = vi.fn();
        render(() => <MaskedKeyField value="sk-existing-key" onSave={onSave} />);
        fireEvent.click(screen.getByRole("button", { name: "Replace" }));
        const input = screen.getByDisplayValue("") as HTMLInputElement;
        fireEvent.input(input, { target: { value: "sk-new-key" } });
        fireEvent.click(screen.getByRole("button", { name: "Save" }));
        expect(onSave).toHaveBeenCalledWith("sk-new-key");
    });

    it("Save is disabled until something is typed", () => {
        const { container } = render(() => <MaskedKeyField value={undefined} onSave={() => {}} />);
        expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
        fireEvent.input(container.querySelector("input") as HTMLInputElement, { target: { value: "x" } });
        expect(screen.getByRole("button", { name: "Save" })).not.toBeDisabled();
    });
});
