// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The preview's states for an attachment with no id: still processing in the
 * composer, versus gone from the store (transcript).
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/platform/pane-overlay", () => ({ usePaneOverlay: () => {} }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { AttachmentsInfoCommand: vi.fn() } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/app-api", () => ({ getApi: () => ({ getAuthKey: () => "k" }) }));
vi.mock("@/util/endpoints", () => ({ getWebServerEndpoint: () => "http://x" }));

import { AttachmentLightbox } from "./AttachmentLightbox";

afterEach(() => cleanup());

const open = (item: { pending?: boolean; error?: string }) =>
    render(() => (
        <AttachmentLightbox
            items={[{ key: "k", number: 1, name: "big.pdf", ...item }]}
            index={0}
            onIndex={() => {}}
            onClose={() => {}}
        />
    ));

describe("AttachmentLightbox", () => {
    it("says a composer attachment is still processing, not gone", () => {
        open({ pending: true });
        expect(screen.getByText("Processing…")).toBeTruthy();
        expect(screen.queryByText(/no longer available/)).toBeNull();
    });

    it("says a transcript attachment without an id is gone", () => {
        open({});
        expect(screen.getByText("This file is no longer available.")).toBeTruthy();
    });

    it("shows a processing failure", () => {
        open({ pending: false, error: "Couldn't read it." });
        expect(screen.getByText("Couldn't read it.")).toBeTruthy();
    });
});
