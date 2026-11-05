// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Deleting a bundle asks first, inside the pane (docs/specs/SPEC_PANE_MODALS_NOT_WINDOW_MODALS_2026_11_02.md
 * row 11). The Delete button lives in the detail view, which replaces the list when a bundle is selected, so
 * the dialog has to be mounted there: a dialog in the list would be gone when the question is asked.
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ModalLayer } from "@/app/element/ModalLayer";

const listMock = vi.fn();
const deleteMock = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListBundlesCommand: (...a: unknown[]) => listMock(...a),
        DeleteBundleCommand: (...a: unknown[]) => deleteMock(...a),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));

import { BundleManager } from "./bundle-manager";

const bundle = { id: "b1", name: "Review kit", description: "", created_at: 0, updated_at: 0 };

beforeEach(() => {
    listMock.mockReset().mockResolvedValue([bundle]);
    deleteMock.mockReset().mockResolvedValue(null);
});
afterEach(cleanup);

async function openDetail() {
    render(() => (
        <ModalLayer scope="pane">
            <BundleManager />
        </ModalLayer>
    ));
    fireEvent.click(await screen.findByText("Review kit"));
    return await screen.findByText("Delete");
}

describe("deleting a bundle", () => {
    it("asks in the pane first; Cancel deletes nothing and the question can be asked again", async () => {
        fireEvent.click(await openDetail());
        expect(await screen.findByText("Delete this bundle?")).toBeTruthy();
        fireEvent.click(screen.getByText("Cancel"));
        expect(deleteMock).not.toHaveBeenCalled();
        await waitFor(() => expect(screen.queryByText("Delete this bundle?")).toBeNull());
        fireEvent.click(screen.getAllByText("Delete")[0]);
        expect(await screen.findByText("Delete this bundle?")).toBeTruthy();
    });

    it("the confirm button deletes it", async () => {
        fireEvent.click(await openDetail());
        await screen.findByText("Delete this bundle?");
        fireEvent.click(screen.getAllByText("Delete").at(-1)!);
        await waitFor(() => expect(deleteMock).toHaveBeenCalledTimes(1));
        expect(deleteMock.mock.calls[0][1]).toEqual({ id: "b1" });
    });
});
