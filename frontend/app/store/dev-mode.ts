// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Dev mode: whether the developer-only parts of the UI are shown (DevTools,
 * Inspect Element, View Page Source, ...). docs/specs/SPEC_DEV_MODE_SETTING_2026_10_28.md.
 *
 * `app:devmode` decides when it is set. Unset, the build decides: on in a task
 * dev or debug build, off in a release build. Every gate asks `devModeOn()`;
 * nothing else reads the key. The host checks the same value itself before it
 * opens DevTools (crates/cef, `dev_mode`).
 */
import { isDev } from "@/app/store/misc-utils";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getSettingsKeyAtom } from "@/store/global";

export const DEV_MODE_KEY = "app:devmode";

/** The pure rule, for the tests. */
export function devModeValue(setting: boolean | undefined, devBuild: boolean): boolean {
    return typeof setting === "boolean" ? setting : devBuild;
}

/** Reactive: reads the settings atom, so a memo that calls it follows a toggle. */
export function devModeOn(): boolean {
    return devModeValue(getSettingsKeyAtom(DEV_MODE_KEY)(), isDev());
}

/** Writes the setting for every window. */
export function setDevMode(on: boolean): void {
    void RpcApi.SetConfigCommand(TabRpcClient, { [DEV_MODE_KEY]: on });
}

/** Palette command. */
export function toggleDevMode(): void {
    setDevMode(!devModeOn());
}
