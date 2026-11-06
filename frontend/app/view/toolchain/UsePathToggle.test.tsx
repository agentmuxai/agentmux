// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const setConfig = vi.fn();
let settings: Record<string, unknown> = {};
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { SetConfigCommand: (_c: unknown, data: unknown) => setConfig(data) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/global", () => ({ settingsAtom: () => settings }));

import { UsePathToggle } from "./UsePathToggle";

const toggle = () => screen.getByRole("switch", { name: "Use the CLI on my PATH" });

describe("UsePathToggle", () => {
    beforeEach(() => {
        setConfig.mockReset();
        setConfig.mockResolvedValue(undefined);
        settings = {};
    });
    afterEach(cleanup);

    it("is off unless the provider is listed, and listing it adds only that provider", async () => {
        settings = { "cli:usepath": ["aider"] };
        const onChanged = vi.fn();
        render(() => <UsePathToggle providerId="goose" onChanged={onChanged} />);
        expect(toggle().getAttribute("aria-checked")).toBe("false");
        fireEvent.click(toggle());
        await waitFor(() => expect(onChanged).toHaveBeenCalledTimes(1));
        expect(setConfig).toHaveBeenCalledWith({ "cli:usepath": ["aider", "goose"] });
    });

    it("switching it off removes the provider, and clears the setting when none is left", async () => {
        settings = { "cli:usepath": ["goose"] };
        const onChanged = vi.fn();
        render(() => <UsePathToggle providerId="goose" onChanged={onChanged} />);
        expect(toggle().getAttribute("aria-checked")).toBe("true");
        fireEvent.click(toggle());
        await waitFor(() => expect(onChanged).toHaveBeenCalledTimes(1));
        expect(setConfig).toHaveBeenCalledWith({ "cli:usepath": null });
    });

    it("ignores a setting that is not a list of ids", async () => {
        settings = { "cli:usepath": "goose" };
        render(() => <UsePathToggle providerId="goose" onChanged={() => {}} />);
        expect(toggle().getAttribute("aria-checked")).toBe("false");
        fireEvent.click(toggle());
        await waitFor(() => expect(setConfig).toHaveBeenCalledWith({ "cli:usepath": ["goose"] }));
    });
});
