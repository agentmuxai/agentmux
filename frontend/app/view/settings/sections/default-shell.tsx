// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Settings → Terminal → Default shell: the shells installed here, the one new
// terminals open with, and a way to pick another
// (docs/specs/SPEC_DEFAULT_SHELL_SETTING_2026_10_13.md).

import { Button, Select, TextInput } from "@/app/element/ui";
import { createEffect, createSignal, For, on, Show, type JSX } from "solid-js";

import { createBlock } from "@/app/store/block-layout-actions";
import { settingsAtom } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import type { ShellList } from "@/app/store/rpc-api/shells";
import { TabRpcClient } from "@/app/store/rpc-util";
import { set } from "../settings-controls";
import { automaticLabel, CUSTOM, describeEffective, selectedEntry, shellLabel } from "./default-shell-model";

export function DefaultShellSetting(): JSX.Element {
    const setting = () => (settingsAtom()?.["term:localshellpath"] as string | undefined) ?? "";
    const opts = () => ((settingsAtom()?.["term:localshellopts"] as string[] | undefined) ?? []).join(" ");
    const [list, setList] = createSignal<ShellList | null>(null);
    // Custom… chosen but no path typed yet.
    const [customOpen, setCustomOpen] = createSignal(false);

    // Ask on mount, and again when the setting changes: the line under the
    // dropdown follows it. srv reuses its listing for 30 s, so this is cheap.
    createEffect(
        on(setting, () => {
            RpcApi.ShellListCommand(TabRpcClient).then(setList, () => {});
        })
    );

    const entry = () => (customOpen() ? CUSTOM : selectedEntry(setting(), list()?.shells ?? []));

    const choose = (value: string) => {
        if (value === CUSTOM) {
            setCustomOpen(true);
            return;
        }
        setCustomOpen(false);
        // A listed shell (or Automatic) brings its own arguments.
        set("term:localshellpath", value || null);
        set("term:localshellopts", null);
    };

    return (
        <div class="default-shell">
            <Select class="setting-select" value={entry()} onChange={choose}>
                <option value="">{automaticLabel(list())}</option>
                <For each={list()?.shells ?? []}>
                    {(shell) => (
                        <option value={shell.path} title={shell.path}>
                            {shellLabel(shell)}
                        </option>
                    )}
                </For>
                <option value={CUSTOM}>Custom…</option>
            </Select>
            <Show when={entry() === CUSTOM}>
                <TextInput
                    class="setting-text"
                    type="text"
                    value={setting()}
                    placeholder="Full path, or a name on PATH (nu, pwsh)"
                    onBlur={(e) => {
                        const path = e.currentTarget.value.trim();
                        if (path) setCustomOpen(false);
                        set("term:localshellpath", path || null);
                    }}
                />
                <TextInput
                    class="setting-text"
                    type="text"
                    value={opts()}
                    placeholder="Arguments, separated by spaces (optional)"
                    onBlur={(e) => {
                        const args = e.currentTarget.value.trim().split(/\s+/).filter(Boolean);
                        set("term:localshellopts", args.length ? args : null);
                    }}
                />
            </Show>
            <Show when={list()}>
                {(l) => (
                    <div class="default-shell-status" classList={{ "default-shell-status--warn": describeEffective(l()).warn }}>
                        {describeEffective(l()).text}
                    </div>
                )}
            </Show>
            <Button onClick={() => void createBlock({ meta: { view: "term", controller: "shell" } })}>
                Open a terminal with it
            </Button>
        </div>
    );
}
