// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { VaultViewModel } from "./vault-model";
import { VaultNarrowActions } from "./vault-narrow-actions";

afterEach(cleanup);

describe("the Vault's narrow-pane actions", () => {
    it("are the header's three buttons, doing what they do there", () => {
        const model = { openTransfer: vi.fn(), setMeta: vi.fn() } as unknown as VaultViewModel;
        render(() => <VaultNarrowActions model={model} />);

        fireEvent.click(screen.getByRole("button", { name: "Import into the Vault" }));
        expect(model.openTransfer).toHaveBeenLastCalledWith("import");
        fireEvent.click(screen.getByRole("button", { name: "Export the Vault" }));
        expect(model.openTransfer).toHaveBeenLastCalledWith("export");
        fireEvent.click(screen.getByRole("button", { name: "What the Vault may ask you" }));
        expect(model.setMeta).toHaveBeenLastCalledWith({ "vault:prompts-read": null });
    });
});
