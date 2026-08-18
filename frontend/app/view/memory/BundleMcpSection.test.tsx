// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * reagentx P1 on PR #2647: a failed "add private server" must not silently
 * discard the name/config the user just typed — `addPrivate` never rejects
 * (errors go to errorAtom, same convention as bind/unbind), so a naive
 * `.then(() => clearForm())` ran unconditionally regardless of success.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

vi.mock("@/app/store/wps", () => ({ waveEventSubscribe: vi.fn(() => () => {}) }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        McpCatalogListForBundleCommand: vi.fn(),
        McpCatalogBindToBundleCommand: vi.fn(),
        McpCatalogUnbindFromBundleCommand: vi.fn(),
        McpCatalogUpsertForBundleCommand: vi.fn(),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { RpcApi } from "@/app/store/rpc-api";
import { BundleMcpSection } from "./BundleMcpSection";

afterEach(() => cleanup());

beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(RpcApi.McpCatalogListForBundleCommand).mockResolvedValue([]);
});

function getInputs() {
    return {
        name: screen.getByPlaceholderText("Server name") as HTMLInputElement,
        config: screen.getByPlaceholderText(/my-tool/) as HTMLTextAreaElement,
        submit: screen.getByRole("button", { name: /Add private server/ }) as HTMLButtonElement,
    };
}

describe("BundleMcpSection — add-private form", () => {
    test("a failed add does NOT clear the typed name/config", async () => {
        vi.mocked(RpcApi.McpCatalogUpsertForBundleCommand).mockRejectedValue(
            new Error("server name 'X' already bound to this bundle"),
        );
        render(() => <BundleMcpSection bundleId="bundle-1" />);
        const user = userEvent.setup();
        const { name, config, submit } = getInputs();

        await user.type(name, "My Tool");
        await user.clear(config);
        await user.click(config);
        await user.paste('{"command":"my-tool"}');
        await user.click(submit);

        await screen.findByText(/Add failed/);
        expect(name.value).toBe("My Tool");
        expect(config.value).toBe('{"command":"my-tool"}');
    });

    test("a successful add DOES clear the form", async () => {
        vi.mocked(RpcApi.McpCatalogUpsertForBundleCommand).mockResolvedValue({
            id: "new-1", name: "My Tool", transport: "stdio", config: "{}",
            is_global: false, created_at: 0, updated_at: 0,
        } as any);
        render(() => <BundleMcpSection bundleId="bundle-1" />);
        const user = userEvent.setup();
        const { name, config, submit } = getInputs();

        await user.type(name, "My Tool");
        await user.click(submit);

        await vi.waitFor(() => expect(name.value).toBe(""));
        expect(config.value).toBe("{}");
    });
});
