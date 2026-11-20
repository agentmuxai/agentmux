// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Settings → Advanced → Unrecognized settings: the keys srv says are not settings, each with its value, its line and a
 * suggestion for a typo, and Remove, which writes the key as null (the one way a key is ever deleted by the app).
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const setConfig = vi.hoisted(() => vi.fn(async (..._args: unknown[]) => undefined));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { SetConfigCommand: (...args: unknown[]) => setConfig(...args) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/global", async () => {
    const signals = await import("@/app/store/config-signals");
    return { settingsAtom: signals.settingsAtom, fullConfigAtom: signals.fullConfigAtom };
});

import { setFullConfigAtom } from "@/app/store/config-signals";
import { resetSaveResults } from "../settings-saved";
import { UnrecognizedGroup, unrecognizedDescription, valueShown } from "./unrecognized-group";

const withUnrecognized = (settings: Record<string, unknown>, unrecognized?: UnrecognizedSetting[]) =>
    setFullConfigAtom({ settings, configerrors: [], unrecognized } as unknown as FullConfigType);

beforeEach(() => {
    setConfig.mockClear();
    resetSaveResults();
});
afterEach(() => {
    cleanup();
    setFullConfigAtom({ settings: {}, configerrors: [] } as unknown as FullConfigType);
});

describe("Unrecognized settings", () => {
    it("shows nothing when srv lists none, or sends no list at all", () => {
        withUnrecognized({});
        const { container } = render(() => <UnrecognizedGroup />);
        expect(container.textContent).toBe("");
        withUnrecognized({}, []);
        expect(container.textContent).toBe("");
    });

    it("lists each key with what it holds, its line and the key it may have been meant to be", () => {
        withUnrecognized({ "term:fontsise": 18, "my:own:note": "hi" }, [
            { key: "term:fontsise", suggestion: "term:fontsize", line: 2 },
            { key: "my:own:note", line: 5 },
        ]);
        render(() => <UnrecognizedGroup />);
        expect(screen.getByText("Unrecognized settings")).toBeInTheDocument();
        const typo = screen.getByText("term:fontsise").closest(".setting-row") as HTMLElement;
        expect(typo.textContent).toContain("Holds 18");
        expect(typo.textContent).toContain("line 2");
        expect(typo.textContent).toContain("Did you mean term:fontsize?");
        const plain = screen.getByText("my:own:note").closest(".setting-row") as HTMLElement;
        expect(plain.textContent).toContain('Holds "hi"');
        expect(plain.textContent).not.toContain("Did you mean");
    });

    it("Remove writes the key as null and nothing else", () => {
        withUnrecognized({ "my:own:note": 1 }, [{ key: "my:own:note" }]);
        render(() => <UnrecognizedGroup />);
        fireEvent.click(screen.getByRole("button", { name: "Remove my:own:note from settings.json" }));
        expect(setConfig).toHaveBeenCalledTimes(1);
        expect(setConfig).toHaveBeenCalledWith({}, { "my:own:note": null });
    });

    it("a write that failed says so on its row, and the row stays", async () => {
        setConfig.mockRejectedValueOnce(new Error("read-only"));
        withUnrecognized({ "my:own:note": 1 }, [{ key: "my:own:note" }]);
        render(() => <UnrecognizedGroup />);
        fireEvent.click(screen.getByRole("button", { name: /Remove my:own:note/ }));
        await vi.waitFor(() => expect(screen.getByText(/Not saved/)).toBeInTheDocument());
        expect(screen.getByText("my:own:note")).toBeInTheDocument();
    });

    it("follows the list when srv sends a new one", () => {
        withUnrecognized({ a: 1 }, [{ key: "a" }]);
        render(() => <UnrecognizedGroup />);
        expect(screen.getByText("a")).toBeInTheDocument();
        withUnrecognized({}, []);
        expect(screen.queryByText("a")).toBeNull();
        expect(screen.queryByText("Unrecognized settings")).toBeNull();
    });
});

describe("what a row says", () => {
    it("cuts a long value and keeps a short one whole", () => {
        expect(valueShown(true)).toBe("true");
        expect(valueShown("x".repeat(100)).length).toBe(41 + 0);
        expect(valueShown({ a: [1, 2] })).toBe('{"a":[1,2]}');
        expect(valueShown(undefined)).toBe("undefined");
    });

    it("joins the sentences it has and leaves out the ones it lacks", () => {
        expect(unrecognizedDescription({ key: "k" }, 1)).toBe("Not a setting, so it does nothing · Holds 1");
        expect(unrecognizedDescription({ key: "k", line: 4, suggestion: "j" }, 1)).toBe("Not a setting, so it does nothing · Holds 1 · line 4 · Did you mean j?");
    });
});
