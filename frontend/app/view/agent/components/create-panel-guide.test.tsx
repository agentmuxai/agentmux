// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The create panel's free-guide check must not suspend. Antigravity's sign-in check (`agy models`) takes a
 * second; read as a resource it hid the whole pane behind the loading spinner for that second, remounted
 * the panel when it came back, and so asked again without end: a black pane, and the CLI run once a second.
 */

// @vitest-environment jsdom

import { cleanup, render, waitFor } from "@solidjs/testing-library";
import { createSignal, Suspense } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { checkCliAuth, listAccounts } = vi.hoisted(() => ({ checkCliAuth: vi.fn(), listAccounts: vi.fn() }));

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { ListIdentityAccountsCommand: listAccounts } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("../flows/login-and-confirm", () => ({ checkCliAuth }));
vi.mock("../install-check-cache", () => ({
    checkProviderInstalled: vi.fn(async () => ({ installed: true, path: "agy.exe" })),
}));
vi.mock("../providers", () => ({
    getProvider: (id: string) =>
        id === "antigravity"
            ? { id, cliCommand: "agy", npmPackage: "", authType: "cli-managed", authCheckCommand: ["models"] }
            : undefined,
}));

import { useFreeGuide } from "./create-panel-guide";

let created = 0;
let guide: ReturnType<typeof useFreeGuide> | undefined;

function Panel(props: { providerId: () => string | undefined }) {
    created++;
    guide = useFreeGuide({ providerId: props.providerId, props: { template: { provider: "antigravity" } } } as never);
    // Read the way the real view reads it, in the markup: that is what suspends a resource.
    return <div data-testid="panel">{guide.showFreeGuide() ? "guide" : "no guide"}</div>;
}

beforeEach(() => {
    created = 0;
    guide = undefined;
    listAccounts.mockResolvedValue([]);
});

afterEach(() => {
    cleanup();
    vi.clearAllMocks();
});

describe("useFreeGuide", () => {
    it("leaves the pane showing while the sign-in check runs, and asks once", async () => {
        let answer: (v: { authenticated: boolean }) => void = () => {};
        checkCliAuth.mockReturnValue(new Promise((resolve) => (answer = resolve)));
        const { queryByText, getByTestId } = render(() => (
            <Suspense fallback={<div>spinner</div>}>
                <Panel providerId={() => "antigravity"} />
            </Suspense>
        ));
        await waitFor(() => expect(checkCliAuth).toHaveBeenCalledTimes(1));
        expect(getByTestId("panel")).toBeTruthy();
        expect(queryByText("spinner")).toBeNull();
        answer({ authenticated: false });
        await waitFor(() => expect(guide?.showFreeGuide()).toBe(true));
        expect(created).toBe(1);
        expect(checkCliAuth).toHaveBeenCalledTimes(1);
    });

    it("shows no guide to someone already signed in to the CLI", async () => {
        checkCliAuth.mockResolvedValue({ authenticated: true });
        render(() => <Panel providerId={() => "antigravity"} />);
        await waitFor(() => expect(checkCliAuth).toHaveBeenCalled());
        await new Promise((resolve) => setTimeout(resolve, 20));
        expect(guide?.showFreeGuide()).toBe(false);
    });

    it("asks again for another provider, and never shows an answer meant for the first", async () => {
        const [id, setId] = createSignal<string | undefined>("antigravity");
        checkCliAuth.mockResolvedValue({ authenticated: false });
        render(() => <Panel providerId={id} />);
        await waitFor(() => expect(guide?.showFreeGuide()).toBe(true));
        setId(undefined);
        await waitFor(() => expect(guide?.showFreeGuide()).toBe(false));
    });
});
