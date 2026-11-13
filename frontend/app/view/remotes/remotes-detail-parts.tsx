// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The details opened below a remote's card, the views: its actions, the result of a connection test, its settings
// (nickname, colour, and for ssh the two session settings) and the agents always allowed on it. (`Detail` in
// remotes-view.tsx puts them together.)

import { For, Show, type JSX } from "solid-js";
import type { RemoteRecord } from "@/app/store/rpc-api/remotes";
import { Button, Select, TextInput } from "@/app/element/ui";
import { connectionAction, runConnectionAction } from "./remotes-connection";
import type { RemotesViewModel } from "./remotes-model";
import { isHidden, isPinned, REMOTE_COLORS, remoteColor } from "./remotes-sections";

/** What a detail view works on: the remote, the model, and the model's `run` with the action's name for the notice. */
export interface DetailProps {
    r: () => RemoteRecord;
    model: RemotesViewModel;
    run: (what: string, action: () => Promise<void>) => () => void;
}

/** Connect or Disconnect, Sessions, Pin, Hide, Forget, Remove helper, Test connection and Edit in ssh config, where they apply. */
export function DetailActions(props: DetailProps): JSX.Element {
    const { r, model, run } = props;
    const connection = () => connectionAction(r());
    return (
        <div class="remotes-detail-actions">
            <Show when={connection()}>
                {(action) => <Button onClick={run(action(), () => runConnectionAction(model, r(), action()))}>{action()}</Button>}
            </Show>
            <Show when={r().kind === "ssh"}>
                <Button onClick={run("Sessions", () => model.showSessions(r().name))}>Sessions…</Button>
            </Show>
            <Button onClick={run(isPinned(r()) ? "Unpin" : "Pin", () => model.setPinned(r().name, !isPinned(r())))}>
                {isPinned(r()) ? "Unpin" : "Pin"}
            </Button>
            <Button onClick={run(isHidden(r()) ? "Unhide" : "Hide", () => model.setHidden(r().name, !isHidden(r())))}>
                {isHidden(r()) ? "Unhide" : "Hide"}
            </Button>
            <Show when={r().sources.includes("recent")}>
                <Button onClick={run("Forget", () => model.forget(r().name))}>Forget</Button>
            </Show>
            <Show when={r().helper.installed}>
                <Button tone="danger" onClick={run("Remove helper", () => model.removeHelper(r().name))}>
                    Remove helper
                </Button>
            </Show>
            <Show when={r().kind === "ssh"}>
                <Button
                    disabled={model.testing() === r().name}
                    onClick={run("Test connection", () => model.testConnection(r().name))}
                >
                    {model.testing() === r().name ? "Testing…" : "Test connection"}
                </Button>
            </Show>
            <Show when={r().sources.includes("ssh_config")}>
                <Button onClick={run("Edit in ssh config", () => model.editInSshConfig(r().name))}>Edit in ssh config</Button>
            </Show>
        </div>
    );
}

/** The last connection test's result for this remote. */
export function TestResult(props: Pick<DetailProps, "r" | "model">): JSX.Element {
    const { r, model } = props;
    return (
        <Show when={model.testResults()[r().name]}>
            {(result) => (
                <div class={result() === true ? "remotes-test-ok" : "remotes-detail-error"} role="status">
                    {result() === true ? "Connected and ran a command." : `Test failed: ${result()}`}
                </div>
            )}
        </Show>
    );
}

/** Nickname, colour, and for ssh remotes keep-sessions-alive and the helper policy. */
export function DetailSettings(props: DetailProps): JSX.Element {
    const { r, model, run } = props;
    const durable = () => r().settings?.["term:durable"];

    const commitNickname = (value: string) => {
        const current = (r().settings?.["display:name"] as string | undefined) ?? "";
        if (value.trim() === current.trim()) return;
        void model.run("Rename", () => model.setNickname(r().name, value));
    };

    return (
        <div class="remotes-settings">
            <label class="remotes-setting">
                <span>Nickname</span>
                <TextInput
                    placeholder={r().name}
                    value={(r().settings?.["display:name"] as string | undefined) ?? ""}
                    onBlur={(e) => commitNickname(e.currentTarget.value)}
                    onKeyDown={(e) => {
                        // Blurring commits it, once.
                        if (e.key === "Enter") e.currentTarget.blur();
                    }}
                />
            </label>
            <div class="remotes-setting">
                <span>Colour</span>
                <div class="remotes-colors" role="radiogroup" aria-label="Colour">
                    <Button
                        class="remotes-color"
                        icon="ban"
                        role="radio"
                        aria-checked={!remoteColor(r())}
                        title="None"
                        aria-label="None"
                        onClick={run("Colour", () => model.setColor(r().name, null))}
                    />
                    <For each={REMOTE_COLORS}>
                        {(c) => (
                            <Button
                                class="remotes-color"
                                role="radio"
                                aria-checked={remoteColor(r())?.toLowerCase() === c.value}
                                title={c.name}
                                aria-label={c.name}
                                onClick={run("Colour", () => model.setColor(r().name, c.value))}
                            >
                                <span class="remotes-color-chip" style={{ background: c.value }} />
                            </Button>
                        )}
                    </For>
                </div>
            </div>
            <Show when={r().kind === "ssh"}>
                <label class="remotes-setting">
                    <span>Keep sessions alive</span>
                    <Select
                        value={durable() === true ? "on" : durable() === false ? "off" : "default"}
                        onChange={(v) =>
                            void model.run("Keep sessions alive", () =>
                                model.setDurable(r().name, v === "on" ? true : v === "off" ? false : null)
                            )
                        }
                        options={[
                            { value: "default", label: "As in Settings" },
                            { value: "on", label: "On" },
                            { value: "off", label: "Off" },
                        ]}
                    />
                </label>
                <label class="remotes-setting">
                    <span>Install the helper</span>
                    <Select
                        value={(r().settings?.["conn:helper"] as string | undefined) || "ask"}
                        onChange={(v) => void model.run("Install the helper", () => model.setHelperPolicy(r().name, v))}
                        options={[
                            { value: "ask", label: "As in Settings" },
                            { value: "always", label: "Always" },
                            { value: "never", label: "Never" },
                        ]}
                    />
                </label>
            </Show>
        </div>
    );
}

/** The agents always allowed on this remote, each with Revoke. */
export function DetailAgents(props: DetailProps): JSX.Element {
    const { r, model, run } = props;
    return (
        <Show when={r().agents.length > 0}>
            <div class="remotes-agents">
                <span>Agents always allowed here (as you, with your SSH keys):</span>
                <ul>
                    <For each={r().agents}>
                        {(agent) => (
                            <li>
                                <span class="remotes-agent-name">{agent}</span>
                                <Button
                                    tone="quiet"
                                    aria-label={`Revoke ${agent} on ${r().name}`}
                                    onClick={run("Revoke", () => model.revokeAgent(r().name, agent))}
                                >
                                    Revoke
                                </Button>
                            </li>
                        )}
                    </For>
                </ul>
            </div>
        </Show>
    );
}
