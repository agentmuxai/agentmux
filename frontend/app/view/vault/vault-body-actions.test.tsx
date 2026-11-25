// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { VaultBodyActions } from "./vault-body-actions";
import type { VaultViewModel } from "./vault-model";

afterEach(cleanup);

const modelOf = (narrow: boolean) => ({ narrow: () => narrow, openTransfer: vi.fn(), setMeta: vi.fn() }) as unknown as VaultViewModel;

describe("the Vault's body actions", () => {
    it("are Import, Export and the note, doing what they did in the tab", () => {
        const model = modelOf(false);
        render(() => <VaultBodyActions model={model} />);
        fireEvent.click(screen.getByRole("button", { name: "Import into the Vault" }));
        expect(model.openTransfer).toHaveBeenLastCalledWith("import");
        fireEvent.click(screen.getByRole("button", { name: "Export the Vault" }));
        expect(model.openTransfer).toHaveBeenLastCalledWith("export");
        fireEvent.click(screen.getByRole("button", { name: "What the Vault may ask you" }));
        expect(model.setMeta).toHaveBeenLastCalledWith({ "vault:prompts-read": null });
    });

    it("name Import and Export in words in a wide pane, and by icon alone in a narrow one", () => {
        render(() => <VaultBodyActions model={modelOf(false)} />);
        expect(screen.getByRole("button", { name: "Import into the Vault" }).textContent).toContain("Import");
        expect(screen.getByRole("button", { name: "Export the Vault" }).textContent).toContain("Export");
        cleanup();
        render(() => <VaultBodyActions model={modelOf(true)} />);
        expect(screen.getByRole("button", { name: "Import into the Vault" }).textContent?.trim()).toBe("");
        expect(screen.getByRole("button", { name: "Export the Vault" }).textContent?.trim()).toBe("");
        expect(screen.getAllByRole("button")).toHaveLength(3);
    });
});
