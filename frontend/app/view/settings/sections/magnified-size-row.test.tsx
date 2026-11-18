// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The magnified size field takes any number but writes it held to the layout area, says so, and puts the corrected
 * number back in the box. A value from an old config shows as the layout reads it.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";

const h = vi.hoisted(() => ({ setConfig: vi.fn() }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { SetConfigCommand: (...args: unknown[]) => h.setConfig(...args) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { MagnifiedSizeRow } from "./magnified-size-row";
import { resetSaveResults } from "../settings-saved";

const ROW = { id: "magnified-size", label: "Magnified size", description: "How much of the layout area a pane fills" };
const KEY = "window:magnifiedblocksize";

/** The stored size follows each write, as the settings do once a write has landed. */
function setup(initial?: number) {
    const [stored, setStored] = createSignal<number | undefined>(initial);
    h.setConfig.mockImplementation((_client: unknown, data: Record<string, number>) => {
        setStored(data[KEY]);
        return Promise.resolve();
    });
    const view = render(() => <MagnifiedSizeRow s={() => (stored() === undefined ? {} : { [KEY]: stored() })} row={ROW} />);
    return { ...view, input: view.getByRole("spinbutton") as HTMLInputElement };
}

/** Type `text` into the box and let the field's commit pause pass. */
async function type(input: HTMLInputElement, text: string) {
    fireEvent.input(input, { target: { value: text } });
    await vi.advanceTimersByTimeAsync(450);
}

beforeEach(() => {
    vi.useFakeTimers();
    h.setConfig.mockReset();
    h.setConfig.mockResolvedValue(undefined);
    resetSaveResults();
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

describe("the magnified size field", () => {
    it("holds 3 to the largest size, says so, and shows the number it used", async () => {
        const { input, getByTestId } = setup(0.9);
        await type(input, "3");
        expect(h.setConfig).toHaveBeenCalledWith({}, { [KEY]: 1 });
        expect(getByTestId("magnified-size-snap").textContent).toBe("3 is out of range: using 1. Use 0.5 to 1.");
        expect(input.value, "the corrected number is back in the box").toBe("1");
    });

    it("holds 0 to the smallest size", async () => {
        const { input, getByTestId } = setup(0.9);
        await type(input, "0");
        expect(h.setConfig).toHaveBeenCalledWith({}, { [KEY]: 0.5 });
        expect(getByTestId("magnified-size-snap").textContent).toContain("using 0.5");
    });

    it("writes a size in range as typed, with no message", async () => {
        const { input, queryByTestId } = setup(0.9);
        await type(input, "0.75");
        expect(h.setConfig).toHaveBeenCalledWith({}, { [KEY]: 0.75 });
        expect(queryByTestId("magnified-size-snap")).toBeNull();
    });

    it("clears the message once a size in range is typed", async () => {
        const { input, queryByTestId } = setup(0.9);
        await type(input, "3");
        expect(queryByTestId("magnified-size-snap")).not.toBeNull();
        await type(input, "0.8");
        expect(queryByTestId("magnified-size-snap")).toBeNull();
    });

    it("leaves the box alone while it is being typed in, and shows the used number when it is left", async () => {
        const { input, getByTestId } = setup(0.9);
        input.focus();
        await type(input, "0");
        expect(getByTestId("magnified-size-snap").textContent).toContain("using 0.5");
        expect(input.value, "a pause after 0 must not turn it into 0.5 under the caret").toBe("0");
        await type(input, "0.75");
        expect(input.value).toBe("0.75");
        expect(h.setConfig).toHaveBeenLastCalledWith({}, { [KEY]: 0.75 });
        await type(input, "3");
        expect(input.value).toBe("3");
        input.blur();
        await vi.advanceTimersByTimeAsync(0);
        expect(input.value, "left: the box shows what was used").toBe("1");
    });

    it("shows a stored size from an old config as the layout reads it, and an unset one as the default", () => {
        expect(setup(3).input.value).toBe("1");
        cleanup();
        expect(setup(1.5).input.value).toBe("1");
        cleanup();
        expect(setup().input.value).toBe("0.9");
    });
});
