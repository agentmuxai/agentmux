// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Swarm row's registration badge and its "Stop receiving messages" entry
 * (swarm-row-registration.tsx): one batched read, nothing for a healthy row.
 */

import { cleanup, render, screen, waitFor } from "@solidjs/testing-library";
import { createSignal, Show } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
    readFlags: vi.fn(),
    unregister: vi.fn(async () => {}),
}));

const client = vi.hoisted(() => ({ id: "tab-client" }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: client }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { GetReactiveRegistrationFlagsCommand: mocks.readFlags, AgentPaneUnregisterCommand: mocks.unregister },
}));
// The real modal pulls in the whole dialog stack; its answer is all that matters here.
vi.mock("@/app/element/confirm-modal", () => ({
    ConfirmModal: (props: any) => (
        <Show when={props.open}>
            <div data-testid="confirm">
                <span>{props.title}</span>
                <button data-testid="confirm-yes" onClick={() => void props.onConfirm()}>
                    {props.confirmLabel}
                </button>
            </div>
        </Show>
    ),
}));

import {
    readRegistrationFlags,
    RegistrationBadge,
    RegistrationFlagsProvider,
    registrationDetail,
    registrationText,
    stopReceivingItems,
    useRegistrationFlag,
    type RegistrationFlag,
} from "./swarm-row-registration";

const claimed: RegistrationFlag = {
    agent_id: "camper",
    block_id: "block-claimed",
    remote: [{ channel: "dev", pid: 4242, updated_at: 1_000_000 }],
    recent_mismatch: null,
};
const refused: RegistrationFlag = {
    agent_id: "korp",
    block_id: "block-refused",
    remote: [],
    recent_mismatch: { timestamp: 1_000_000, block_id: "0123456789abcdef", error_message: "wrong identity" },
};

beforeEach(() => {
    mocks.readFlags.mockReset();
    mocks.unregister.mockClear();
});

afterEach(() => cleanup());

describe("the words", () => {
    it("says the claim first, and the refusal when there is none", () => {
        expect(registrationText(claimed)).toBe("Another instance claims this agent");
        expect(registrationText(refused)).toBe("A message to it was refused: identity mismatch");
        expect(registrationText({ ...claimed, recent_mismatch: refused.recent_mismatch })).toBe(
            "Another instance claims this agent"
        );
    });

    it("details the channel, pid, time, block and error", () => {
        const now = 1_000_000 + 5 * 60_000;
        expect(registrationDetail(claimed, now)).toContain("channel dev (pid 4242), updated 5 min ago");
        expect(registrationDetail(claimed, now)).toContain("whichever instance registered last");
        const detail = registrationDetail(refused, now);
        expect(detail).toContain("5 min ago, block 01234567: wrong identity");
        expect(detail).not.toContain("Also claimed");
    });
});

describe("readRegistrationFlags", () => {
    it("returns what srv flags", async () => {
        mocks.readFlags.mockResolvedValue({ flags: [claimed] });
        expect(await readRegistrationFlags()).toEqual([claimed]);
        expect(mocks.readFlags).toHaveBeenCalledWith(client);
    });

    it("is empty when srv does not know the command, or answers nothing", async () => {
        mocks.readFlags.mockRejectedValue(new Error("unknown command"));
        expect(await readRegistrationFlags()).toEqual([]);
        mocks.readFlags.mockResolvedValue(null);
        expect(await readRegistrationFlags()).toEqual([]);
    });
});

describe("the row", () => {
    function Row(props: { blockId: string }) {
        const flag = useRegistrationFlag(() => props.blockId);
        return (
            <div data-testid={`row-${props.blockId}`}>
                <RegistrationBadge flag={flag} />
                <button data-testid={`stop-${props.blockId}`} onClick={() => stopReceivingItems(flag())[0]?.click?.()}>
                    menu
                </button>
                <span data-testid={`items-${props.blockId}`}>{stopReceivingItems(flag()).length}</span>
            </div>
        );
    }

    function renderRows() {
        const [visible] = createSignal(true);
        return render(() => (
            <RegistrationFlagsProvider visible={visible}>
                <Row blockId="block-claimed" />
                <Row blockId="block-refused" />
                <Row blockId="block-healthy" />
            </RegistrationFlagsProvider>
        ));
    }

    it("badges the flagged rows only, by block, with the detail on hover", async () => {
        mocks.readFlags.mockResolvedValue({ flags: [claimed, refused] });
        renderRows();

        await waitFor(() => expect(screen.getAllByText(/instance claims|identity mismatch/)).toHaveLength(2));
        const claimedRow = screen.getByTestId("row-block-claimed");
        expect(claimedRow.textContent).toContain("Another instance claims this agent");
        expect(claimedRow.querySelector(".swarm-reg-badge")?.getAttribute("title")).toContain("pid 4242");
        expect(screen.getByTestId("row-block-refused").textContent).toContain("identity mismatch");
        expect(screen.getByTestId("row-block-healthy").querySelector(".swarm-reg-badge")).toBeNull();
    });

    it("offers Stop receiving messages on a badged row and not on a healthy one", async () => {
        mocks.readFlags.mockResolvedValue({ flags: [claimed] });
        renderRows();
        await waitFor(() => expect(screen.getByTestId("items-block-claimed").textContent).toBe("1"));
        expect(screen.getByTestId("items-block-healthy").textContent).toBe("0");
    });

    it("unregisters the row's agent and block after the person confirms", async () => {
        mocks.readFlags.mockResolvedValue({ flags: [claimed] });
        renderRows();
        await waitFor(() => expect(screen.getByTestId("items-block-claimed").textContent).toBe("1"));

        screen.getByTestId("stop-block-claimed").click();
        expect(await screen.findByTestId("confirm")).toBeTruthy();
        expect(mocks.unregister).not.toHaveBeenCalled();

        screen.getByTestId("confirm-yes").click();
        await waitFor(() => expect(mocks.unregister).toHaveBeenCalledTimes(1));
        expect(mocks.unregister).toHaveBeenCalledWith(client, { agent_id: "camper", block_id: "block-claimed" });
    });
});
