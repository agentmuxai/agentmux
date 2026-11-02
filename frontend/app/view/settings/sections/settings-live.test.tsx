// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Saving settings.json in an editor updates the Settings controls at once: srv re-reads the file and
 * pushes the full config, which lands in `fullConfigAtom`; every control follows `settingsAtom` and
 * shows the new value without reopening Settings, and a file with a mistake shows its error live.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { SetConfigCommand: vi.fn() } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/global", async () => {
    const signals = await import("@/app/store/config-signals");
    return {
        settingsAtom: signals.settingsAtom,
        fullConfigAtom: signals.fullConfigAtom,
        getSettingsKeyAtom: (key: string) => () => (signals.settingsAtom() as Record<string, unknown>)[key],
    };
});

import { makeTestHostApi } from "@/app/host/test-host";
import { fullConfigAtom, setFullConfigAtom } from "@/app/store/config-signals";
import { ADVANCED_SETTINGS, AdvancedSection } from "./advanced-section";
import { ConfigErrorsBanner } from "../settings-view";

const config = (settings: Record<string, unknown>, configerrors: Array<{ file: string; err: string }> = []) =>
    ({ settings, configerrors }) as unknown as FullConfigType;

const rowOf = (label: string): HTMLElement => {
    const row = screen.getByText(label).closest(".setting-row");
    expect(row).not.toBeNull();
    return row as HTMLElement;
};

afterEach(cleanup);
beforeEach(() => {
    window.api = makeTestHostApi();
    setFullConfigAtom(config({}));
});

describe("Settings controls follow a settings.json saved elsewhere", () => {
    it("a number field shows the new value, and its default when the key is removed", () => {
        setFullConfigAtom(config({ "agent:askquestiontimeoutms": 45000 }));
        render(() => <AdvancedSection />);
        const input = () => rowOf(ADVANCED_SETTINGS.autoAnswerTimeout.label).querySelector("input[type=number]") as HTMLInputElement;
        expect(input().value).toBe("45");
        // saved in an editor with another value
        setFullConfigAtom(config({ "agent:askquestiontimeoutms": 90000 }));
        expect(input().value).toBe("90");
        // the key deleted: back to the default (30 s)
        setFullConfigAtom(config({}));
        expect(input().value).toBe("30");
    });

    it("a switch flips", () => {
        setFullConfigAtom(config({ "term:disablewebgl": true }));
        render(() => <AdvancedSection />);
        const toggle = () => rowOf(ADVANCED_SETTINGS.disableWebgl.label).querySelector('[role="switch"]') as HTMLElement;
        expect(toggle()).toHaveAttribute("aria-checked", "true");
        setFullConfigAtom(config({ "term:disablewebgl": false }));
        expect(toggle()).toHaveAttribute("aria-checked", "false");
    });
});

describe("Settings shows a settings.json that does not parse", () => {
    it("lists the error when the config carries one, and drops it when the file is right again", () => {
        render(() => <ConfigErrorsBanner onOpen={() => {}} />);
        expect(screen.queryByText(/JSON parse error/)).toBeNull();
        setFullConfigAtom(config({}, [{ file: "settings.json", err: "JSON parse error: expected `,` at line 3" }]));
        expect(screen.getByText(/JSON parse error/)).toBeInTheDocument();
        setFullConfigAtom(config({}));
        expect(screen.queryByText(/JSON parse error/)).toBeNull();
        expect(fullConfigAtom()?.configerrors ?? []).toHaveLength(0);
    });
});
