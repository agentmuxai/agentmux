// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Dev mode: the setting when it is set, the build otherwise.
 * docs/specs/SPEC_DEV_MODE_SETTING_2026_10_28.md §3.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

const state = vi.hoisted(() => ({ setting: undefined as unknown, devBuild: false, sent: [] as unknown[] }));
vi.mock("@/app/store/misc-utils", () => ({ isDev: () => state.devBuild }));
vi.mock("@/store/global", () => ({ getSettingsKeyAtom: () => () => state.setting }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { SetConfigCommand: vi.fn((_client: unknown, data: unknown) => state.sent.push(data)) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { DEV_MODE_KEY, devModeOn, devModeValue, setDevMode, toggleDevMode } from "./dev-mode";

describe("devModeValue", () => {
    it("is the setting when it is set, either way, in any build", () => {
        for (const devBuild of [true, false]) {
            expect(devModeValue(true, devBuild)).toBe(true);
            expect(devModeValue(false, devBuild)).toBe(false);
        }
    });

    it("is the build's default when the setting is absent: on in a dev build, off in a release build", () => {
        expect(devModeValue(undefined, true)).toBe(true);
        expect(devModeValue(undefined, false)).toBe(false);
    });
});

describe("devModeOn / setDevMode / toggleDevMode", () => {
    beforeEach(() => {
        state.setting = undefined;
        state.devBuild = false;
        state.sent = [];
    });

    it("reads the settings key and falls back to the build", () => {
        expect(devModeOn()).toBe(false);
        state.devBuild = true;
        expect(devModeOn()).toBe(true);
        state.setting = false;
        expect(devModeOn()).toBe(false);
        state.devBuild = false;
        state.setting = true;
        expect(devModeOn()).toBe(true);
    });

    it("writes app:devmode through the settings command", () => {
        setDevMode(true);
        setDevMode(false);
        expect(state.sent).toEqual([{ [DEV_MODE_KEY]: true }, { [DEV_MODE_KEY]: false }]);
        expect(DEV_MODE_KEY).toBe("app:devmode");
    });

    it("toggles from what is shown, so a dev build's first toggle turns it off", () => {
        state.devBuild = true;
        toggleDevMode();
        state.devBuild = false;
        toggleDevMode();
        expect(state.sent).toEqual([{ "app:devmode": false }, { "app:devmode": true }]);
    });
});
