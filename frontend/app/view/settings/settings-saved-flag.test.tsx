// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A row that names its setting shows "Saved" once the write has succeeded, then lets it fade; a failed write shows
 * that the setting was not saved, until the next write. A row that names no setting shows neither.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ setConfig: vi.fn() }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { SetConfigCommand: (...args: unknown[]) => h.setConfig(...args) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { DURATION_LINGER_MS } from "@/app/motion/tokens";
import { set, SettingRow } from "./settings-controls";
import { resetSaveResults, trackSave } from "./settings-saved";

const KEY = "model:spend_cap_usd:anthropic";

function Row(p: { settingKey?: string }) {
    return <SettingRow id="x" settingKey={p.settingKey} label="Daily cap" control={<button onClick={() => set(KEY, 500)}>write</button>} />;
}

const flush = () => vi.advanceTimersByTimeAsync(0);

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

describe("the Saved flag", () => {
    it("shows nothing at rest, and the write goes to the setting named", async () => {
        const { getByRole, queryByText } = render(() => <Row settingKey={KEY} />);
        expect(queryByText("Saved")).toBeNull();
        expect(getByRole("status").textContent, "the live region is there, empty").toBe("");
        fireEvent.click(getByRole("button"));
        await flush();
        expect(h.setConfig).toHaveBeenCalledWith({}, { [KEY]: 500 });
    });

    it("shows once the write has succeeded, not before", async () => {
        let done: () => void = () => {};
        h.setConfig.mockReturnValue(new Promise<void>((resolve) => (done = resolve)));
        const { getByRole, queryByText } = render(() => <Row settingKey={KEY} />);
        fireEvent.click(getByRole("button"));
        await flush();
        expect(queryByText("Saved"), "still writing").toBeNull();
        done();
        await flush();
        expect(queryByText("Saved")).not.toBeNull();
        expect(getByRole("status").textContent).toBe("Saved");
    });

    it("goes after the linger the app gives a confirmation", async () => {
        const { getByRole, queryByText } = render(() => <Row settingKey={KEY} />);
        fireEvent.click(getByRole("button"));
        await flush();
        await vi.advanceTimersByTimeAsync(DURATION_LINGER_MS - 10);
        expect(queryByText("Saved")).not.toBeNull();
        await vi.advanceTimersByTimeAsync(20);
        expect(queryByText("Saved")).toBeNull();
    });

    it("stays up through a burst of writes and goes once, after the last", async () => {
        const { getByRole, queryByText } = render(() => <Row settingKey={KEY} />);
        fireEvent.click(getByRole("button"));
        await flush();
        await vi.advanceTimersByTimeAsync(1000);
        fireEvent.click(getByRole("button"));
        await flush();
        await vi.advanceTimersByTimeAsync(DURATION_LINGER_MS - 100); // 2.4 s after the first, 1.4 s after the second
        expect(queryByText("Saved")).not.toBeNull();
        await vi.advanceTimersByTimeAsync(200);
        expect(queryByText("Saved")).toBeNull();
    });

    it("shows that a write failed, and keeps showing it until the next write", async () => {
        h.setConfig.mockRejectedValue(new Error("no"));
        const { getByRole, queryByText } = render(() => <Row settingKey={KEY} />);
        fireEvent.click(getByRole("button"));
        await flush();
        expect(queryByText("Not saved: the setting could not be written")).not.toBeNull();
        expect(queryByText("Saved")).toBeNull();
        await vi.advanceTimersByTimeAsync(DURATION_LINGER_MS * 4);
        expect(queryByText("Not saved: the setting could not be written"), "it does not fade").not.toBeNull();

        h.setConfig.mockResolvedValue(undefined);
        fireEvent.click(getByRole("button"));
        await flush();
        expect(queryByText("Not saved: the setting could not be written")).toBeNull();
        expect(queryByText("Saved")).not.toBeNull();
    });

    it("shows a failure that throws before a request is made the same way", async () => {
        h.setConfig.mockImplementation(() => {
            throw new Error("no client");
        });
        const { getByRole, queryByText } = render(() => <Row settingKey={KEY} />);
        fireEvent.click(getByRole("button"));
        await flush();
        expect(queryByText("Not saved: the setting could not be written")).not.toBeNull();
    });

    it("is not on a row that names no setting", async () => {
        const { getByRole, queryByRole, queryByText } = render(() => <Row />);
        fireEvent.click(getByRole("button"));
        await flush();
        expect(queryByRole("status")).toBeNull();
        expect(queryByText("Saved")).toBeNull();
    });

    it("keeps the control's name the label, without the flag in it", async () => {
        const { getByRole, getByText } = render(() => <Row settingKey={KEY} />);
        fireEvent.click(getByRole("button"));
        await flush();
        expect(getByText("Daily cap").tagName).toBe("LABEL");
        expect(getByText("Daily cap").textContent).toBe("Daily cap");
    });
});


describe("trackSave, for a setting a store function writes", () => {
    /** A row named for its own flag, with a button that writes through a function of the store's, not through `set`. */
    function SharedRow(p: { flag: string; write: () => Promise<unknown> }) {
        return <SettingRow id="y" settingKey={p.flag} label="Colour" control={<button onClick={() => trackSave([p.flag], p.write())}>write</button>} />;
    }

    it("shows Saved on the row that changed, once its write has succeeded, and on no other row", async () => {
        let done: () => void = () => {};
        const write = () => new Promise<void>((resolve) => (done = resolve));
        const { getAllByRole, getAllByText, queryAllByText } = render(() => (
            <>
                <SharedRow flag="pane:colors:agent" write={write} />
                <SharedRow flag="pane:colors:terminal" write={write} />
            </>
        ));
        fireEvent.click(getAllByRole("button")[0]);
        await flush();
        expect(queryAllByText("Saved"), "still writing").toHaveLength(0);
        done();
        await flush();
        expect(getAllByText("Saved")).toHaveLength(1);
        expect(getAllByRole("status")[0].textContent).toBe("Saved");
        expect(getAllByRole("status")[1].textContent).toBe("");
    });

    it("shows that the setting was not saved when the write fails", async () => {
        const { getByRole, queryByText } = render(() => <SharedRow flag="app:devmode" write={() => Promise.reject(new Error("no"))} />);
        fireEvent.click(getByRole("button"));
        await flush();
        expect(queryByText("Saved")).toBeNull();
        expect(queryByText("Not saved: the setting could not be written")).not.toBeNull();
    });
});
